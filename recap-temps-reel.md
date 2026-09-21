# Le depart de la run, et le temps reel

Ce qui a ete fait le 21/09/2026, ce qui est prouve, et ce qui reste ouvert.
Suite de [recap-21-09-2026.md](recap-21-09-2026.md), qui posait trois questions.

---

## 1. Les trois questions de depart, et leurs reponses

| question | reponse |
| --- | --- |
| demarrer le chrono sans attendre le balayage du tas | **resolu autrement** : le balayage reste sur le chemin, mais l'instant du depart se reconstruit apres coup, donc sa duree n'entre plus dans le chronometrage |
| `duration` est-il un meilleur *game time* que `gameChrono` ? | **non** : les deux s'arretent aux memes moments. `duration` sert a autre chose -- a dater le depart |
| trouver l'instant exact du depart en temps reel | **resolu** : `frameTimer - (frameTimer - duration)`, lu entierement en memoire, verifie a 6 ms sur une minute |

---

## 2. Ce que le source du jeu a tranche

Source lu : `motion-twin/hammerfest`, classes `Chrono`, `GameMode`, `Mode`,
`GameMechanics`, `Loader`.

### `duration` ne vaut rien de plus que `gameChrono` pour la pause **[PROUVE]**

```mt
GameMode.main()
    gameChrono.update();        // frameTimer = Std.getTimer()
    if ( fl_pause ) { ... }
    if ( fl_lock ) return;      // <- onPause() appelle lock()
    ...
    duration += Timer.tmod;     // <- jamais atteint pendant la pause
```

`onPause()` appelle `lock()`, qui arrete `gameChrono` **et** fait sortir
`main()` avant l'incrementation de `duration`. Mesure : 26,2 s de pause
donnent `duration` +0,0 cycle et `gameChrono` +0 ms.

L'hypothese 2 du recap precedent est donc morte : `duration` n'inclut pas les
pauses.

### Le depart officiel est la retombee de `fl_lock` **[PROUVE]**

La regle de course dit : *the timer begins when the loading text disappears and
fades in to level 0*.

```mt
GameMechanics.onViewReady()     la vue du niveau est attachee
    game.onLevelReady()
        unlock()                fl_lock = false
```

La vue est attachee et le mode deverrouille dans la **meme image**. L'ecran
noir se termine donc a `fl_lock = false`, a une image pres -- 31 ms.

### `gameChrono` ne part pas de la **[PROUVE]**

`Chrono.new()` laisse `suspendTimer` a `null`, donc le premier `start()` ne
decale pas `gameTimer`. `gameChrono` compte depuis la construction du `Chrono`,
chargement du niveau 0 compris.

Mesure sur quatre parties : **535, 539, 547, 562 ms** au deverrouillage. Le
*game time* affiche avant ce travail etait donc systematiquement une demi
seconde trop long.

### `duration` vaut exactement zero avant le depart **[PROUVE]**

Consequence du premier point : tant que le mode est verrouille, `main()` sort
avant l'incrementation. C'est ce qui rend le depart reconstructible.

---

## 3. La solution retenue

```text
origine    = frameTimer - duration     a la premiere lecture deverrouillee
temps reel = frameTimer - origine
```

Une seule formule couvre les deux cas :

* **resolus a temps** -- `duration` est nulle, l'origine est `frameTimer` ;
* **resolus en retard** -- `duration` dit de combien, et l'origine se
  reconstruit exactement.

Les deux compteurs qui la rendent possible :

| compteur | ce qu'il mesure | preuve |
| --- | --- | --- |
| `frameTimer` | `Std.getTimer()`, millisecondes reelles. `Chrono.update()` tourne avant le test de pause et avant le `return` sur `fl_lock`, donc il ne s'arrete jamais | +13 843 ms pour 13,9 s de pause |
| `duration` | temps pendant lequel le jeu a tourne, en cycles de `Data.SECOND` = 32 | 67,20 s pour 67,12 s reelles, soit 0,1 % |

**Verification de bout en bout : 6 ms d'ecart sur 60,9 s.** **[PROUVE]**

### Pourquoi le temps exact va dans le canal *game time*

L'API ASR n'expose que `start`, `split`, `reset`, `set_game_time` et
`pause_game_time`. **Elle ne sait pas reculer un chrono deja demarre.** Le
*real time* de LiveSplit part donc de l'appel a `start()`, c'est-a-dire du
moment ou le balayage aboutit.

