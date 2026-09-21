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
#[cfg(not(feature = "scan-budget"))]
const CHUNKS_PER_TICK: usize = 8;
// Meme plafond de volume que huit blocs de 1 Mio. Le plafond d'appels limite
// le travail lorsque la carte contient beaucoup de petites regions.
#[cfg(feature = "scan-budget")]
const BYTES_PER_TICK: u64 = 8 * CHUNK as u64;
#[cfg(feature = "scan-budget")]
const READS_PER_TICK: u64 = 128;

const MAX_LEVEL: i64 = 256;

/// Ce que le balayage doit compter pour se conduire : rien de plus.
///
/// Le budget decide quand rendre la main au runtime. Tout ce qui ne sert qu'a
/// mesurer -- durees, etapes, issue -- vit dans `trace`, qui n'existe pas dans
/// la compilation normale.
#[derive(Default)]
struct Scan {
    /// Lectures demandees au runtime, et octets qu'elles portaient.
    calls: u64,
    requested: u64,
    #[cfg(feature = "scan-budget")]
    last_yield_requested: u64,
    #[cfg(feature = "scan-budget")]
    last_yield_calls: u64,
    trace: crate::diagnostics::ScanTrace,
}

impl Scan {
    fn read_block(&mut self, process: &Process, base: u64, buf: &mut [u8]) -> bool {
        self.calls += 1;
        self.requested += buf.len() as u64;
        let ok = process.read_into_slice(Address::new(base), buf).is_ok();
        self.trace.read(buf.len(), ok);
        ok
    }

    fn paused(&mut self) {
        self.trace.paused();
        #[cfg(feature = "scan-budget")]
        {
            self.last_yield_requested = self.requested;
            self.last_yield_calls = self.calls;
        }
    }

    /// Rendre la main sur un volume, et non sur un nombre de blocs.
    ///
    /// Une carte faite de beaucoup de petites regions donnait une pause par
    /// region -- donc une pause pour quelques Kio lus, la ou huit blocs de
    /// 1 Mio en autorisent une pour huit Mio.
    fn should_yield(&self, _chunks: usize) -> bool {
        #[cfg(feature = "scan-budget")]
        {
            self.requested - self.last_yield_requested >= BYTES_PER_TICK
                || self.calls - self.last_yield_calls >= READS_PER_TICK
        }
        #[cfg(not(feature = "scan-budget"))]
        { _chunks % CHUNKS_PER_TICK == 0 }
    }

    /// Marque l'etape en cours. Sans la feature `diagnostics`, ne fait rien.
    fn stage(&mut self, next: &'static str) {
        self.trace.stage(next, self.requested, self.calls);
    }

    /// Issue de la tentative, pour la trace. Sans la feature, ne fait rien.
    fn outcome(&mut self, outcome: &'static str) {
        self.trace.outcome(outcome);
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.trace.stage("done", self.requested, self.calls);
        self.trace.finish(self.requested, self.calls);
    }
}

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
    lock: u64,
    duration: u64,
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
fn heap_iter(process: &Process) -> impl Iterator<Item = (u64, u64)> + '_ {
    use asr::MemoryRangeFlags as F;
    process.memory_ranges().filter_map(|r| {
        let flags = r.flags().ok()?;
        if !flags.contains(F::READ | F::WRITE) || flags.contains(F::PATH) {
            return None;
        }
        let (addr, size) = r.range().ok()?;
        let a = addr.value();
        (size > 0).then(|| (a, a + size))
    })
}

pub fn heap_ranges(process: &Process) -> Vec<(u64, u64)> {
    heap_iter(process).collect()
}

/// Total des octets engages dans ce tas.
///
/// Sert a savoir si un nouveau balayage a une chance d'apprendre quelque
/// chose. Les objets AVM1 du SWF ne naissent pas un par un : le tas passe de
/// deux a quatre-vingts Mio en quelques secondes, puis se stabilise. Tant
/// qu'il ne grandit pas, rebalayer ne peut rien trouver de plus -- et quand il
/// grandit d'un coup, attendre une temporisation est du delai pur.
///
/// Mesurer coute une centaine d'appels au runtime, contre quatre-vingts Mio
/// recopies pour un balayage.
pub fn heap_size(process: &Process) -> u64 {
    heap_iter(process).map(|(a, b)| b - a).sum()
}

