//! Quand demarrer, splitter, remettre a zero, ou lacher une resolution.
//!
//! Une machine a etats pure : elle recoit a chaque tick ce que le jeu dit --
//! ou rien, si on n'a pas trouve de partie -- et rend les actions a executer.
//! Elle ne lit aucune memoire et ne connait pas LiveSplit.

/// Ce que le jeu dit de lui-meme a un instant donne.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct State {
    /// `GameMode.world.currentId` : le niveau, tel que le jeu l'affiche.
    pub level: i64,
    /// `SetManager._previousId`.
    pub previous: i64,
    /// `Chrono.get()`, en millisecondes.
    pub chrono_ms: i64,
    /// `Chrono.frameTimer` : avance a chaque frame tant que ce GameMode tourne.
    pub frame_timer: i64,
    /// `GameMode.currentDim` : 0 pour le monde principal.
    pub dim: i64,
    /// `GameMode.fl_gameOver`.
    pub game_over: bool,
}

/// Les reglages, vus du coeur : de simples booleens.
#[derive(Copy, Clone, Debug)]
pub struct Rules {
    pub auto_start: bool,
    pub split_on_level: bool,
    pub main_world_only: bool,
    pub auto_reset: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            auto_start: true,
            split_on_level: true,
            main_world_only: true,
            auto_reset: true,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TimerState {
    NotRunning,
    Running,
    Paused,
    Ended,
    Unknown,
}

/// Ce que l'appelant doit faire, dans cet ordre.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Actions {
    pub reset: bool,
    pub start: bool,
    pub split: bool,
    /// La resolution courante n'est plus valable : la relacher et rechercher.
    pub drop_resolution: bool,
}

impl Actions {
    fn nothing() -> Self {
        Self::default()
    }
}

/// Le premier niveau d'une aventure.
const FIRST_LEVEL: i64 = 0;
/// Ticks sans que `frameTimer` bouge avant de declarer le GameMode mort.
/// Genereux : Flash tourne a une trentaine d'images par seconde, et une fenetre
/// en arriere-plan est ralentie davantage.
const STALE_TICKS: u32 = 90;
/// Ticks sans lecture valide avant de considerer une partie abandonnee.
const LOST_BEFORE_RESET: u32 = 240;

#[derive(Default)]
pub struct Policy {
    /// Dernier etat confirme, celui sur lequel on a agi.
    prev: Option<State>,
    /// Lecture precedente, pas encore confirmee.
    seen: Option<State>,
    /// A-t-on constate l'absence de partie depuis la derniere vue ?
    saw_no_game: bool,
    /// A-t-on deja lu une partie tout court ?
    had_game: bool,
    lost: u32,
    heartbeat: i64,
    frozen: u32,
}