`set_game_time`, lui, accepte une valeur absolue. C'est donc le *game time* qui
porte le temps exact, et `gameChrono` passe en variable a cote du chrono.

> **Dans LiveSplit : clic droit -> Compare Against -> Game Time.**
> Sinon le grand chrono affiche le temps reel, celui qu'on ne peut pas reparer.

---

## 4. Ce qui a ete implemente

### `core` -- ce qui decide, sous tests

* `atom.rs` : `double_at` et `decode_double`, car `duration` est un flottant
  boxe et non un entier d'atome ;
* `policy.rs` : `State` porte `locked` et `duration_ms` ; `Policy` pose
  l'origine une fois et rend `real_time_ms` dans `Actions` ;
* le chrono ne demarre plus a la premiere lecture, mais au **deverrouillage** :
  pendant l'ecran noir, la run n'a pas commence.

Trente tests, dont six neufs sur l'origine. Les deux qui comptent :

* `pose_l_origine_a_l_image_du_deverrouillage` -- resolution a l'heure ;
* `reconstruit_l_origine_quand_le_balayage_arrive_en_retard` -- le cas courant.

Deux garde-fous, testes aussi : l'origine est refaite quand `duration` ou
`frameTimer` recule -- donc a la partie suivante -- mais **pas** quand la
resolution est perdue et reprise en cours de partie. La refaire alors la
poserait trop tard de tout le temps passe entre les niveaux, que `duration` ne
compte pas, et le chrono reculerait sous les yeux du joueur.

### `src` -- ce qui lit et execute

* `build.rs` : clefs `FL_LOCK` et `DURATION`. `duration` n'etant pas renomme
  par l'obfuscateur, il passe par une table a part, avec une assertion qui
  casse la compilation si la convention de `hf.map.json` change ;
* `avm1::as_number` lit les deux formes d'atome -- `duration` vaut l'entier 0 a
  la construction, puis devient un flottant ;
* `duration` est **obligatoire** : si elle ne se lit pas, la lecture est
  invalide et on rebalaye. La lire a zero par defaut posait l'origine a
  l'instant de la resolution, donc un chrono court de tout le retard, en
  silence ;
* le chrono est pose **apres** `start()` : demarrer une course remet le game
  time a zero, donc une valeur posee plus tot etait perdue.

### Outillage

`scripts/hf_trace.py` (`mise run trace`) horodate le demarrage d'une partie,
du process jusqu'au depart officiel, et ecrit un CSV. C'est lui qui a fourni
toutes les mesures ci-dessus.

---

## 5. Le balayage du tas : ce qui a ete mesure et corrige

Le chrono affiche la bonne valeur des sa premiere image. Ce qui restait a
reduire, c'est le **retard avant qu'il apparaisse** -- de la cosmetique, pas de
la mesure.

### Ce que l'instrumentation a montre

Le module n'a pas d'horloge : le runtime ASR n'en expose aucune, et l'`Instant`
d'`asr` n'existe que sur la cible WASI. Deux compteurs en tiennent lieu, les
Mio recopies et les pauses rendues au runtime, annonces dans les logs.

Le premier releve a renverse le diagnostic :

```text
rien trouve, 634 Mio lus, 46 pauses      <- tentative infructueuse
GameManager 0x..., 18 Mio lus, 1 pauses  <- tentative qui aboutit
```

**Echouer coutait trente fois plus cher que reussir**, et le tas ne fait que
80 Mio : il etait relu neuf a douze fois. La cause etait dans le repli de
`find_string`, qui cherchait les octets de la clef -- presents dans le pool de
constantes bien avant l'objet String -- puis refaisait une passe complete
**par occurrence**.

Accessoirement : 764 Mio et 56 pauses en une seconde, c'est la lecture qui
domine, a environ 700 Mio/s. Les pauses ne coutent presque rien, le runtime
etant deja en retard quand il rappelle.

### Les corrections, dans l'ordre

| correction | effet |
| --- | --- |
| tri des candidats dans le tampon local | supprime une lecture distante par objet String, et il y en a des milliers |
| amorce du layout avec les offsets mesures, verifiee avant usage | la premiere resolution d'une session ne paie plus la recherche par contenu |
| une passe pour tous les candidats au lieu d'une par candidat | 9 passes -> 2 |
| plus de repli une fois le layout eprouve | si la vtable est la bonne et que la chaine n'y est pas, elle n'existe pas encore : relire n'apprend rien |
| balayer uniquement les regions neuves ou qui ont grandi | les objets du SWF naissent dans de la memoire fraichement engagee ; passe complete toutes les huit tentatives par securite |
| rebalayer des que le tas grandit de 4 Mio, au lieu d'attendre une temporisation | supprime jusqu'a une seconde d'attente pure |