// -- scans ------------------------------------------------------------------

/// Toutes les adresses alignees ou `pat` apparait.
async fn scan_bytes(
    process: &Process,
    ranges: &[(u64, u64)],
    pat: &[u8],
    align: usize,
    limit: usize,
    cost: &mut Scan,
) -> Vec<u64> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if cost.read_block(process, base, &mut buf[..n]) {
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
            if cost.should_yield(chunks) {
                cost.paused();
                next_tick().await;
            }
        }
    }
    out
}

/// Balaye les positions alignees ou `pat` apparait et appelle `on_hit` sur
/// chacune. Renvoyer `true` arrete le balayage.
///
/// `on_hit` recoit l'adresse **et les octets qui suivent**, jusqu'au bout du
/// bloc deja lu. C'est ce qui permet de trier les candidats sans repasser la
/// frontiere du process : un motif frequent -- la vtable des String, que tous
/// les milliers d'objets String du tas portent -- couterait sinon une lecture
/// distante par objet, et cette lecture-la coute bien plus cher que les octets
/// qu'elle rapporte.
async fn scan_bytes_until(
    process: &Process,
    ranges: &[(u64, u64)],
    pat: &[u8],
    align: usize,
    cost: &mut Scan,
    mut on_hit: impl FnMut(u64, &[u8]) -> bool,
) {
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if cost.read_block(process, base, &mut buf[..n]) {
                let mut i = 0;
                while i + pat.len() <= n {
                    if &buf[i..i + pat.len()] == pat && on_hit(base + i as u64, &buf[i..n]) {
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
            if cost.should_yield(chunks) {
                cost.paused();
                next_tick().await;
            }
        }
    }
}

/// Balaye les qwords alignes dont la valeur est l'une de `values`.
///
/// Une passe pour tous les candidats, et non une par candidat : chercher qui
/// pointe sur huit adresses coutait huit relectures du tas, soit sept cents
/// Mio par tentative infructueuse.
async fn scan_u64_any(
    process: &Process,
    ranges: &[(u64, u64)],
    values: &[u64],
    cost: &mut Scan,
    mut on_hit: impl FnMut(u64) -> bool,
) {
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if cost.read_block(process, base, &mut buf[..n]) {
                let mut i = 0;
                while i + 8 <= n {
                    let v = u64::from_le_bytes([
                        buf[i], buf[i + 1], buf[i + 2], buf[i + 3], buf[i + 4],
                        buf[i + 5], buf[i + 6], buf[i + 7],
                    ]);
                    if values.contains(&v) && on_hit(base + i as u64) {
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
            if cost.should_yield(chunks) {
                cost.paused();
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
    cost: &mut Scan,
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
            if cost.read_block(process, base, &mut buf[..n]) {
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
            if cost.should_yield(chunks) {
                cost.paused();
                next_tick().await;
            }
        }
    }
}

/// Un qword lu dans un tampon local, si l'offset y tient.
fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    let raw = buf.get(off..off + 8)?;
    Some(u64::from_le_bytes(<[u8; 8]>::try_from(raw).ok()?))
}

// -- resolution -------------------------------------------------------------

/// Ce qu'une resolution apprend et que les suivantes reutilisent.
///
/// Rien ici n'est l'adresse d'un objet de partie -- ce serait le piege. Les
/// deux adresses retenues appartiennent a des objets qui vivent aussi longtemps
/// que le SWF : la chaine internee d'une clef, et la table du `GameManager`.
/// Elles sont revalidees avant chaque usage, et l'`Anchor` est remis a zero a
/// chaque nouveau process plugin.
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
    /// Regions vues au dernier balayage, bornes comprises.
    ///
    /// Ce qui n'y figure pas est neuf : region fraichement engagee, ou region
    /// qui a grandi. C'est la que naissent les objets du SWF -- les balayages
    /// qui aboutissent ne lisent que seize Mio, ceux qui echouent en relisent
    /// deux cents. Voir `resolve`.
    regions: Vec<(u64, u64)>,
    /// Tentatives depuis le dernier balayage complet.
    sweeps: u32,
}

/// Ce qu'on retient du binaire lui-meme, d'un process plugin a l'autre.
///
/// Le process plugin va et vient, mais c'est toujours la meme DLL : ses
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

/// Layout mesure sur `pepflashplayer.dll` win32-x64 32.0.0.465, en offsets
/// relatifs au module.
///
/// Ce n'est pas une adresse en dur, c'est une **amorce**. Elle est verifiee
/// exactement comme une valeur derivee -- la chaine est relue et comparee --
/// et abandonnee sans bruit si le binaire differe, auquel cas la recherche
/// complete reprend.
///
/// Ce qu'elle fait gagner : sans elle, la premiere resolution d'une session
/// cherche la chaine par son contenu, puis refait une passe complete sur le
/// tas **par candidat** pour trouver qui pointe dessus. Avec elle, une seule
/// passe sur les en-tetes d'objets suffit, des la premiere partie. Le prix a
/// payer sur un binaire inconnu est cette passe-la, perdue une fois.
const MEASURED: Layout = Layout {
    module: (0, 0),
    str_vt: 0x1756db8,
    str_buf: 0x08,
    str_len: 0x30,
    tbl_vt: 0x174a460,
    profile: PROFILES[0],
    so_tbl: 0x30,
};

impl Binary {
    /// Profil deja mesure, reconnu par les en-tetes PE et quatre methodes.
    /// Le repli habituel reste actif si une seule verification echoue.
    #[cfg(feature = "known-flash")]
    pub fn recognize(&mut self, process: &Process, module: (u64, u64)) -> bool {
        let u32_at = |off| process.read::<u32>(Address::new(module.0 + off)).ok();
        let u16_at = |off| process.read::<u16>(Address::new(module.0 + off)).ok();
        if u16_at(0) != Some(0x5a4d)
            || u32_at(0x3c) != Some(0x158)
            || u32_at(0x158) != Some(0x4550)
            || u16_at(0x15c) != Some(0x8664)
            || u32_at(0x160) != Some(0x5fbd874b)
            || u16_at(0x170) != Some(0x20b)
            || u32_at(0x1a8) != Some(0x209e000)
            || u32_at(0x1b0) != Some(0x1f7c652)
        {
            return false;
        }
        for (slot, method) in [
            (MEASURED.str_vt, 0x4391d0),
            (MEASURED.str_vt + 8, 0x374620),
            (MEASURED.tbl_vt, 0x39ec60),
            (MEASURED.tbl_vt + 8, 0x3c2e10),
        ] {
            if read_u64(process, module.0 + slot) != Some(module.0 + method) {
                return false;
            }
        }
        self.layout = Some(MEASURED);
        true
    }

    /// Le layout, rebase sur le module de ce process-ci.
    fn layout(&self, module: (u64, u64)) -> Option<Layout> {
        let mut l = self.layout.unwrap_or(MEASURED);
        l.str_vt = module.0 + l.str_vt;
        l.tbl_vt = module.0 + l.tbl_vt;
        l.module = module;
        Some(l)
    }

    /// Le layout a-t-il deja servi a lire quelque chose dans ce binaire ?
    ///
    /// Tant que non, il faut garder le repli : l'amorce peut ne pas valoir
    /// pour cette version du lecteur. Une fois oui, le repli ne peut plus rien
    /// apprendre -- il ne ferait que relire le tas pour rien, et c'est
    /// exactement ce qui coutait sept cents Mio par tentative infructueuse
    /// pendant le chargement du SWF.
    fn proven(&self) -> bool {
        self.layout.is_some()
    }

    fn learn(&mut self, layout: &Layout) {
        let mut l = *layout;
        l.str_vt -= layout.module.0;
        l.tbl_vt -= layout.module.0;
        self.layout = Some(l);
    }
}

impl Anchor {
    /// Les adresses apprises dans un process n'ont aucun sens dans le
    /// suivant : l'ASLR les deplace, et le tas AVM1 est reconstruit.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Tentatives infructueuses tolerees avant de remettre en cause une ancre qui
/// n'a jamais rien donne. A raison d'une par seconde environ, cela laisse une
/// bonne demi-minute avant de reprendre le balayage.
const MANAGER_IDLE_LIMIT: u32 = 30;

/// Tentatives sans region neuve avant de relire le tas en entier.
///
/// Le filet de securite de la regle ci-dessus : un objet peut naitre dans de
/// la memoire deja engagee, et aucune region neuve ne le signalerait.
const FULL_SWEEP: u32 = 8;

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
    supplied_ranges: Option<&[(u64, u64)]>,
) -> Option<Game> {
    if let Some(game) = resolve_via_manager(process, anchor) {
        return Some(game);
    }

    let mut cost = Scan::default();
    let all = supplied_ranges.map_or_else(|| heap_ranges(process), |rs| rs.to_vec());
    if all.is_empty() {
        return None;
    }

    // Ne balayer que ce qui a change.
    //
    // Les objets du SWF naissent tous ensemble, dans de la memoire qui vient
    // d'etre engagee : les balayages qui aboutissent ne lisent que seize Mio,
    // ceux qui echouent en relisaient deux cents pour rien. Or c'est le prix
    // de ces echecs-la qui decide de tout -- pendant qu'un balayage inutile
    // dure, le niveau 0 apparait, et le chrono demarre en retard.
    //
    // Une region absente de la liste precedente est neuve, ou elle a grandi.
    // Quand il n'y en a aucune, rien n'a pu naitre depuis la derniere fois :
    // le balayage n'apprendrait rien, et on s'abstient. De loin en loin, tout
    // de meme, une passe complete -- pour le cas ou un objet naitrait dans de
    // la memoire deja engagee, que cette regle ne verrait jamais.
    let mut ranges: Vec<(u64, u64)> = all
        .iter()
        .filter(|r| !anchor.regions.contains(r))
        .copied()
        .collect();
    if ranges.is_empty() {
        anchor.sweeps += 1;
        if anchor.sweeps % FULL_SWEEP != 0 {
            cost.outcome("unchanged_ranges");
            return None;
        }
        ranges = all.clone();
    }
    anchor.regions = all.clone();
    // Les tables se cherchent partout, meme quand la chaine ne se cherche que
    // dans le neuf.
    let mut full = all;
    if let Some(addr) = anchor.last_game_mode {
        move_region_first(&mut ranges, addr);
        move_region_first(&mut full, addr);
    }

    // Poser l'ancre d'abord : une fois le GameManager connu, plus aucun
    // balayage n'est necessaire, et le lancement d'une partie se voit en
    // quelques lectures au lieu d'une demi-seconde a plusieurs secondes.
    if anchor.manager.is_none() {
        cost.stage("manager");
        if let Some((layout, tbl)) =
            scan_for_manager(process, module, &ranges, &mut full, anchor, binary, &mut cost).await
        {
            asr::print_message(&alloc::format!(
                "Hammerfest: GameManager 0x{tbl:x}, layout {}",
                layout.profile.name,
            ));
            binary.learn(&layout);
            anchor.layout = Some(layout);
            anchor.manager = Some(tbl);
            anchor.current_hint = 0;
            let game = resolve_via_manager(process, anchor);
            cost.outcome(if game.is_some() { "game_via_manager" } else { "manager_only" });
            return game;
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
    cost.stage("world_string");
    let (layout, strobj) = find_string(
        process,
        module,
        &ranges,
        keys::WORLD,
        &mut anchor.string_world,
        binary,
        &mut cost,
    )
    .await?;
    // La chaine internee et les tables qui la citent vivent dans le meme tas
    // AVM1 : commencer par sa region evite le plus souvent d'avoir a lire le
    // reste, et c'est ce qui rend la duree stable d'une fois sur l'autre.
    move_region_first(&mut full, strobj);

    cost.stage("world_tables");
    let game = scan_tables(process, &full, layout, strobj, keys::WORLD, &mut cost, |l, t| {
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
    cost.outcome("game_via_world");
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
    fresh: &[(u64, u64)],
    ranges: &mut [(u64, u64)],
    anchor: &mut Anchor,
    binary: &mut Binary,
    cost: &mut Scan,
) -> Option<(Layout, u64)> {
    // La chaine ne se cherche que dans ce qui a change -- c'est la que le SWF
    // vient de la creer. Les tables qui la citent, en revanche, se cherchent
    // partout : une fois la chaine trouvee, on sait qu'une partie existe, et
    // la passe suivante s'arrete a la premiere table valable.
    cost.stage("manager_string");
    let (layout, strobj) = find_string(
        process,
        module,
        fresh,
        keys::F_VERSION,
        &mut anchor.string_version,
        binary,
        cost,
    )
    .await?;

    // La chaine internee et les tables qui la citent vivent dans le meme tas
    // AVM1 : commencer par sa region evite le plus souvent d'avoir a lire le
    // reste. C'est ce qui rendait la duree stable sur le chemin de repli, et
    // cela manquait ici.
    move_region_first(ranges, strobj);

    cost.stage("manager_tables");
    scan_tables(process, ranges, layout, strobj, keys::F_VERSION, cost, |mut l, t| {
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
    cost: &mut Scan,
) -> Option<(Layout, u64)> {
    let units = key.encode_utf16().count() as u64;
    #[cfg(feature = "diagnostics")]
    asr::print_message(&alloc::format!(
        "HF_DIAG event=find_string t_us={} key={key} proven={} cached={}",
        crate::diagnostics::now_us(), binary.proven(), cache.is_some()
    ));
    if let Some(so) = *cache {
        if let Some(layout) = string_layout_at(process, module, so, key, units) {
            return Some((layout, so));
        }
        *cache = None;
    }

    // Vtable connue : une seule passe suffit, sur les en-tetes d'objets.
    if let Some(seed) = binary.layout(module) {
        cost.stage("string_seed");
        let mut found = None;
        scan_bytes_until(process, ranges, &seed.str_vt.to_le_bytes(), 8, cost, |so, rest| {
            // La longueur d'abord, et dans le tampon quand elle y tient : elle
            // ecarte presque tous les objets String, et la chaine elle-meme
            // n'est relue que pour les rares survivants.
            let len = u64_at(rest, seed.str_len as usize)
                .or_else(|| read_u64(process, so + seed.str_len));
            if len == Some(units) && seed.string_eq(process, so, key) {
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
        if binary.proven() {
            // La vtable est la bonne et la chaine n'y est pas : elle n'existe
            // pas encore. Le SWF ne l'a pas creee, et aucune autre recherche
            // ne la fera apparaitre.
            return None;
        }
    }

    // Repli : la vtable n'est pas connue, ou l'amorce ne vaut pas pour ce
    // binaire. Deux passes, pas davantage -- les octets de la clef d'abord,
    // puis une seule passe pour tous les candidats a la fois.
    let needle: Vec<u8> = key.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    cost.stage("string_bytes");
    let buffers = scan_bytes(process, ranges, &needle, 2, 8, cost).await;
    if buffers.is_empty() {
        return None;
    }

    let mut found = None;
    cost.stage("string_references");
    scan_u64_any(process, ranges, &buffers, cost, |slot| {
        for buf_off in STR_BUF_CANDIDATES {
            let Some(so) = slot.checked_sub(buf_off) else {
                continue;
            };
            if let Some(layout) = string_layout_at(process, module, so, key, units) {
                found = Some((layout, so));
                return true;
            }
        }
        false
    })
    .await;
    if let Some((_, so)) = found {
        *cache = Some(so);
    }
    found
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
    cost: &mut Scan,
    mut accept: impl FnMut(Layout, u64) -> Option<T>,
) -> Option<T> {
    let mut layout = layout;
    let mut result = None;
    scan_atoms(process, ranges, strobj, cost, |slot| {
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
            // `fl_lock` est vrai pendant l'ecran noir qui precede le niveau 0 :
            // c'est sa retombee qui donne le depart officiel de la run.
            locked: l
                .get_cached(process, self.game_mode, keys::FL_LOCK, &mut self.hints.lock)
                .and_then(avm1::as_bool)
                .unwrap_or(false),
            // Obligatoire, comme le niveau et le chrono : c'est elle qui date
            // le depart de la run. La lire a zero par defaut poserait
            // l'origine a l'instant de la resolution, donc un chrono court de
            // tout le retard du balayage -- et en silence. Mieux vaut declarer
            // la lecture invalide et rebalayer.
            duration_ms: hammerfest_core::duration_ms(avm1::as_number(
                process,
                l.get_cached(
                    process,
                    self.game_mode,
                    keys::DURATION,
                    &mut self.hints.duration,
                )?,
            )?),
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
/// Il ne meurt pas forcement avec la partie : quatre parties consecutives ont
/// ete observees dans un meme process. Ce qui meurt avec la partie, ce sont les
/// objets AVM1 -- d'ou la revalidation systematique plutot qu'une confiance au
/// process.
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
) -> Option<(Process, (u64, u64), ProcessId)> {
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
                Some(range) => found = Some((process, range, pid)),
                None => rejected.push(pid),
            }
        }
    }

    rejected.retain(|pid| alive.contains(pid));
    found
}