impl Policy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Un tick. `read` vaut None quand aucune partie n'a pu etre lue.
    pub fn tick(&mut self, timer: TimerState, rules: &Rules, read: Option<State>) -> Actions {
        let Some(now) = read else {
            return self.no_game(timer, rules);
        };

        let mut actions = Actions::nothing();
        self.lost = 0;
        self.had_game = true;

        // Une partie apparait la ou il n'y en avait pas : c'est un lancement.
        // Avoir constate l'absence est ce qui distingue ce cas d'un
        // autosplitter demarre alors qu'une partie tournait deja.
        if self.saw_no_game {
            self.saw_no_game = false;
            if rules.auto_start && timer == TimerState::NotRunning {
                actions.start = true;
            }
        }

        // `fl_gameOver` est le signal exact de fin de partie. On lache aussitot
        // la resolution : un GameMode termine reste lisible longtemps, avec des
        // valeurs plausibles.
        if now.game_over {
            if rules.auto_reset && timer == TimerState::Running {
                actions.reset = true;
            }
            self.forget();
            actions.drop_resolution = true;
            return actions;
        }

        // `frameTimer` avance a chaque frame tant que ce GameMode est celui qui
        // tourne. Fige, l'objet est mort -- typiquement apres un retour au menu.
        if now.frame_timer == self.heartbeat {
            self.frozen = self.frozen.saturating_add(1);
            if self.frozen >= STALE_TICKS {
                self.forget();
                actions.drop_resolution = true;
                return actions;
            }
        } else {
            self.heartbeat = now.frame_timer;
            self.frozen = 0;
        }

        // On n'agit que sur un niveau lu deux fois de suite.
        //
        // `Adventure.nextLevel` ecrit `currentId` trois fois dans la meme frame
        // -- 1, puis 0, puis 10 -- et rien ne synchronise notre lecture avec la
        // frame du jeu. Sans cette confirmation, tomber au milieu produirait
        // deux splits au lieu d'un, de facon aleatoire.
        if self.seen.map(|s| s.level) == Some(now.level) {
            self.decide(timer, rules, &now, &mut actions);
            self.prev = Some(now);
        }
        self.seen = Some(now);
        actions
    }

    fn no_game(&mut self, timer: TimerState, rules: &Rules) -> Actions {
        let mut actions = Actions::nothing();
        actions.drop_resolution = true;
        self.saw_no_game = true;
        self.prev = None;
        self.seen = None;

        // Ne compter les lectures perdues qu'apres avoir vu une partie : ne pas
        // (encore) en trouver n'est pas la meme chose que d'en avoir perdu une.
        // Le plugin Flash existe des l'ouverture de l'application, donc bien
        // avant qu'il y ait quoi que ce soit a resoudre.
        if self.had_game {
            self.lost = self.lost.saturating_add(1);
            if self.lost == LOST_BEFORE_RESET
                && rules.auto_reset
                && timer == TimerState::Running
            {
                actions.reset = true;
            }
        }
        actions
    }

    fn forget(&mut self) {
        self.prev = None;
        self.seen = None;
        self.saw_no_game = true;
    }

    fn decide(&self, timer: TimerState, rules: &Rules, now: &State, actions: &mut Actions) {
        let Some(prev) = self.prev else {
            return;
        };

        // Un timer deja fini n'accepte pas `start` : il faut le remettre a zero
        // d'abord, et seulement si on en a l'autorisation.
        if rules.auto_start
            && rules.auto_reset
            && timer == TimerState::Ended
            && now.level == FIRST_LEVEL
        {
            actions.reset = true;
            actions.start = true;
            return;
        }

        // Le chrono du jeu recule : une autre partie a commence sans qu'on ait
        // vu la transition.
        if now.chrono_ms + 2_000 < prev.chrono_ms {
            if rules.auto_reset && timer == TimerState::Running {
                actions.reset = true;
            }
            if rules.auto_start {
                actions.start = true;
            }
            return;
        }

        // Tout progres vers l'avant compte, pas seulement `+1`. Hammerfest
        // saute des niveaux : le raccourci du niveau 0 mene directement au 10
        // (`Adventure.nextLevel` sous `fl_warpStart`), et les warpzones
        // avancent de 1 a 3 (`SpecialManager.warpZone` -> `forcedGoto`).
        //
        // Un seul split par franchissement, quel que soit le nombre de niveaux
        // enjambes : l'itineraire d'un run passe par ces raccourcis, donc un
        // segment leur correspond. Un retour en arriere -- mort, restart --
        // n'est pas un progres.
        if rules.split_on_level
            && timer == TimerState::Running
            && now.level > prev.level
            && (!rules.main_world_only || now.dim == 0)
        {
            actions.split = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un etat de partie vivant, dont chaque test ne change que ce qui compte.
    fn at(level: i64, chrono_ms: i64) -> State {
        State {
            level,
            previous: level - 1,
            chrono_ms,
            frame_timer: 1_000 + chrono_ms,
            dim: 0,
            game_over: false,
        }
    }

    /// Joue une suite de lectures et rend les actions du dernier tick.
    ///
    /// Le timer est fourni par l'appelant : la politique ne le connait que par
    /// ce qu'on lui en dit, ce qui evite d'avoir a simuler LiveSplit.
    struct Run {
        policy: Policy,
        rules: Rules,
        timer: TimerState,
    }

    impl Run {
        fn new() -> Self {
            Self {
                policy: Policy::new(),
                rules: Rules::default(),
                timer: TimerState::NotRunning,
            }
        }

        fn running(mut self) -> Self {
            self.timer = TimerState::Running;
            self
        }

        fn tick(&mut self, read: Option<State>) -> Actions {
            let actions = self.policy.tick(self.timer, &self.rules, read);
            if actions.reset {
                self.timer = TimerState::NotRunning;
            }
            if actions.start {
                self.timer = TimerState::Running;
            }
            actions
        }

        /// Deux lectures identiques : la politique n'agit que sur un niveau
        /// confirme.
        fn confirm(&mut self, s: State) -> Actions {
            self.tick(Some(s));
            self.tick(Some(s))
        }
    }

    // -- demarrage ---------------------------------------------------------

    #[test]
    fn demarre_quand_une_partie_apparait_la_ou_il_n_y_en_avait_pas() {
        let mut r = Run::new();
        r.tick(None);
        assert!(r.tick(Some(at(0, 3_000))).start);
    }

    #[test]
    fn ne_demarre_pas_si_une_partie_tournait_deja_a_l_attache() {
        // Autosplitter charge en cours de run : la premiere lecture n'est pas
        // un lancement.
        let mut r = Run::new();
        assert!(!r.tick(Some(at(12, 90_000))).start);
    }

    #[test]
    fn demarre_meme_si_le_chrono_n_est_pas_a_zero() {
        // Le chrono court depuis la construction du GameMode : chargement et
        // intro compris. L'exiger petit empechait tout demarrage.
        let mut r = Run::new();
        r.tick(None);
        assert!(r.tick(Some(at(0, 12_000))).start);
    }

    #[test]
    fn demarre_meme_si_le_niveau_0_est_deja_passe() {
        // Un runner quitte le niveau 0 en deux secondes ; la resolution peut
        // mettre plus longtemps.
        let mut r = Run::new();
        r.tick(None);
        assert!(r.tick(Some(at(10, 4_000))).start);
    }

    // -- splits ------------------------------------------------------------

    #[test]
    fn splitte_sur_un_niveau_franchi() {
        let mut r = Run::new().running();
        r.confirm(at(3, 10_000));
        assert!(r.confirm(at(4, 20_000)).split);
    }

    #[test]
    fn splitte_sur_le_raccourci_du_niveau_0() {
        // `Adventure.nextLevel` sous `fl_warpStart` : 0 -> 10 d'un coup.
        let mut r = Run::new().running();
        r.confirm(at(0, 5_000));
        assert!(r.confirm(at(10, 9_000)).split);
    }

    #[test]
    fn un_seul_split_malgre_les_ecritures_intermediaires_de_la_meme_frame() {
        // Le jeu ecrit `currentId` a 1, puis 0, puis 10 dans la meme frame. Nos
        // lectures ne sont pas synchronisees avec elle : sans confirmation, on
        // splitterait deux fois.
        let mut r = Run::new().running();
        r.confirm(at(0, 5_000));
        let mut splits = 0;
        for level in [1, 0, 10, 10] {
            if r.tick(Some(at(level, 9_000))).split {
                splits += 1;
            }
        }
        assert_eq!(splits, 1);
    }

    #[test]
    fn ne_splitte_pas_en_arriere() {
        let mut r = Run::new().running();
        r.confirm(at(7, 30_000));
        assert!(!r.confirm(at(0, 31_000)).split);
    }

    #[test]
    fn ne_splitte_pas_dans_une_dimension_parallele() {
        let mut r = Run::new().running();
        r.confirm(at(3, 10_000));
        let mut s = at(4, 20_000);
        s.dim = 1;
        assert!(!r.confirm(s).split);
    }

    #[test]
    fn ne_splitte_pas_si_le_timer_ne_tourne_pas() {
        let mut r = Run::new();
        r.confirm(at(3, 10_000));
        assert!(!r.confirm(at(4, 20_000)).split);
    }

    // -- fin de partie et objets morts --------------------------------------

    #[test]
    fn remet_a_zero_sur_game_over_et_lache_la_resolution() {
        let mut r = Run::new().running();
        r.confirm(at(9, 40_000));
        let mut fin = at(9, 41_000);
        fin.game_over = true;
        let actions = r.tick(Some(fin));
        assert!(actions.reset);
        assert!(actions.drop_resolution);
    }

    #[test]
    fn lache_la_resolution_quand_le_battement_de_coeur_se_fige() {
        // Un GameMode abandonne reste lisible, avec un niveau et un chrono
        // plausibles ; seul `frameTimer` le trahit.
        let mut r = Run::new().running();
        let mort = at(9, 40_000);
        let mut actions = Actions::default();
        for _ in 0..STALE_TICKS + 1 {
            actions = r.tick(Some(mort));
        }
        assert!(actions.drop_resolution);
    }

    #[test]
    fn ne_lache_rien_tant_que_le_battement_avance() {
        let mut r = Run::new().running();
        for i in 0..STALE_TICKS * 2 {
            let s = at(9, 40_000 + i as i64);
            assert!(!r.tick(Some(s)).drop_resolution);
        }
    }

    // -- absence de partie --------------------------------------------------

    #[test]
    fn ne_remet_jamais_a_zero_avant_d_avoir_vu_une_partie() {
        // Le plugin Flash existe des l'ouverture de l'application. Compter les
        // lectures perdues avant la premiere partie remettait le timer a zero
        // pendant l'ecran de chargement.
        let mut r = Run::new().running();
        for _ in 0..LOST_BEFORE_RESET * 2 {
            assert!(!r.tick(None).reset);
        }
    }

    #[test]
    fn remet_a_zero_apres_avoir_perdu_une_partie_assez_longtemps() {
        let mut r = Run::new().running();
        r.confirm(at(5, 20_000));
        let mut reset = false;
        for _ in 0..LOST_BEFORE_RESET + 1 {
            reset |= r.tick(None).reset;
        }
        assert!(reset);
    }

    #[test]
    fn une_nouvelle_partie_apres_une_perte_redemarre() {
        let mut r = Run::new().running();
        r.confirm(at(5, 20_000));
        r.tick(None);
        r.timer = TimerState::NotRunning;
        assert!(r.tick(Some(at(0, 2_000))).start);
    }

    // -- reglages ------------------------------------------------------------

    #[test]
    fn respecte_les_reglages_desactives() {
        let mut r = Run::new();
        r.rules = Rules {
            auto_start: false,
            split_on_level: false,
            main_world_only: true,
            auto_reset: false,
        };
        r.tick(None);
        assert!(!r.tick(Some(at(0, 1_000))).start);

        r.timer = TimerState::Running;
        r.confirm(at(3, 10_000));
        assert!(!r.confirm(at(4, 20_000)).split);

        let mut fin = at(4, 21_000);
        fin.game_over = true;
        assert!(!r.tick(Some(fin)).reset);
    }
}
