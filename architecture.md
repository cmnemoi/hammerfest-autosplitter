# Architecture

Comment l'autosplitter est construit, et pourquoi il l'est ainsi. Ce que le jeu
cache en memoire et comment on l'y trouve est l'autre document,
[reverse-engineering.md](reverse-engineering.md).

---

## Deux crates, et la separation n'est pas decorative

```text
core/     logique pure : quand demarrer, splitter, remettre a zero, lacher une
          resolution ; decodage des atomes AVM1. Aucune dependance, aucun acces
          memoire. -> les tests sont la.

src/      infrastructure : trouver le process du plugin, balayer le tas, lire
          les objets AVM1, parler a LiveSplit. Ne decide de rien.
```

Les symboles du runtime ASR n'existent que dans le bac a sable WebAssembly.
**Tout ce qui touche a `asr` est donc intestable sur la machine de
developpement**, et ce qui doit etre teste doit en etre libre. D'ou le crate
`core`, qui recoit un `State` et rend des `Actions`.

`src/lib.rs` se contente de lire un etat, de le passer a `core::Policy`, et
d'executer ce qu'elle repond.

Les tests portent sur les regles qui ont reellement casse : le raccourci du
niveau 0 qui saute de 0 a 10, les trois ecritures de `currentId` dans une meme
image, le chrono qui ne vaut pas zero au debut d'une partie, la remise a zero
qui ne doit pas se declencher avant d'avoir vu une partie, et l'origine du
temps reel dans ses deux cas.

---

## Le chronometrage

La regle de course fixe le depart : *the timer begins when the loading text
disappears and fades in to level 0*. C'est l'image ou `GameMode.fl_lock`
retombe -- `onViewReady` attache la vue et appelle `onLevelReady`, qui
deverrouille, dans la meme image.

Le balayage du tas, lui, aboutit apres. **Et il n'y a pas a gagner cette
course** : les objets AVM1 du SWF naissent tous ensemble, une demi-seconde
avant l'apparition du niveau. La fenetre vaut 0,55 s et un balayage complet en
coutait autant.

Il ne faut pas la gagner, parce que le jeu porte lui-meme l'instant du depart.
`GameMode.main()` sort sur `fl_lock` **avant** d'incrementer `duration`, qui
vaut donc exactement zero pendant tout l'ecran noir :

```text
origine    = frameTimer - duration     a la premiere lecture deverrouillee
temps reel = frameTimer - origine
```

Arrive a temps, `duration` est nulle et l'origine est `frameTimer`. Arrive en
retard, `duration` dit de combien. Verification de bout en bout : **6 ms
d'ecart sur 60,9 s**.

`Policy` pose cette origine une fois, puis rend `real_time_ms` a chaque tick.
Deux garde-fous, testes : l'origine est refaite quand `duration` ou
`frameTimer` recule -- donc a la partie suivante -- mais **pas** quand la
resolution est perdue et reprise en cours de partie. La refaire alors la
poserait trop tard de tout le temps passe entre les niveaux, que `duration` ne
compte pas, et le chrono reculerait sous les yeux du joueur.

### Pourquoi le temps exact va dans le canal *game time*

L'API ASR n'expose que `start`, `split`, `reset`, `set_game_time` et
`pause_game_time`. **Elle ne sait pas reculer un chrono deja demarre.** Le
*real time* de LiveSplit part donc de l'appel a `start()`, c'est-a-dire du
moment ou le balayage aboutit.

`set_game_time` accepte une valeur absolue. C'est donc lui qui porte le temps
juste, et `gameChrono` -- le chrono que le jeu affiche, pauses et transitions
de niveau exclues -- passe en variable a cote.

> Dans LiveSplit : **Compare Against -> Game Time**.

---

## La resolution, du moins cher au plus cher

```text
1. GameManager.current          quelques lectures, tentee a chaque tick
2. balayage : fVersion          -> le GameManager, qui nait avec le SWF
3. balayage : world             -> le GameMode directement, en dernier recours
```

Une fois l'ancre posee, une partie qui demarre se voit en quelques lectures.
Tout est revalide avant usage : le jeu reconstruit ses objets entre deux
parties, et un emplacement abandonne reste lisible en contenant une valeur
parfaitement plausible.

### Ce qui rend un balayage acceptable

| mecanisme | ce qu'il evite |
| --- | --- |
| amorce du layout avec les offsets deja mesures, verifiee avant usage | la recherche par contenu, qui coutait plusieurs passes a la premiere resolution d'une session |
| tri des candidats dans le tampon local | une lecture distante par objet String, et il y en a des milliers |
| une passe pour tous les candidats | huit relectures du tas quand la chaine apparait plusieurs fois |
| plus de repli une fois le layout eprouve | relire pour rien : si la vtable est la bonne et que la chaine n'y est pas, elle n'existe pas encore |
| balayage des seules regions neuves ou agrandies | relire cent Mio pour trouver ce qui est dans les quatre derniers |
| budget de 8 Mio ou 128 lectures avant de rendre la main | une pause par region quand la carte en compte beaucoup de petites |

### Le cache d'une seconde du runtime

C'est le mecanisme le moins evident, et celui qui comptait le plus.

