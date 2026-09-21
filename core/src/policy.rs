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
    /// `GameMode.fl_lock` : vrai pendant l'ecran noir du debut, pendant les
    /// transitions de niveau et pendant la pause. Le jeu ne simule pas.
    pub locked: bool,
    /// `GameMode.duration`, convertie en millisecondes.
    ///
    /// `main()` sort sur `fl_lock` **avant** de l'incrementer : elle vaut donc
    /// exactement zero tant que le niveau 0 n'est pas apparu, et mesure
    /// ensuite le temps pendant lequel le jeu a tourne.
    pub duration_ms: i64,
}

/// `Data.SECOND` : cycles de jeu par seconde.
const SECOND: f64 = 32.0;

/// `GameMode.duration` en millisecondes.
///
/// `duration += Timer.tmod` a chaque image, et `Timer.tmod` vaut 1 par image a
/// la cadence de reference. La somme suit donc le temps reel : mesure sur une
/// partie de 67 s, l'ecart est de 0,1 %.
pub fn duration_ms(cycles: f64) -> i64 {
    (cycles * (1000.0 / SECOND)) as i64
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
    /// Temps reel ecoule depuis le depart officiel, en millisecondes.
    ///
    /// Absolu : un demarrage tardif se rattrape de lui-meme des la premiere
    /// lecture. C'est ce qui sort le balayage du tas du chemin critique.
    pub real_time_ms: Option<i64>,
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
    /// A-t-on vu cette partie naitre, plutot que de la trouver en cours ?
    launched: bool,
    /// Origine du temps reel, dans l'horloge du lecteur Flash.
    ///
    /// `Std.getTimer()` compte les millisecondes reelles depuis le demarrage
    /// du plugin. La poser une fois suffit : tout le reste est une
    /// soustraction, et rien ne peut plus deriver.
    origin: Option<i64>,
    /// Derniere `duration` lue : elle ne recule qu'a la partie suivante.
    duration_seen: i64,
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
            self.launched = true;
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

        // Le depart officiel est l'image ou `fl_lock` retombe : l'ecran noir
        // s'acheve et le niveau 0 apparait. `GameMechanics.onViewReady` appelle
        // `GameMode.onLevelReady`, qui deverrouille, dans la meme image que
        // l'attachement de la vue.
        //
        // Une seule formule couvre les deux cas, parce que `duration` ne court
        // que depuis ce deverrouillage-la :
        //
        //   * resolus a temps, on lit le premier etat deverrouille avec
        //     `duration` encore nulle : l'origine est `frameTimer`, a un tick ;
        //   * resolus en retard -- le cas courant, le balayage prend une demie
        //     seconde de trop -- `duration` dit de combien, et l'origine se
        //     reconstruit exactement.
        //
        // Le balayage sort donc du chemin critique : sa duree n'entre plus dans
        // le chronometrage, elle ne fait que retarder l'affichage.
        // Deux compteurs seulement peuvent reculer, et seulement d'une partie
        // a l'autre : `duration`, qui repart de zero avec le GameMode, et
        // `Std.getTimer()`, qui repart de zero avec le process plugin. Quand
        // l'un des deux recule, l'origine appartient a la partie precedente.
        //
        // Le test porte sur eux plutot que sur la perte de la resolution :
        // celle-ci est relachee et reprise en cours de partie, et
        // reconstruire l'origine a ce moment-la la placerait trop tard de tout
        // le temps passe entre deux niveaux, que `duration` ne compte pas.
        if self.origin.is_some_and(|o| now.frame_timer < o)
            || now.duration_ms < self.duration_seen
        {
            self.origin = None;
        }
        self.duration_seen = now.duration_ms;
        if self.origin.is_none() && !now.locked {
            self.origin = Some(now.frame_timer - now.duration_ms);
            if self.launched && rules.auto_start && timer == TimerState::NotRunning {
                actions.start = true;
            }
        }
        actions.real_time_ms = self.origin.map(|o| now.frame_timer - o);

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
        // L'origine appartient a la partie qui vient de finir : la garder
        // ferait courir le chrono de la suivante depuis le mauvais instant.
        self.origin = None;
        self.launched = false;
        self.duration_seen = 0;
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
    ///
    /// `frameTimer` court depuis le demarrage du plugin, `gameChrono` depuis la
    /// construction du GameMode, et `duration` depuis l'apparition du niveau 0.
    /// Les 550 ms qui separent les deux dernieres sont mesurees : 535, 539, 547
    /// et 562 ms sur quatre parties.
    fn at(level: i64, chrono_ms: i64) -> State {
        State {
            level,
            previous: level - 1,
            chrono_ms,
            frame_timer: 1_000 + chrono_ms,
            dim: 0,
            game_over: false,
            locked: false,
            duration_ms: (chrono_ms - 550).max(0),
        }
    }

    /// Le meme etat, mais verrouille : ecran noir, transition, ou pause.
    fn locked(level: i64, chrono_ms: i64) -> State {
        State {
            locked: true,
            ..at(level, chrono_ms)
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

    #[test]
    fn ne_demarre_pas_tant_que_l_ecran_noir_dure() {
        // Le niveau 0 n'est pas encore apparu : la run n'a pas commence, meme
        // si le GameMode existe et que gameChrono court deja.
        let mut r = Run::new();
        r.tick(None);
        let actions = r.tick(Some(locked(0, 300)));
        assert!(!actions.start);
        assert_eq!(actions.real_time_ms, None);
    }

    // -- origine du temps reel ----------------------------------------------

    #[test]
    fn pose_l_origine_a_l_image_du_deverrouillage() {
        // Resolus avant l'apparition du niveau : on voit la transition, et
        // `duration` est encore nulle.
        let mut r = Run::new();
        r.tick(None);
        r.tick(Some(locked(0, 300)));

        let mut depart = at(0, 550);
        depart.duration_ms = 0;
        let actions = r.tick(Some(depart));
        assert!(actions.start);
        assert_eq!(actions.real_time_ms, Some(0));

        // 2 s plus tard, le temps reel les a comptees.
        let mut suite = at(0, 2_550);
        suite.frame_timer = depart.frame_timer + 2_000;
        assert_eq!(r.tick(Some(suite)).real_time_ms, Some(2_000));
    }

    #[test]
    fn reconstruit_l_origine_quand_le_balayage_arrive_en_retard() {
        // Le cas courant : le balayage aboutit une demie seconde trop tard.
        // `duration` ne court que depuis le deverrouillage, donc elle dit de
        // combien -- et le temps reel affiche est juste des la premiere
        // lecture, sans rattrapage visible.
        let mut r = Run::new();
        r.tick(None);

        let mut tard = at(0, 1_150);
        tard.duration_ms = 600;
        let actions = r.tick(Some(tard));
        assert!(actions.start);
        assert_eq!(actions.real_time_ms, Some(600));
    }

    #[test]
    fn le_temps_reel_ne_s_arrete_ni_en_pause_ni_entre_deux_niveaux() {
        // `Chrono.update` tourne avant le test de pause et avant le `return`
        // sur `fl_lock` : `frameTimer` suit le temps reel quoi qu'il arrive.
        // Mesure : +13843 ms pour 13,9 s de pause.
        let mut r = Run::new().running();
        let depart = at(0, 550);
        r.tick(Some(depart));

        let mut pause = locked(3, 20_000);
        pause.frame_timer = depart.frame_timer + 30_000;
        pause.duration_ms = 12_000; // figee, elle
        assert_eq!(r.tick(Some(pause)).real_time_ms, Some(30_000));
    }

    #[test]
    fn garde_l_origine_quand_la_resolution_est_perdue_puis_reprise() {
        // Relacher la resolution en cours de partie est normal : l'ancre meurt,
        // on rebalaye. Reconstruire l'origine a ce moment-la la poserait trop
        // tard de tout le temps passe entre les niveaux, que `duration` ne
        // compte pas -- et le chrono reculerait sous les yeux du joueur.
        let mut r = Run::new().running();
        let depart = at(0, 550);
        r.tick(Some(depart));

        let mut plus_tard = at(6, 40_000);
        plus_tard.frame_timer = depart.frame_timer + 60_000;
        plus_tard.duration_ms = 38_000; // 22 s de transitions, non comptees
        assert_eq!(r.tick(Some(plus_tard)).real_time_ms, Some(60_000));

        r.tick(None); // resolution perdue, rebalayage

        let mut reprise = plus_tard;
        reprise.frame_timer += 1_000;
        reprise.duration_ms += 1_000;
        assert_eq!(r.tick(Some(reprise)).real_time_ms, Some(61_000));
    }

    #[test]
    fn oublie_l_origine_quand_l_horloge_recule() {
        // Un autre process plugin, donc une autre partie : `Std.getTimer()`
        // repart de zero. Garder l'ancienne origine donnerait un temps negatif.
        let mut r = Run::new().running();
        r.tick(Some(at(5, 60_000)));

        let mut neuve = at(0, 550);
        neuve.frame_timer = 900; // le plugin vient de naitre
        neuve.duration_ms = 0;
        assert_eq!(r.tick(Some(neuve)).real_time_ms, Some(0));
    }

    #[test]
    fn repart_d_une_origine_neuve_a_la_partie_suivante() {
        let mut r = Run::new().running();
        r.tick(Some(at(4, 30_000)));
        let mut fin = at(4, 31_000);
        fin.game_over = true;
        r.tick(Some(fin));

        r.tick(None);
        let mut neuve = at(0, 550);
        neuve.frame_timer = 400_000;
        neuve.duration_ms = 0;
        let actions = r.tick(Some(neuve));
        assert!(actions.start);
        assert_eq!(actions.real_time_ms, Some(0));
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
