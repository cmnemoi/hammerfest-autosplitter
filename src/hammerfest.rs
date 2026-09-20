//! Resolution de l'etat Hammerfest dans le process du plugin Flash.
//!
//! Il n'existe aucun chemin de pointeurs statique vers ces valeurs : ce ne sont
//! pas des variables C mais des proprietes d'objets ActionScript 2 crees a
//! l'execution par un SWF telecharge. L'ancre est donc une chaine internee du
//! SWF, et tout le reste se deduit d'elle.
//!
//! ```text
//! scan "]=[]8" dans le tas    `world`, nom obfusque connu par hf.map.json
//!   -> objet String           le qword module-pointant devant = la vtable
//!   -> slots citant la chaine scan des 8 encodages d'atome
//!   -> table de GameMode      celle qui possede cette clef
//! GameMode.world              -> GameMechanics
//!   .setName                  -> un monde Hammerfest connu     verification
//!   .currentId                -> le niveau
//! GameMode.gameChrono         -> fl_stop ? haltedTimer : frameTimer-gameTimer
//! ```

use alloc::{vec, vec::Vec};
use asr::{future::next_tick, Address, Process, ProcessId};

use crate::avm1::{self, read_u64, Layout, PROFILES, STR_BUF_CANDIDATES};
use crate::keys;

/// Le plugin, selon la plateforme.
pub const PLUGINS: &[&str] = &[
    "pepflashplayer.dll",
    "libpepflashplayer.so",
    "PepperFlashPlayer",
];

/// Taille des blocs de lecture. Le tas fait une centaine de Mio : a 64 Kio
/// c'etait un bon millier d'appels au runtime par passe, et chaque appel coute
/// bien plus cher que les octets qu'il rapporte.
const CHUNK: usize = 1024 * 1024;
/// Recouvrement entre deux morceaux, pour ne pas manquer un motif a cheval.
const OVERLAP: usize = 32;
/// Nombre de blocs lus avant de rendre la main au runtime.
const CHUNKS_PER_TICK: usize = 8;

const MAX_LEVEL: i64 = 256;

/// Indices d'entrees retenus d'une lecture a l'autre, toujours re-verifies.
#[derive(Default)]
struct Hints {
    world: u64,
    chrono: u64,
    current_id: u64,
    previous_id: u64,
    set_name: u64,
    dim: u64,
    game_over: u64,
    frame: u64,
    game: u64,
    halted: u64,
    stop: u64,
}

pub struct Game {
    pub layout: Layout,
    pub game_mode: u64,
    pub set: &'static str,
    hints: Hints,
}

pub use hammerfest_core::State;

// -- plages memoire ---------------------------------------------------------

/// Les plages ou vit le tas AVM1 : lisibles, ecrivables, sans fichier derriere.
pub fn heap_ranges(process: &Process) -> Vec<(u64, u64)> {
    use asr::MemoryRangeFlags as F;
    process
        .memory_ranges()
        .filter_map(|r| {
            let flags = r.flags().ok()?;
            if !flags.contains(F::READ | F::WRITE) || flags.contains(F::PATH) {
                return None;
            }
            let (addr, size) = r.range().ok()?;
            let a = addr.value();
            (size > 0).then(|| (a, a + size))
        })
        .collect()
}

// -- scans ------------------------------------------------------------------

/// Toutes les adresses alignees ou `pat` apparait.
async fn scan_bytes(
    process: &Process,
    ranges: &[(u64, u64)],
    pat: &[u8],
    align: usize,
    limit: usize,
) -> Vec<u64> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if process
                .read_into_slice(Address::new(base), &mut buf[..n])
                .is_ok()
            {
                let mut i = 0;
                while i + pat.len() <= n {
                    if &buf[i..i + pat.len()] == pat {
                        out.push(base + i as u64);
                        if out.len() >= limit {
                            return out;
                        }
                    }
                    i += align;
                }
            }
            if n <= OVERLAP {
                break;
            }
            base += (n - OVERLAP) as u64;
            chunks += 1;
            if chunks % CHUNKS_PER_TICK == 0 {
                next_tick().await;
            }
        }
    }
    out
}