`livesplit-auto-splitting` met la carte memoire en cache **une seconde par
processus attache** (`refresh_memory_ranges`, livesplit-core `377f598`). Le
balayage differentiel comparait donc des regions perimees : il ne pouvait pas
voir naitre celles ou le SWF venait de creer ses objets, et attendait la
prochaine expiration.

Un acces temporaire au meme PID rend une carte independante de ce cache.
`diagnostics::FreshMap` en prend une toutes les cent millisecondes pendant que
le module cherche la partie, et `resolve` balaye celle-la. L'acces principal et
les ancres de la partie restent valides.

Resultat : **douze departs, onze a 0 ms** de retard d'affichage. Le premier
d'un module neuf, caches vides, tombe a 135 ms.

### Ce qui coute encore

Une tentative infructueuse lit environ 272 Mio en quatre passes, dont 76,5 %
pour les deux recherches par contenu. C'est la que se trouvent les retards
rares. Le detail est dans l'historique git, commit `6be7eb9`.

---

## Le diagnostic ne vit pas dans le metier

**La compilation normale ne mesure rien** : aucune trace, aucun compteur, et le
`.wasm` ne contient meme pas les chaines correspondantes.

```sh
grep -c HF_ target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm   # 0
```

Tout ce qui mesure vit dans `src/diagnostics.rs`, derriere la feature du meme
nom. Le code metier ne fait que l'appeler : sans la feature, ces appels n'ont
pas de corps, et les types qu'ils manipulent sont vides.

Deux exceptions, qui sont du metier : l'horloge WASI -- importee directement,
car l'API ASR n'en expose aucune et l'`Instant` d'`asr` n'existe que sur la
cible wasi -- et `FreshMap`, dont le correctif ci-dessus depend.

### Mesurer

```sh
mise run build-diagnostics     # module trace, dans son propre dossier
mise run capture-startup       # enchaine des parties et releve leur delai
mise run summarize-startup     # resume un journal exporte
mise run probe-runtime         # mesure le cache de la DLL ASR de LiveSplit
```

La compilation `diagnostics` ecrit des lignes `HF_DIAG`, `HF_SCAN` et
`HF_START` que ces scripts lisent. Une compilation `known-flash` existe aussi :
elle reconnait le lecteur deja mesure a ses en-tetes PE, avant la premiere
partie.

---

## Les scripts

Trois familles, qui ne se lisent pas de la meme facon.

**Bibliotheques** -- elles ne s'executent pas seules, tout le reste s'appuie
dessus.

| fichier | role |
| --- | --- |
| `winmem.py` | lecture seule d'un process Windows par ctypes, sans dependance. Equivalent Windows de `memlib.py` du depot `hammerfest-re`, qui lisait `/proc/<pid>/mem` |
| `avm1.py` | le modele objet AVM1 : atomes, chaines, tables. Le layout est derive a l'execution, jamais suppose |
| `hfmap.py` | noms du source vers noms obfusques, depuis `vendor/hf.map.json` |

**Outils** -- ils servent encore.

| commande | ce qu'elle fait |
| --- | --- |
| `mise run state` / `watch` / `dump` | lire une partie en cours sans passer par LiveSplit |
| `mise run trace` | horodater un demarrage, du process au depart officiel, et ecrire un CSV |
| `mise run capture-startup` / `summarize-startup` / `probe-runtime` | mesurer le delai d'affichage |

**Releves** -- ils ont servi une fois, a etablir qu'aucun chemin de pointeurs
statique ne mene aux objets du jeu. Leur conclusion est en section 10 de
[reverse-engineering.md](reverse-engineering.md), et leur code n'a pas a etre
beau.

| fichier | question | reponse |
| --- | --- | --- |
| `anchors.py` | quelles adresses survivent au relancement d'une partie ? | aucune |
| `stable_slots.py` | quels emplacements le lecteur reutilise-t-il ? | aucun ne pointe vers le film courant |
| `xrefs.py` | combien de pointeurs du module sont cites par du code ? | 109 sur 550 |
| `vtable_globals.py` | quels globals les methodes AVM1 consultent-elles ? | trois, dont deux allocateurs |
| `ptrscan.py`, `findchain.py`, `checkchain.py` | existe-t-il une chaine courte du module au film courant ? | rien trouve ; la section 10 conclut sans eux |

Ils dependent des bibliotheques ci-dessus, donc ils ne se deplacent pas sans
elles -- c'est pourquoi ils restent ici plutot que de rejoindre le depot
`hammerfest-re`, qui porte un autre travail : la lecture du **score** sous
Linux.

---

## Ce qui reste ouvert

**Le *real time* de LiveSplit reste en retard** du delai de resolution. L'API
ne permet pas de le corriger ; le chrono juste est celui du canal *game time*.

**La fin de la run n'est pas implementee.** La regle dit *ends when the player
enters the door and can no longer control the character*. L'autosplitter
splitte a chaque changement de niveau et remet a zero sur `fl_gameOver` ; le
split final n'est pas traite.

**Aucun reglage n'est expose.** Le module applique `Rules::default()` :
demarrage, split au changement de niveau dans le monde principal, remise a
zero. Les reglages sauvegardes par d'anciennes versions sont ignores.

**Les hypotheses non levees** sont listees en fin de
[reverse-engineering.md](reverse-engineering.md) : une seule version du lecteur
Flash, une seule machine, les dimensions paralleles hors perimetre.
