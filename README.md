# hammerfest-autosplitter

Autosplitter [Hammerfest](https://github.com/motion-twin/hammerfest) pour
LiveSplit : il demarre le chrono quand le niveau 0 apparait, splitte a chaque
niveau franchi, et remet a zero en fin de partie. Tout seul.

Le jeu n'est pas modifie : lecture memoire seule, aucune ecriture, aucun patch.

## Installation

- Telecharger `hammerfest_autosplitter.wasm`, ou le compiler (voir
  [Contribuer](#contribuer)).
- Dans LiveSplit : **Edit Splits -> Activate**, puis choisir le fichier.
- Clic droit -> **Compare Against -> Game Time**.

Cette derniere etape n'est pas optionnelle. Le grand chrono de LiveSplit
affiche par defaut le *real time*, qui demarre quand l'autosplitter detecte la
partie -- quelques centaines de millisecondes trop tard. Le temps juste est
dans le canal *game time*.

## Utilisation

Rien a regler. Lancer une partie sur [EternalTwin](https://eternaltwin.org) :

1. le chrono demarre a l'instant ou le niveau 0 apparait ;
2. il splitte a chaque niveau franchi, y compris sur les raccourcis -- le
   niveau 0 mene directement au 10, les warpzones sautent jusqu'a trois
   niveaux, et chacun ne compte que pour un split ;
3. il remet a zero quand la partie se termine ou est abandonnee.

Le temps affiche est un **temps reel**, mesure depuis le depart officiel de la
run : il compte les pauses et les chargements. Le chrono interne du jeu, qui
les exclut, est publie a cote dans la variable `Chrono du jeu (ms)`.

| variable | contenu |
| --- | --- |
| `Niveau` | le numero affiche par le jeu, sans decalage |
| `Monde` | `xml_adventure` et les mondes paralleles |
| `Chrono du jeu (ms)` | le chrono que Hammerfest remonte en fin de partie |

Les niveaux des dimensions paralleles ne declenchent pas de split.

## Comment c'est possible

Le niveau et le temps ne sont pas des variables C : ce sont des proprietes
d'objets ActionScript 2 crees a l'execution par un SWF telecharge, dans le tas
d'AVM1. **Aucun chemin de pointeurs statique n'y mene**, et le SWF est
obfusque.

Deux choses rendent la lecture possible :

1. **La table des noms obfusques est publique.** `eternalfest/project-phoenix`,
   le decompilateur du jeu, s'appuie sur `game-types/src/lib/hf.map.json` --
   2952 entrees `clair -> obfusque`, MIT. On sait donc que `currentId`
   s'appelle `-BBEO` dans le SWF, sans avoir a le chercher.
2. **Une chaine connue du SWF sert d'ancre.** On la cherche dans le tas, ce qui
   donne l'objet String, puis les tables qui la citent, puis le `GameMode`.

Le layout memoire est mesure a l'execution, jamais suppose : il differe entre
Linux et Windows. Les preuves et les pieges sont dans
**[reverse-engineering.md](reverse-engineering.md)**.

Le chronometrage, lui, repose sur un fait du jeu : `GameMode.duration` ne court
que depuis l'apparition du niveau 0. L'instant du depart se **reconstruit donc
apres coup**, et le temps que met la recherche en memoire n'entre pas dans le
chronometrage. Detail dans **[architecture.md](architecture.md)**.

## Contribuer

Rust pour le module, Python pour l'exploration memoire.
[mise](https://mise.jdx.dev) installe le reste.

**Prerequis** : un linker systeme. Le `.wasm` se lie tout seul, mais `asr`
depend d'une macro procedurale, qu'il faut compiler *pour la machine*. Sous
Windows, les Build Tools de Visual Studio avec la charge de travail C++ :

```sh
winget install Microsoft.VisualStudio.2022.BuildTools \
  --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Sous Linux ou macOS, le linker du systeme suffit.

```sh
mise run build    # -> target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
mise run test     # les regles, dans core/, sans runtime ni memoire
```

Le module est **recharge tout seul** par
[asr-debugger](https://github.com/LiveSplit/asr-debugger) quand le fichier
change : le laisser ouvert et relancer `mise run build` suffit. Il donne les
logs, les variables et un faux timer.

```sh
tools/asr-debugger.exe target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
```

Pour lire une partie en cours sans passer par LiveSplit :

```sh
mise run state    # un releve : niveau, chrono, monde
mise run watch    # suit les changements de niveau en direct
mise run dump     # les tables GameMode, world et Chrono, noms en clair
```

Il faut une partie en cours : le process `--type=ppapi` qui porte Pepper Flash
n'existe que tant qu'une instance Flash vit.

**Avant de toucher au code**, lire [architecture.md](architecture.md) : il dit
ou les decisions se prennent, pourquoi le diagnostic ne vit pas dans le code
metier, et ce qui reste ouvert.

## Etat

**Prouve** sur EternalTwin, `pepflashplayer.dll` win32-x64 32.0.0.465 :
resolution sans adresse en dur, lecture du niveau et du temps, detection des
changements de niveau, remise a zero immediate en fin de partie.

**Mesure** : le depart est date a l'image pres, meme quand la recherche aboutit
en retard. Douze parties, onze avec un affichage immediat ; ecart de 6 ms sur
une minute entre le temps affiche et une horloge exterieure.

**Hors perimetre** : les dimensions paralleles, et le split final de la regle
de course -- *enters the door and can no longer control the character*.

## Credits

Le reverse engineering precedent (`cmnemoi/hammerfest-re`, lecture du score
sous Linux) et celui-ci ont ete realises par Claude.

`vendor/hf.map.json` vient d'[Eternalfest](https://gitlab.com/eternalfest),
MIT.
