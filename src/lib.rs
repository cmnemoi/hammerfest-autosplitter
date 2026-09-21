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
//!
//! **Ce qui mesure ne vit pas ici.** La compilation normale ne contient que
//! l'autosplitter : ni trace, ni compteur, ni horodatage. Tout cela est dans
//! [`diagnostics`], derriere la feature du meme nom, et le code metier ne fait
//! que l'appeler -- sans elle, ces appels n'ont pas de corps. La verification
//! tient en une commande : aucune chaine `HF_` n'apparait dans le `.wasm`
//! normal.

#![no_std]

extern crate alloc;

#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

mod avm1;
mod diagnostics;
mod hammerfest;

/// Noms de proprietes obfusques, extraits de `vendor/hf.map.json` par build.rs.
mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

use asr::{future::next_tick, time::Duration, timer, Process};
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
    // Les process EternalTwin deja examines et ecartes : voir `attach_plugin`.
    let mut rejected = alloc::vec::Vec::new();
    // Ce qu'une resolution apprend et que la suivante reutilise.
    let mut anchor = hammerfest::Anchor::default();
    // Ce qu'on retient du binaire, lui, survit au process plugin.
    let mut binary = hammerfest::Binary::default();
    // La politique traverse les process : un plugin qui disparait fait partie
    // de l'histoire d'une partie.
    let mut policy = Policy::new();

    // Une seule ligne au chargement, qui dit quelle compilation tourne : c'est
    // ce qu'on cherche en premier dans un journal qu'on nous envoie.
    asr::print_message(&alloc::format!(
        "Hammerfest: autosplitter demarre (budget={}, diagnostics={})",
        cfg!(feature = "scan-budget"),
        cfg!(feature = "diagnostics"),
    ));
    diagnostics::event("module_started");

    loop {
        match hammerfest::attach_plugin(PROCESS_NAMES, &mut rejected) {
            Some((process, module, pid)) => {
                asr::print_message("Hammerfest: plugin Flash attache");
                diagnostics::event("plugin_attached");
                // Rien de ce qu'un autre process avait appris ne vaut ici :
                // l'ASLR deplace le module et le tas AVM1 est reconstruit.
                anchor.reset();
                #[cfg(feature = "known-flash")]
                {
                    let matched = binary.recognize(&process, module);
                    asr::print_message(&alloc::format!(
                        "HF_DIAG event=binary_profile t_us={} matched={matched}",
                        diagnostics::now_us()
                    ));
                }
                run(
                    &process,
                    pid,
                    module,
                    &mut anchor,
                    &mut binary,
                    &mut policy,
                )
                .await;
                asr::print_message("Hammerfest: plugin Flash ferme");
            }
            None => {
                apply(policy.tick(timer_state(), &Rules::default(), None));
                next_tick().await;
            }
        }
    }
}

async fn run(
    process: &Process,
    pid: asr::ProcessId,
    module: (u64, u64),
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
    let mut fresh_map = diagnostics::FreshMap::default();
    #[cfg(feature = "diagnostics")]
    let mut last_read = None;
    #[cfg(feature = "diagnostics")]
    let mut last_loop = diagnostics::now_us();

    while process.is_open() {
        #[cfg(feature = "diagnostics")]
        {
            let now = diagnostics::now_us();
            if now - last_loop > 50_000 {
                asr::print_message(&alloc::format!("HF_DIAG event=loop_gap t_us={now} elapsed_us={}", now - last_loop));
            }
            last_loop = now;
        }

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
                let ranges = fresh_map.poll(pid);
                let now = ranges.map_or_else(|| hammerfest::heap_size(process), |rs| rs.iter().map(|(a,b)| b-a).sum());
                let grown = now > heap + HEAP_GROWTH;
                if cooldown > 0 && !grown {
                    cooldown -= 1;
                } else {
                    #[cfg(feature = "diagnostics")]
                    asr::print_message(&alloc::format!(
                        "HF_DIAG event=resolve_trigger t_us={} grown={grown} cooldown={cooldown} heap={now}",
                        diagnostics::now_us()
                    ));
                    heap = now;
                    game = hammerfest::resolve(process, module, anchor, binary, ranges).await;
                    if game.is_none() {
                        cooldown = backoff;
                        #[cfg(feature = "diagnostics")]
                        asr::print_message(&alloc::format!(
                            "HF_DIAG event=retry_wait t_us={} ticks={cooldown}", diagnostics::now_us()
                        ));
                        backoff = (backoff * 2).min(RESOLVE_MAX_COOLDOWN);
                    }
                }
            }
            if game.is_some() {
                backoff = RESOLVE_MIN_COOLDOWN;
            }
        }

        let read = game.as_mut().and_then(|g| g.read(process));
        #[cfg(feature = "diagnostics")]
        {
            let signature = read.as_ref().map(|s| (game.as_ref().unwrap().game_mode, s.locked));
            if signature != last_read {
                asr::print_message(&alloc::format!("HF_DIAG event=read t_us={} state={signature:?}", diagnostics::now_us()));
                last_read = signature;
            }
        }
        if let Some(state) = read.as_ref() {
            publish(state, game.as_ref().map_or("", |g| g.set));
        }

        let actions = policy.tick(timer_state(), &Rules::default(), read);
        if actions.start {
            diagnostics::started(actions.real_time_ms.unwrap_or(-1), pid);
        }
        match actions.real_time_ms {
            Some(ms) if !announced => {
                announced = true;
                asr::print_message(&alloc::format!(
                    "Hammerfest: depart date, {ms} ms deja ecoulees"
                ));
                #[cfg(feature = "diagnostics")]
                asr::print_message(&alloc::format!("HF_DIAG event=origin t_us={} elapsed_ms={ms} start={} fresh={}", diagnostics::now_us(), actions.start, true));
            }
            None => announced = false,
            _ => {}
        }

        // Le chrono est pose **apres** `start()`, jamais avant : demarrer une
        // course remet le game time a zero, donc une valeur posee plus tot
        // serait perdue et le chrono afficherait zero pendant une image.
        let drop_resolution = apply(actions);
        if let Some(ms) = actions.real_time_ms {
            // Valeur absolue : le retard de detection ne decale pas le temps.
            // LiveSplit ne doit pas ajouter sa propre avance entre les lectures.
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
        diagnostics::event("start_called");
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
