//! Autosplitter Hammerfest pour LiveSplit (Auto Splitting Runtime).
//!
//! Ce module ne decide de rien. Il fait trois choses, toutes tournees vers le
//! runtime :
//!
//! 1. trouver le process du plugin Flash et y lire l'etat du jeu ;
//! 2. remettre cet etat a [`hammerfest_core::Policy`], qui decide ;
//! 3. executer ce qu'elle repond, et publier le temps et les variables.
//!
//! Tout ce qui se decide vit dans le crate `core`, sans memoire ni runtime,
//! sous tests. C'est la seule facon de tester ces regles : les symboles du
//! runtime ASR n'existent que dans le bac a sable WebAssembly.
//!
//! Le detail des mesures memoire est dans `hammerfest-level-re.md`.

#![no_std]

extern crate alloc;

#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

mod avm1;
mod hammerfest;

/// Noms de proprietes obfusques, extraits de `vendor/hf.map.json` par build.rs.
mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

use asr::{future::next_tick, settings::Gui, time::Duration, timer, Process};
use hammerfest_core::{Policy, Rules, State, TimerState};

use hammerfest::Game;

asr::async_main!(stable);
asr::panic_handler!();

/// EternalTwin lance plusieurs process du meme nom ; seul celui qui a charge
/// Pepper Flash nous interesse.
const PROCESS_NAMES: &[&str] = &["Eternaltwin.exe", "Eternaltwin", "etwin"];

/// Attente avant de retenter une resolution ratee, en ticks. Une resolution
/// balaye tout le tas : la repeter 60 fois par seconde serait absurde, mais
/// attendre une seconde de plus au debut d'une partie se voit. D'ou un depart
/// court qui s'allonge tant que rien n'est trouve.
const RESOLVE_MIN_COOLDOWN: u32 = 20;
const RESOLVE_MAX_COOLDOWN: u32 = 60;

/// Croissance du tas qui autorise a rebalayer sans attendre, en octets.
///
/// Le chargement du SWF le fait passer de deux a quatre-vingts Mio par bonds
/// de plusieurs Mio ; le jeu, une fois lance, ne le fait plus varier que de
/// quelques centaines de Kio. Le seuil separe les deux, et evite de rebalayer
/// en boucle sur du bruit d'allocateur.
const HEAP_GROWTH: u64 = 4 << 20;

#[derive(Gui)]
struct Settings {
    /// Demarrer le chrono au debut d'une partie
    #[default = true]
    auto_start: bool,

    /// Splitter a chaque niveau franchi
    #[default = true]
    split_on_level: bool,

    /// Ignorer les niveaux des dimensions paralleles
    #[default = true]
    main_world_only: bool,

    /// Afficher le temps reel exact comme game time
    #[default = true]
    use_game_time: bool,

    /// Remettre a zero quand la partie est abandonnee ou relancee
    #[default = true]
    auto_reset: bool,
}

impl Settings {
    fn rules(&self) -> Rules {
        Rules {
            auto_start: self.auto_start,
            split_on_level: self.split_on_level,
            main_world_only: self.main_world_only,
            auto_reset: self.auto_reset,
        }
    }
}

fn timer_state() -> TimerState {
    match timer::state() {
        timer::TimerState::NotRunning => TimerState::NotRunning,
        timer::TimerState::Running => TimerState::Running,
        timer::TimerState::Paused => TimerState::Paused,
        timer::TimerState::Ended => TimerState::Ended,
        _ => TimerState::Unknown,
    }
}

async fn main() {
    let mut settings = Settings::register();
    // Les process EternalTwin deja examines et ecartes : voir `attach_plugin`.
    let mut rejected = alloc::vec::Vec::new();
    // Ce qu'une resolution apprend et que la suivante reutilise.
    let mut anchor = hammerfest::Anchor::default();
    // Ce qu'on retient du binaire, lui, survit au process plugin.
    let mut binary = hammerfest::Binary::default();
    // La politique traverse les process : un plugin qui disparait fait partie
    // de l'histoire d'une partie.
    let mut policy = Policy::new();

    asr::print_message("Hammerfest: autosplitter demarre");

    loop {
        match hammerfest::attach_plugin(PROCESS_NAMES, &mut rejected) {
            Some((process, module)) => {
                asr::print_message("Hammerfest: plugin Flash attache");
                // Rien de ce qu'un autre process avait appris ne vaut ici :
                // l'ASLR deplace le module et le tas AVM1 est reconstruit.
                anchor.reset();
                run(
                    &process,
                    module,
                    &mut settings,
                    &mut anchor,
                    &mut binary,
                    &mut policy,
                )
                .await;
                asr::print_message("Hammerfest: plugin Flash ferme");
            }
            None => {
                settings.update();
                apply(policy.tick(timer_state(), &settings.rules(), None));
                next_tick().await;
            }
        }
    }
}