/// Balaye les positions alignees ou `pat` apparait et appelle `on_hit` sur
/// chacune. Renvoyer `true` arrete le balayage.
async fn scan_bytes_until(
    process: &Process,
    ranges: &[(u64, u64)],
    pat: &[u8],
    align: usize,
    mut on_hit: impl FnMut(u64) -> bool,
) {
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if process
                .read_into_slice(Address::new(base), &mut buf[..n])
                .is_ok()
            {
                let mut i = 0;
                while i + pat.len() <= n {
                    if &buf[i..i + pat.len()] == pat && on_hit(base + i as u64) {
                        return;
                    }
                    i += align;
                }
            }
            if n <= OVERLAP {
                break;
            }
            base += (n - OVERLAP) as u64;
            chunks += 1;
            if chunks % CHUNKS_PER_TICK == 0 {
                next_tick().await;
            }
        }
    }
}

/// Balaye les slots contenant un atome pointant vers `ptr`, quel que soit son
/// tag, et appelle `on_hit` sur chacun. Renvoyer `true` arrete le balayage.
///
/// Un atome vaut `(valeur << 3) | tag` : les 8 variantes ne different que par
/// les 3 bits bas du premier octet. On parcourt donc les positions alignees en
/// comparant les 7 octets de poids fort, puis le premier octet masque.
///
/// Les hits sont livres au fil de l'eau plutot que collectes : il n'y a qu'une
/// petite dizaine de citations dans tout le tas, donc attendre la fin du
/// balayage pour les examiner reviendrait a toujours lire les cent Mio, meme
/// quand la bonne table est la premiere rencontree.
async fn scan_atoms(
    process: &Process,
    ranges: &[(u64, u64)],
    ptr: u64,
    mut on_hit: impl FnMut(u64) -> bool,
) {
    let bytes = ptr.to_le_bytes();
    let (lo, tail) = (bytes[0], &bytes[1..]);
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if process
                .read_into_slice(Address::new(base), &mut buf[..n])
                .is_ok()
            {
                let mut i = 0;
                while i + 8 <= n {
                    if buf[i] & !7 == lo && &buf[i + 1..i + 8] == tail && on_hit(base + i as u64) {
                        return;
                    }
                    i += 8;
                }
            }
            if n <= OVERLAP {
                break;
            }
            base += (n - OVERLAP) as u64;
            chunks += 1;
            if chunks % CHUNKS_PER_TICK == 0 {
                next_tick().await;
            }
        }
    }
}

// -- resolution -------------------------------------------------------------

/// Ce qu'une resolution apprend et que les suivantes reutilisent.
///
/// Rien ici n'est l'adresse d'un objet de partie -- ce serait le piege. Les
/// deux adresses retenues appartiennent a des objets qui vivent aussi longtemps
/// que le SWF : la chaine internee d'une clef, et la table du `GameManager`.
/// Elles sont revalidees avant chaque usage, et l'`Anchor` est remis a zero a
/// chaque nouveau process plugin -- il en nait un par partie.
#[derive(Default)]
pub struct Anchor {
    /// Objets String des clefs d'ancrage, internes par le SWF.
    string_world: Option<u64>,
    string_version: Option<u64>,
    /// Table de proprietes du `GameManager`, et le layout qui va avec.
    ///
    /// C'est l'ancre que le jeu offre lui-meme : `GameManager` nait avec le SWF
    /// et designe le mode qui tourne (`transition()` y ecrit chaque nouveau
    /// mode). Le trouver **pendant les menus** permet de ne plus jamais
    /// balayer ensuite : la partie, quand elle demarre, est au bout d'un
    /// pointeur.
    manager: Option<u64>,
    layout: Option<Layout>,
    current_hint: u64,
    /// L'ancre a-t-elle deja livre une partie ?
    ///
    /// Tant qu'elle ne l'a pas fait, le balayage de repli reste autorise. Une
    /// ancre fausse -- ou juste inexploitable -- ne doit pas pouvoir empecher
    /// a elle seule toute detection : c'est ce qui s'est produit quand
    /// l'offset `ScriptObject -> table` n'etait pas derive sur ce chemin-la.
    ///
    /// Mais l'autorisation ne peut pas durer : le balayage bloque la boucle une
    /// a deux secondes, et tant qu'aucune partie n'a ete vue -- donc pile au
    /// moment ou l'on attend le lancement -- il masquerait le clic. On fait
    /// donc confiance a l'ancre tout de suite, et on ne la remet en cause
    /// qu'apres un long silence.
    manager_proven: bool,
    /// Tentatives infructueuses depuis que l'ancre est posee.
    manager_idle: u32,
    /// Region ou le dernier GameMode a ete trouve : balayee en premier.
    last_game_mode: Option<u64>,
}