### Le resultat, session par session

| session | parties | departs a 0 ms | mediane | maximum |
| --- | --- | --- | --- | --- |
| avant ce travail | -- | -- | -- | 0,5 a 4 s (observe a l'oeil) |
| premiere mesure | 4 | 1 | 715 ms | 841 ms |
| + amorce, tri local | 19 | 5 | 400 ms | 1148 ms |
| + regions neuves d'abord | 10 | 4 | 427 ms | 1002 ms |
| + une passe, plus de repli | 24 | 11 | 75 ms | 1303 ms |
| + balayage differentiel | 17 | **10** | **0 ms** | 906 ms |

---

## 6. Ce qui reste ouvert

### La variance d'affichage : resolue, et pas la ou je cherchais

**Mise a jour du 21/09/2026, apres ce recapitulatif.** Ce qui suit corrige ce
que cette section disait -- a savoir que la variance etait structurelle, et
qu'il fallait grappiller des Mio.

La cause etait ailleurs. `livesplit-auto-splitting` met la carte memoire en
cache **une seconde par processus attache** (`refresh_memory_ranges`). Le
balayage differentiel comparait donc des regions perimees : il ne pouvait pas
voir naitre celles ou le SWF venait de creer ses objets, et attendait la
prochaine expiration du cache. Toutes les optimisations decrites plus haut
etaient justes, mais elles s'attaquaient a un dixieme du probleme.

Un acces temporaire au meme PID rend une carte independante de ce cache. Le
module en prend une toutes les cent millisecondes pendant qu'il cherche la
partie.

Resultat : **douze departs, onze a 0 ms**. Le premier d'un module neuf, caches
vides, tombe a 135 ms.

Ce qu'il reste de couteux est mesure dans
[exploration-premier-depart.md](exploration-premier-depart.md) : une tentative
infructueuse lit 272 Mio en quatre passes, dont 76,5 % pour les deux
recherches par contenu. C'est la que se trouvent les retards rares.

Lecon, pour la prochaine fois : cinq iterations ont ete depensees a optimiser
sans instrumenter. Les deux qui ont compte sont venues apres la mesure.

### Le *real time* de LiveSplit reste en retard

Du meme delai, et rien dans l'API ASR ne permet de le corriger. Le contournement
est le canal *game time*. Une autre voie existerait -- demarrer le chrono des
l'apparition du process -- mais elle affiche faux pendant une a quatre secondes
avant de se corriger d'un bond : plus tot, mais pire.

### La fin de la run n'est pas implementee

La regle dit : *ends when the player enters the door and can no longer control
the character*. L'autosplitter splitte aujourd'hui a chaque changement de
niveau et remet a zero sur `fl_gameOver`. Le split final devrait etre l'entree
dans la porte du dernier niveau, ce qui n'est pas traite.

### Le `Loader` n'existe pas dans EternalTwin **[PROUVE]**

Le fondu de l'ecran de chargement du SWF d'origine (`Loader.mainGame`,
`loading._alpha -= 2` sur cinquante images) n'est pas observable : aucune table
portant `fVersion` ne porte `gameInst`. EternalTwin n'utilise pas ce loader.
C'est pourquoi le depart est defini sur `fl_lock` et non sur la fin du fondu.

### Hypotheses non levees

* l'amorce du layout vaut pour `pepflashplayer.dll` win32-x64 **32.0.0.465**,
  mesuree sur une seule machine. Une autre version la fait rejeter et le repli
  reprend -- mais cela n'a pas ete teste ;
* les dimensions paralleles restent hors perimetre ;
* le process plugin **ne meurt pas** avec la partie : quatre parties
  consecutives observees dans un meme process. La documentation affirmait le
  contraire, c'est corrige.

---

## 7. Ou regarder

| quoi | ou |
| --- | --- |
| la chaine de resolution et toutes les preuves | [hammerfest-level-re.md](hammerfest-level-re.md), section 9 pour le depart |
| ce que l'autosplitter lit et affiche | [README.md](README.md) |
| les regles, sous tests | `core/src/policy.rs` |
| la mesure du demarrage | `scripts/hf_trace.py`, `mise run trace` |
| le cout d'un balayage | les logs, lignes `Mio lus` et `pauses` |
