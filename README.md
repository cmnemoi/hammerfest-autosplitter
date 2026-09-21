# hammerfest-autosplitter

Autosplitter [Hammerfest](https://github.com/motion-twin/hammerfest) pour
LiveSplit : splitte au changement de niveau et affiche le temps reel corrige
depuis le depart. L'IGT reste disponible comme information secondaire.

Le jeu n'est pas modifie : lecture memoire seule, aucune ecriture, aucun patch.

## Ce qui est lu

| valeur | provenance |
| --- | --- |
| niveau | `GameMode.world.currentId` -- le numero affiche par le jeu, sans decalage |
| temps reel | `Chrono.frameTimer` moins l'instant du depart, tous deux lus en memoire |
| chrono du jeu (IGT) | `GameMode.gameChrono` -- publie en variable pour information |

La regle de course fixe le depart : *the timer begins when the loading text
disappears and fades in to level 0*. C'est l'image ou `GameMode.fl_lock`
retombe, et cet instant se **reconstruit apres coup** : `GameMode.duration` ne
court que depuis lui, donc elle dit de combien on est arrive en retard.

```text
origine    = frameTimer - duration       a la premiere lecture deverrouillee
temps reel = frameTimer - origine
```

`frameTimer` est `Std.getTimer()`, et `Chrono.update()` tourne avant le test de
pause : il suit donc le temps reel sans jamais s'arreter. Mesure de bout en
bout : **6 ms d'ecart sur 60,9 s**.

C'est ce qui sort le balayage du tas du chemin critique -- il ne retarde plus
que l'affichage, pas le chronometrage. Le detail est en
[§9](hammerfest-level-re.md).

## Comment ca marche

Ces valeurs ne sont pas des variables C : ce sont des proprietes d'objets
ActionScript 2 crees a l'execution par un SWF telecharge, dans le tas d'AVM1.
Aucun chemin de pointeurs statique n'y mene, et le SWF est obfusque.

Deux choses rendent la lecture possible :

1. **La table de correspondance des noms est publique.** `eternalfest/obf` ne
   produit pas ces noms-la, mais `eternalfest/project-phoenix` -- le
   decompilateur du jeu -- s'appuie sur `game-types/src/lib/hf.map.json`, 2952
   entrees `clair -> obfusque`, MIT. Copiee dans `vendor/`. On sait donc que
   `currentId` s'appelle `-BBEO` dans le SWF, sans avoir a le chercher.
2. **Une chaine connue du SWF sert d'ancre.** On cherche `]=[]8` (`world`) dans
   le tas, ce qui donne l'objet String, puis les tables qui possedent cette
   clef, puis le GameMode.

Le layout memoire, lui, est mesure a l'execution et non suppose : il differe
entre Linux et Windows. Le detail, les preuves et les pieges sont dans
**[hammerfest-level-re.md](hammerfest-level-re.md)**.

## Prerequis

- [mise](https://mise.jdx.dev) installe les outils declares dans `mise.toml`
  (Python, Rust, cible `wasm32-unknown-unknown`).
- **Un linker systeme.** Le `.wasm` se lie tout seul, rustc embarquant
  `rust-lld` pour wasm32, mais `asr` depend de `bytemuck` avec la feature
  `derive`, qui est une macro procedurale : il faut donc compiler et lier des
  artefacts *pour la machine*. Sous Windows cela veut dire les Build Tools de
  Visual Studio avec la charge de travail C++ :

  ```sh
  winget install Microsoft.VisualStudio.2022.BuildTools \
    --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```

  Sous Linux ou macOS, le linker du systeme suffit (`build-essential`,
  Xcode command line tools).

## Architecture

Deux crates, et la separation n'est pas decorative : les symboles du runtime
ASR n'existent que dans le bac a sable WebAssembly, donc **tout ce qui touche a
`asr` est intestable sur la machine de developpement**. Ce qui doit etre teste
doit donc en etre libre.

```text
core/          logique pure : quand demarrer, splitter, remettre a zero,
               abandonner une resolution ; decodage des atomes AVM1.
               Aucune dependance, aucun acces memoire. -> les tests sont la.

src/           infrastructure : trouver le process du plugin, balayer le tas,
               lire les objets AVM1, parler a LiveSplit. Ne decide de rien.
```

`src/lib.rs` se contente de lire un etat, de le passer a `core::Policy`, et
d'executer ce qu'elle repond.

```sh
mise run test
```

Les tests portent sur les regles qui ont reellement casse : le raccourci du
niveau 0 qui saute de 0 a 10, les trois ecritures de `currentId` dans une meme
frame, le chrono qui ne vaut pas zero au debut d'une partie, la remise a zero
qui ne doit pas se declencher avant d'avoir vu une partie.

## Utilisation

La compilation normale inclut le budget de recherche (8 Mio ou 128 lectures
avant de rendre la main). C'est elle qu'on charge dans LiveSplit et dans
asr-debugger. La ligne annoncee au chargement dit laquelle tourne :

```text
Hammerfest: autosplitter demarre (budget=true, diagnostics=false)
```

Le module rafraichit la carte memoire toutes les 100 ms pendant qu'il cherche
la partie -- celle que le runtime lui donne a une seconde de retard, et c'est
ce retard qui decalait le premier affichage. Il lui faut pour cela l'horloge
monotone WASI, verifiee avec le composant ASR de LiveSplit 1.8.37.

**La compilation normale ne mesure rien.** Aucune trace, aucun compteur : le
`.wasm` ne contient meme pas les chaines correspondantes. Tout ce qui mesure
est derriere la feature `diagnostics`, decrite dans
[diagnostic-affichage.md](diagnostic-affichage.md).

```sh
mise run build     # -> target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
```

Charger le `.wasm` dans LiveSplit (Edit Splits -> Activate), ou dans
asr-debugger pendant le developpement.

Le module ne propose aucun reglage. Le demarrage, les splits au changement
de niveau dans le monde principal et la remise a zero sont automatiques.
Le temps reel corrige est toujours envoye au canal **Game Time** de LiveSplit.
Il compte les pauses et les chargements. Choisir **Game Time** comme methode
de comparaison dans LiveSplit.

L'IGT est publie separement dans la variable `Chrono du jeu (ms)`.
Un composant de layout capable de lire cette variable peut l'afficher.
Les anciens reglages sauvegardes par LiveSplit sont ignores.

Le *real time* de LiveSplit part de l'appel a `start()`, et l'API ASR ne sait
pas reculer un chrono deja demarre. Il accuse donc le retard du balayage, 0,4 a
0,7 s. Le chrono juste est celui du canal *game time* : comparer contre lui.

### asr-debugger

[asr-debugger](https://github.com/LiveSplit/asr-debugger) fait tourner le
`.wasm` hors de LiveSplit, avec les logs, les variables et un faux timer. Des
binaires sont publies :

```sh
# Windows, dans tools/ qui est ignore par git
curl -L -o asr-debugger.zip https://github.com/LiveSplit/asr-debugger/releases/latest/download/asr-debugger-v0.1.2-x86_64-pc-windows-msvc.zip
tools/asr-debugger.exe target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
```

L'interface est un ensemble de panneaux :

| panneau | contenu |
| --- | --- |
| **Main** | le fichier charge, `Restart` / `Kill`, la case `Optimize`, l'etat du timer avec `Start` / `Reset` |
| **Logs** | ce que l'autosplitter ecrit ; `Save` les exporte dans un fichier |
| **Variables** | `Niveau`, `Monde`, `Chrono du jeu (ms)` |
| **Settings GUI** | aucun reglage expose |
| **Processes** | les process auxquels l'autosplitter s'est attache |
| **Performance** | le temps passe par tick |

Le `.wasm` est **recharge tout seul** quand le fichier change : laisser le
debugger ouvert et relancer `mise run build` suffit.

Le demarrage automatique se declenche quand une partie apparait la ou il n'y en
avait pas. Pour observer des splits au milieu d'une partie deja lancee, appuyer
sur `Start` a la main.

Decocher `Optimize` desactive l'optimisation du module, ce qui permet d'y
attacher LLDB et d'avancer pas a pas.

## Exploration memoire

Les scripts Python servent au reverse et a verifier une lecture sans passer par
LiveSplit. Ils n'ont aucune dependance : `ReadProcessMemory` via `ctypes`.

```sh
mise run state     # un releve : niveau, chrono, monde
mise run watch     # suit les transitions de niveau en direct
mise run dump      # affiche les tables GameMode, world et Chrono, noms en clair
```

Il faut une partie en cours : le process `--type=ppapi` qui porte Pepper Flash
n'existe que tant qu'une instance Flash est vivante.

```text
set            xml_adventure (dimension 0)
niveau         20   (precedent 19)
chrono         613806 ms
duration       19650.1 cycles  = 614.1 s

  niveau 20 -> 21  chrono 622766 ms  (+8927 ms)
  niveau 21 -> 22  chrono 650878 ms  (+28112 ms)
```

## Etat

**Prouve**, sur EternalTwin / `pepflashplayer.dll` win32-x64 32.0.0.465 :
resolution du GameMode sans adresse en dur, lecture du niveau et du chrono,
detection des changements de niveau, coherence des deux compteurs de temps du
jeu entre eux.

**Verifie en jeu** : la resolution et les splits fonctionnent dans LiveSplit, y
compris sur le raccourci du niveau 0, et la remise a zero est immediate en fin
de partie.

**Mesure** : le depart de la run est date a l'image pres, meme quand le
balayage aboutit une demi seconde trop tard. Quatre parties tracees par
`scripts/hf_trace.py`, ecart final de 6 ms sur une minute.

**Verifie en jeu** : quatre parties d'affilee, le chrono s'affiche avec 841,
501, 715 et **0 ms** de retard -- la derniere resolue pendant l'ecran noir,
donc a l'instant exact du depart. La valeur affichee, elle, est juste des la
premiere image dans les quatre cas.

**Reste en retard** : le *real time* de LiveSplit, du meme delai, faute d'une
API qui sache reculer un chrono demarre. Le chrono juste est celui du canal
*game time*.

**Hors perimetre** : les dimensions paralleles.

## Credits

Le reverse engineering precedent (`cmnemoi/hammerfest-re`, lecture du score sous
Linux) et celui-ci ont ete realises par Claude.

`vendor/hf.map.json` vient d'[Eternalfest](https://gitlab.com/eternalfest), MIT.