async fn run(
    process: &Process,
    module: (u64, u64),
    settings: &mut Settings,
    anchor: &mut hammerfest::Anchor,
    binary: &mut hammerfest::Binary,
    policy: &mut Policy,
) {
    let mut game: Option<Game> = None;
    let mut cooldown = 0u32;
    let mut backoff = RESOLVE_MIN_COOLDOWN;
    // Taille du tas au dernier balayage : voir HEAP_GROWTH.
    let mut heap = 0u64;
    // L'origine n'est annoncee qu'une fois par partie : c'est la seule trace
    // qui dise de combien le balayage est arrive en retard.
    let mut announced = false;

    while process.is_open() {
        settings.update();

        if game.is_none() {
            // La voie rapide suit `GameManager.current` : quelques lectures,
            // donc on peut la tenter a chaque tick. Le balayage complet, lui,
            // n'intervient que pour apprendre l'ancre, ou si elle a bouge.
            game = hammerfest::resolve_via_manager(process, anchor);

            if game.is_none() {
                // Le tas qui grandit d'un coup, c'est le SWF qui cree ses
                // objets : rebalayer tout de suite, sans attendre la
                // temporisation. C'est la seule fenetre qui compte -- la
                // partie commence une demi seconde plus tard -- et attendre
                // une temporisation fixe y ajoutait jusqu'a une seconde de
                // retard, au hasard de la tentative precedente.
                let now = hammerfest::heap_size(process);
                let grown = now > heap + HEAP_GROWTH;
                if cooldown > 0 && !grown {
                    cooldown -= 1;
                } else {
                    heap = now;
                    game = hammerfest::resolve(process, module, anchor, binary).await;
                    if game.is_none() {
                        cooldown = backoff;
                        backoff = (backoff * 2).min(RESOLVE_MAX_COOLDOWN);
                    }
                }
            }
            if game.is_some() {
                backoff = RESOLVE_MIN_COOLDOWN;
            }
        }

        let read = game.as_mut().and_then(|g| g.read(process));
        if let Some(state) = read.as_ref() {
            publish(state, game.as_ref().map_or("", |g| g.set));
        }

        let actions = policy.tick(timer_state(), &settings.rules(), read);
        match actions.real_time_ms {
            Some(ms) if !announced => {
                announced = true;
                asr::print_message(&alloc::format!(
                    "Hammerfest: depart date, {ms} ms deja ecoulees"
                ));
            }
            None => announced = false,
            _ => {}
        }

        // Le chrono est pose **apres** `start()`, jamais avant : demarrer une
        // course remet le game time a zero, donc une valeur posee plus tot
        // serait perdue et le chrono afficherait zero pendant une image.
        let drop_resolution = apply(actions);
        if let (true, Some(ms)) = (settings.use_game_time, actions.real_time_ms) {
            // Le temps reel de LiveSplit part de l'appel a `start()`, donc du
            // moment ou le balayage du tas a fini -- quelques centaines de
            // millisecondes trop tard, et variable. Celui-ci est calcule depuis
            // l'instant ou le niveau 0 est apparu, et pose en absolu : le
            // retard du balayage n'entre pas dans le chronometrage, il ne fait
            // que retarder le premier affichage.
            timer::pause_game_time();
            timer::set_game_time(Duration::milliseconds(ms));
        }

        if drop_resolution {
            // Perdre une partie annonce presque toujours la suivante : le jeu
            // recree son GameManager a chaque lancement, donc l'ancre meurt
            // avec la partie et il faut rebalayer. Attendre en plus serait du
            // delai pur -- on repart sans temporisation.
            game = None;
            cooldown = 0;
            backoff = RESOLVE_MIN_COOLDOWN;
        }

        next_tick().await;
    }
}

/// Execute ce que la politique a decide. Rend `true` si la resolution courante
/// doit etre relachee.
fn apply(actions: hammerfest_core::Actions) -> bool {
    if actions.reset {
        timer::reset();
    }
    if actions.start {
        asr::print_message("Hammerfest: partie lancee");
        timer::start();
    }
    if actions.split {
        timer::split();
    }
    actions.drop_resolution
}

/// Ce que LiveSplit affiche a cote du chrono.
fn publish(state: &State, set: &str) {
    // `GameInterface.setLevel` ecrit `""+currentId` : le numero affiche par le
    // jeu est bien cet index-la, sans decalage.
    timer::set_variable_int("Niveau", state.level);
    timer::set_variable("Monde", set);
    // Le chrono que le jeu remonte lui-meme en fin de partie
    // (`"T="+gameChrono.get()`). Il exclut les pauses et les transitions de
    // niveau, donc il ne peut pas servir de temps reel -- mais c'est le chiffre
    // que le joueur voit, d'ou son affichage a cote.
    timer::set_variable_int("Chrono du jeu (ms)", state.chrono_ms);
    // Ce qui date le depart de la run. Au premier affichage, le chrono doit
    // valoir cette duree-la : c'est ce qui distingue un rattrapage normal d'une
    // origine fausse.
    timer::set_variable_int("Duree de jeu (ms)", state.duration_ms);
    if state.dim != 0 {
        timer::set_variable_int("Dimension", state.dim);
    }
}