/// Ce qu'on retient du binaire lui-meme, d'un process plugin a l'autre.
///
/// Le plugin meurt avec la partie, mais c'est toujours la meme DLL : ses
/// vtables sont au meme offset dans le module, seule la base change avec
/// l'ASLR. Les retenir permet de trouver une chaine internee en **une** passe
/// -- balayer les en-tetes d'objets String -- au lieu de deux : chercher le
/// buffer, puis qui pointe dessus.
///
/// C'est du binaire, pas de la partie : rien ici ne peut devenir obsolete
/// entre deux parties, et tout est revalide a l'usage.
#[derive(Copy, Clone, Default)]
pub struct Binary {
    layout: Option<Layout>,
}

impl Binary {
    /// Le layout, rebase sur le module de ce process-ci.
    fn layout(&self, module: (u64, u64)) -> Option<Layout> {
        let mut l = self.layout?;
        l.str_vt = module.0 + l.str_vt;
        l.tbl_vt = module.0 + l.tbl_vt;
        l.module = module;
        Some(l)
    }

    fn learn(&mut self, layout: &Layout) {
        let mut l = *layout;
        l.str_vt -= layout.module.0;
        l.tbl_vt -= layout.module.0;
        self.layout = Some(l);
    }
}

impl Anchor {
    /// Le plugin Flash meurt avec la partie : les adresses apprises dans le
    /// process precedent n'ont aucun sens dans le suivant.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Tentatives infructueuses tolerees avant de remettre en cause une ancre qui
/// n'a jamais rien donne. A raison d'une par seconde environ, cela laisse une
/// bonne demi-minute avant de reprendre le balayage.
const MANAGER_IDLE_LIMIT: u32 = 30;

/// Voie rapide : suit `GameManager.current`, sans rien balayer.
///
/// Quelques lectures, contre une centaine de Mio. C'est ce qui permet de
/// chercher la partie a chaque tick. Renvoie None si l'ancre n'est pas encore
/// apprise, si la table du GameManager a bouge, ou si le mode courant n'est pas
/// une partie jouable -- un menu, par exemple.
pub fn resolve_via_manager(process: &Process, anchor: &mut Anchor) -> Option<Game> {
    let layout = anchor.layout?;
    let manager = anchor.manager?;
    if read_u64(process, manager)? != layout.tbl_vt {
        anchor.manager = None;
        return None;
    }
    // Plus de `current` : ce n'est plus le GameManager, la table a du bouger.
    // Lacher l'ancre relance un balayage plutot que de rester aveugle.
    let Some(current) =
        layout.get_cached(process, manager, keys::CURRENT, &mut anchor.current_hint)
    else {
        anchor.manager = None;
        return None;
    };
    let tbl = layout.table_of(process, current)?;
    let game = validate(process, layout, tbl)?;
    anchor.last_game_mode = Some(tbl);
    anchor.manager_proven = true;
    anchor.manager_idle = 0;
    Some(game)
}

/// Cherche la partie en cours.
///
/// Trois etages, du moins cher au plus cher :
///
/// 1. suivre `GameManager.current`, si l'ancre est connue ;
/// 2. sinon, localiser le `GameManager` -- il existe des le chargement du SWF,
///    donc cette recherche aboutit deja dans les menus, avant toute partie ;
/// 3. en dernier recours, chercher directement un `GameMode` par sa clef
///    `world`. Ne sert que si l'etage 2 echoue.
pub async fn resolve(
    process: &Process,
    module: (u64, u64),
    anchor: &mut Anchor,
    binary: &mut Binary,
) -> Option<Game> {
    if let Some(game) = resolve_via_manager(process, anchor) {
        return Some(game);
    }

    let mut ranges = heap_ranges(process);
    if ranges.is_empty() {
        return None;
    }
    if let Some(addr) = anchor.last_game_mode {
        move_region_first(&mut ranges, addr);
    }

    // Poser l'ancre d'abord : une fois le GameManager connu, plus aucun
    // balayage n'est necessaire, et le lancement d'une partie se voit en
    // quelques lectures au lieu d'une demi-seconde a plusieurs secondes.
    if anchor.manager.is_none() {
        if let Some((layout, tbl)) = scan_for_manager(process, module, &ranges, anchor, binary).await
        {
            asr::print_message(&alloc::format!(
                "Hammerfest: GameManager 0x{tbl:x}, layout {}",
                layout.profile.name,
            ));
            binary.learn(&layout);
            anchor.layout = Some(layout);
            anchor.manager = Some(tbl);
            anchor.current_hint = 0;
            return resolve_via_manager(process, anchor);
        }
    }

    // Ancre posee mais `current` ne designe pas de partie : il n'y en a
    // simplement pas. Rien a balayer, c'est deja la reponse -- et surtout, ne
    // pas balayer laisse la boucle libre de voir la partie demarrer des le
    // tick suivant.
    if anchor.manager.is_some() {
        anchor.manager_idle += 1;
        if anchor.manager_proven || anchor.manager_idle < MANAGER_IDLE_LIMIT {
            return None;
        }
        // Longtemps muette et jamais eprouvee : c'est peut-etre elle le
        // probleme. On la lache et on recherche.
        asr::print_message("Hammerfest: ancre muette, nouvelle recherche");
        anchor.manager = None;
        anchor.manager_idle = 0;
        return None;
    }

    // Repli : chercher le GameMode lui-meme, ancre sur `world` -- porte par une
    // poignee d'objets seulement.
    let (layout, strobj) =
        find_string(process, module, &ranges, keys::WORLD, &mut anchor.string_world, binary).await?;

    // La chaine internee et les tables qui la citent vivent dans le meme tas
    // AVM1 : commencer par sa region evite le plus souvent d'avoir a lire le
    // reste, et c'est ce qui rend la duree stable d'une fois sur l'autre.
    move_region_first(&mut ranges, strobj);

    let game = scan_tables(process, &ranges, layout, strobj, keys::WORLD, |l, t| {
        validate(process, l, t)
    })
    .await?;

    asr::print_message(&alloc::format!(
        "Hammerfest: GameMode 0x{:x}, monde {}, layout {}",
        game.game_mode,
        game.set,
        game.layout.profile.name,
    ));
    binary.learn(&game.layout);
    anchor.last_game_mode = Some(game.game_mode);
    anchor.layout = Some(game.layout);
    // `Mode.manager` mene au GameManager. Le retenir ne raccourcit pas le
    // demarrage d'une partie -- le plugin meurt avec elle, donc l'ancre ne
    // survit pas jusqu'a la suivante -- mais rend gratuite toute
    // re-resolution au sein d'une meme partie.
    anchor.manager = game.layout.child(process, game.game_mode, keys::MANAGER);
    anchor.current_hint = 0;
    Some(game)
}

/// Localise le `GameManager`, l'ancre qui rend toute detection ulterieure
/// immediate.
///
/// Ancree sur `fVersion`, pose dans son constructeur et qu'aucune autre classe
/// ne porte. Une clef banale comme `current` -- que chaque SetManager possede
/// -- serait citee des dizaines de fois, et chaque candidat coute la
/// reconstruction d'une table.
///
/// L'interet est le moment : le plugin Flash existe des l'ouverture de
/// l'application, donc bien avant qu'une partie soit lancee. Ce balayage-la a
/// tout le temps d'aboutir pendant que le joueur est encore dans les ecrans de
/// chargement, et la partie qui demarre ensuite se voit en quelques lectures.
async fn scan_for_manager(
    process: &Process,
    module: (u64, u64),
    ranges: &[(u64, u64)],
    anchor: &mut Anchor,
    binary: &mut Binary,
) -> Option<(Layout, u64)> {
    let (layout, strobj) = find_string(
        process,
        module,
        ranges,
        keys::F_VERSION,
        &mut anchor.string_version,
        binary,
    )
    .await?;

    scan_tables(process, ranges, layout, strobj, keys::F_VERSION, |mut l, t| {
        // La reference croisee valide le candidat *et* derive au passage
        // l'offset `ScriptObject -> table`, sans lequel rien de ce qui suit ne
        // peut etre lu : `GameManager.current` designe un mode dont le champ
        // `manager` redesigne ce meme GameManager.
        let current = l.get(process, t, keys::CURRENT)?;
        let mode = l.derive_so_tbl(process, current, keys::MANAGER)?;
        let back = l.child(process, mode, keys::MANAGER)?;
        (back == t).then_some((l, t))
    })
    .await
}

/// Met en tete la region contenant `addr`, si elle y est.
fn move_region_first(ranges: &mut [(u64, u64)], addr: u64) {
    if let Some(i) = ranges.iter().position(|&(a, b)| a <= addr && addr < b) {
        ranges.swap(0, i);
    }
}

/// Trouve l'objet String interne d'une clef, et le layout des String avec lui.
///
/// Le cache evite deux balayages complets par tentative : ces objets viennent
/// du pool de constantes du SWF, donc ils vivent aussi longtemps que le plugin.
/// Il est revalide en redecodant la chaine, jamais suppose valide.
async fn find_string(
    process: &Process,
    module: (u64, u64),
    ranges: &[(u64, u64)],
    key: &str,
    cache: &mut Option<u64>,
    binary: &Binary,
) -> Option<(Layout, u64)> {
    let units = key.encode_utf16().count() as u64;
    if let Some(so) = *cache {
        if let Some(layout) = string_layout_at(process, module, so, key, units) {
            return Some((layout, so));
        }
        *cache = None;
    }

    // Vtable connue : une seule passe suffit, sur les en-tetes d'objets.
    if let Some(seed) = binary.layout(module) {
        let mut found = None;
        scan_bytes_until(process, ranges, &seed.str_vt.to_le_bytes(), 8, |so| {
            if read_u64(process, so + seed.str_len) == Some(units) && seed.string_eq(process, so, key)
            {
                found = Some(so);
                return true;
            }
            false
        })
        .await;
        if let Some(so) = found {
            *cache = Some(so);
            return Some((seed, so));
        }
    }

    let needle: Vec<u8> = key.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    for buffer in scan_bytes(process, ranges, &needle, 2, 8).await {
        for r in scan_bytes(process, ranges, &buffer.to_le_bytes(), 8, 16).await {
            for buf_off in STR_BUF_CANDIDATES {
                let Some(so) = r.checked_sub(buf_off) else {
                    continue;
                };
                if let Some(layout) = string_layout_at(process, module, so, key, units) {
                    *cache = Some(so);
                    return Some((layout, so));
                }
            }
        }
    }
    None
}

/// Balaye les tables possedant `key` et rend la premiere qu'`accept` retient.
///
/// La validation se fait au fil du balayage : il n'y a qu'une petite dizaine de
/// citations dans tout le tas, donc les collecter avant de les examiner
/// obligerait a toujours lire les cent Mio, meme quand la bonne table est la
/// premiere rencontree.
async fn scan_tables<T>(
    process: &Process,
    ranges: &[(u64, u64)],
    layout: Layout,
    strobj: u64,
    key: &str,
    mut accept: impl FnMut(Layout, u64) -> Option<T>,
) -> Option<T> {
    let mut layout = layout;
    let mut result = None;
    scan_atoms(process, ranges, strobj, |slot| {
        for &profile in PROFILES {
            layout.profile = profile;
            layout.tbl_vt = 0;
            layout.so_tbl = 0;
            if !layout.key_is(process, slot, key) {
                continue;
            }
            let Some(tbl) = layout.table_base(process, slot) else {
                continue;
            };
            if let Some(found) = accept(layout, tbl) {
                result = Some(found);
                return true;
            }
        }
        false
    })
    .await;
    result
}

/// Deduit le layout des objets String a partir de l'adresse d'un objet String.
///
/// Trois contraintes independantes : le qword de tete pointe dans le module
/// (c'est la vtable), un qword vaut la longueur attendue, et la chaine ainsi
/// decodee est bien celle qu'on cherche.
fn string_layout_at(
    process: &Process,
    module: (u64, u64),
    so: u64,
    key: &str,
    units: u64,
) -> Option<Layout> {
    let vt = read_u64(process, so)?;
    if vt < module.0 || vt >= module.1 {
        return None;
    }
    for buf_off in STR_BUF_CANDIDATES {
        let mut len_off = 0x08;
        while len_off < 0x80 {
            if read_u64(process, so + len_off) == Some(units) {
                let layout = Layout {
                    module,
                    str_vt: vt,
                    str_buf: buf_off,
                    str_len: len_off,
                    tbl_vt: 0,
                    profile: PROFILES[0],
                    so_tbl: 0,
                };
                if layout.string_eq(process, so, key) {
                    return Some(layout);
                }
            }
            len_off += 8;
        }
    }
    None
}

/// Une table possedant `world` est-elle vraiment le GameMode ?
///
/// `world` seul ne suffit pas : les objets `View` en portent un aussi, et
/// pointent vers le meme `GameMechanics`. Seul le GameMode possede en plus un
/// `gameChrono`.
fn validate(process: &Process, mut layout: Layout, tbl: u64) -> Option<Game> {
    let world_atom = layout.get(process, tbl, keys::WORLD)?;
    let wtbl = layout.derive_so_tbl(process, world_atom, keys::SET_NAME)?;

    let set_atom = layout.get(process, wtbl, keys::SET_NAME)?;
    let set = keys::WORLDS
        .iter()
        .find(|(obf, _)| layout.string_eq(process, set_atom & !7, obf))
        .map(|&(_, clear)| clear)?;

    let level = layout.get_int(process, wtbl, keys::CURRENT_ID)?;
    if !(0..MAX_LEVEL).contains(&level) {
        return None;
    }

    let chrono = layout.child(process, tbl, keys::GAME_CHRONO)?;
    layout.get_int(process, chrono, keys::FRAME_TIMER)?;

    // Un GameMode deja en game over est un GameMode fini : s'y raccrocher
    // ferait lire une partie terminee au lieu d'attendre la suivante.
    if layout
        .get(process, tbl, keys::FL_GAME_OVER)
        .and_then(avm1::as_bool)
        == Some(true)
    {
        return None;
    }

    Some(Game {
        layout,
        game_mode: tbl,
        set,
        hints: Hints::default(),
    })
}

// -- lecture ----------------------------------------------------------------

impl Game {
    /// Etat courant, ou None si la resolution n'est plus valable.
    ///
    /// Tout est relu depuis GameMode a chaque appel. Garder l'adresse finale
    /// serait dangereux : le jeu reconstruit ses objets entre deux parties et
    /// le slot abandonne reste lisible, contenant une valeur plausible.
    pub fn read(&mut self, process: &Process) -> Option<State> {
        // Lu avant d'emprunter `self.layout` : `chrono_ms` a besoin de `self`
        // en entier.
        let (chrono_ms, frame_timer) = self.chrono(process)?;
        let l = &self.layout;
        let world_atom = l.get_cached(process, self.game_mode, keys::WORLD, &mut self.hints.world)?;
        let world = l.table_of(process, world_atom)?;

        // Le monde doit toujours etre un monde connu : c'est ce qui detecte
        // qu'on lit desormais de la memoire recyclee.
        let set_atom = l.get_cached(process, world, keys::SET_NAME, &mut self.hints.set_name)?;
        if !keys::WORLDS
            .iter()
            .any(|(obf, _)| l.string_eq(process, set_atom & !7, obf))
        {
            return None;
        }

        let level = avm1::as_int(l.get_cached(
            process,
            world,
            keys::CURRENT_ID,
            &mut self.hints.current_id,
        )?)?;
        if !(0..MAX_LEVEL).contains(&level) {
            return None;
        }
        let previous = l
            .get_cached(process, world, keys::PREVIOUS_ID, &mut self.hints.previous_id)
            .and_then(avm1::as_int)
            .unwrap_or(-1);

        Some(State {
            level,
            previous,
            chrono_ms,
            frame_timer,
            dim: l
                .get_cached(process, self.game_mode, keys::CURRENT_DIM, &mut self.hints.dim)
                .and_then(avm1::as_int)
                .unwrap_or(0),
            game_over: l
                .get_cached(
                    process,
                    self.game_mode,
                    keys::FL_GAME_OVER,
                    &mut self.hints.game_over,
                )
                .and_then(avm1::as_bool)
                .unwrap_or(false),
        })
    }

    /// `Chrono.get()` en millisecondes, et le `frameTimer` brut.
    ///
    /// ```mt
    /// function get() {
    ///     if ( fl_stop )  return haltedTimer;
    ///     else            return Math.floor( frameTimer-gameTimer );
    /// }
    /// ```
    fn chrono(&mut self, process: &Process) -> Option<(i64, i64)> {
        let l = &self.layout;
        let chrono =
            l.child_cached(process, self.game_mode, keys::GAME_CHRONO, &mut self.hints.chrono)?;

        let frame = avm1::as_int(l.get_cached(
            process,
            chrono,
            keys::FRAME_TIMER,
            &mut self.hints.frame,
        )?)?;

        let stopped = l
            .get_cached(process, chrono, keys::FL_STOP, &mut self.hints.stop)
            .and_then(avm1::as_bool)
            .unwrap_or(false);
        if stopped {
            if let Some(halted) = l
                .get_cached(process, chrono, keys::HALTED_TIMER, &mut self.hints.halted)
                .and_then(avm1::as_int)
            {
                return Some((halted, frame));
            }
        }

        let game = avm1::as_int(l.get_cached(
            process,
            chrono,
            keys::GAME_TIMER,
            &mut self.hints.game,
        )?)?;
        Some((frame - game, frame))
    }
}

/// Le process plugin : celui des process EternalTwin ou Pepper Flash est
/// charge. Il n'existe que tant qu'une instance Flash vit, donc son pid ne doit
/// jamais etre mis en cache.
///
/// EternalTwin en lance une demi-douzaine sous le meme nom, et savoir lequel
/// porte le plugin demande de s'y attacher -- il n'y a pas d'autre moyen de
/// lister ses modules. Or le runtime journalise chaque attache et chaque
/// detachement : sonder les six a chaque tick noie les logs sous des centaines
/// de lignes par seconde, ce qui rend le debugger inutilisable des qu'on quitte
/// une partie sans fermer l'application.
///
/// `rejected` retient donc les pids deja ecartes, pour ne sonder que les
/// nouveaux venus -- et le process PPAPI en est toujours un, puisqu'il nait
/// avec la partie. Les pids disparus en sont retires, pour que la liste ne
/// grossisse pas indefiniment.
pub fn attach_plugin(
    names: &[&str],
    rejected: &mut Vec<ProcessId>,
) -> Option<(Process, (u64, u64))> {
    let mut alive = Vec::new();
    let mut found = None;

    for name in names {
        let Some(pids) = Process::list_by_name(name) else {
            continue;
        };
        for pid in pids {
            alive.push(pid);
            if found.is_some() || rejected.contains(&pid) {
                continue;
            }
            let Some(process) = Process::attach_by_pid(pid) else {
                continue;
            };
            let range = PLUGINS.iter().find_map(|plugin| {
                let (addr, size) = process.get_module_range(plugin).ok()?;
                Some((addr.value(), addr.value() + size))
            });
            match range {
                Some(range) => found = Some((process, range)),
                None => rejected.push(pid),
            }
        }
    }

    rejected.retain(|pid| alive.contains(pid));
    found
}
