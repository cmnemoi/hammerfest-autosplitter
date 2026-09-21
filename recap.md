# Autosplitter Hammerfest — contexte / état des lieux

> **Document de depart, anterieur au code.** Il definissait l'objectif et
> l'architecture souhaitee avant qu'une ligne soit ecrite. Ce qui s'est
> reellement passe est dans [hammerfest-level-re.md](hammerfest-level-re.md) ;
> plusieurs choix envisages ici ont ete abandonnes, faute de chemin de
> pointeurs statique. Garde comme intention d'origine, pas comme etat des
> lieux.


## Objectif

Construire un autosplitter Hammerfest pour LiveSplit avec **Rust + ASR / WebAssembly**.

Premier objectif volontairement minimal :

```text
détecter le changement de niveau
→ déclencher un split
```

On ne cherche pas encore à gérer :

* les dimensions parallèles ;
* les niveaux secrets proprement ;
* le score ;
* les items ;
* toute la progression de l'aventure ;
* une abstraction parfaite multi-plateforme dès le premier commit.

Le speedrun ciblé pour l'instant reste dans le jeu principal.

La plateforme prioritaire est **Windows**, parce que c'est probablement celle utilisée par la majorité des joueurs/speedrunners.

Un environnement Windows est maintenant disponible pour faire les expérimentations.

---

# 1. Architecture générale

L'autosplitter sera compilé en `.wasm` et exécuté via l'Auto Splitting Runtime.

ASR fournit une API abstraite pour :

* trouver/attacher un process ;
* inspecter les modules ;
* lire sa mémoire ;
* scanner des signatures ;
* utiliser des watchers ;
* piloter le timer LiveSplit.

Le `.wasm` peut être cross-platform même si la façon de retrouver les données Hammerfest dans Pepper Flash doit être différente selon l'OS.

Séparer conceptuellement :

```text
Hammerfest semantic state
    current_level()
    ↓
AVM1 / Pepper Flash resolver
    ↓
ASR process memory API
```

La logique Hammerfest peut être commune.

Le resolver Pepper Flash peut avoir des implémentations différentes Windows/Linux.

---

# 2. Runtime concerné

Hammerfest tourne dans EternalTwin via Chromium + PPAPI Pepper Flash / AVM1.

Environnement déjà étudié côté Linux :

```text
EternalTwin Electron 11 / Chromium 87
Pepper Flash 32.0.0.465
AVM1 / ActionScript 2
Linux x86-64
```

ATTENTION :

Le layout mémoire Pepper Flash découvert sous Linux ne doit **pas** être supposé identique sous Windows.

Le SWF, en revanche, sera le même.

Donc :

```text
SWF / noms de propriétés / logique du jeu
=> communs

layout C++ Pepper Flash / ABI / offsets / modules
=> potentiellement spécifiques à Windows
```

---

# 3. Repositories utiles

Source Hammerfest originale :

* `motion-twin/hammerfest`

Outillage Eternalfest autour de l'obfuscation :

* `eternalfest/obf`
* `eternalfest/project-phoenix`

`project-phoenix` est notamment destiné à la déobfuscation de jeux MTASC/Haxe.

Il faut examiner ces deux repos avant de refaire inutilement du reverse dynamique.

Repo contenant le précédent reverse engineering du score :

* `cmnemoi/hammerfest-re`

IMPORTANT : le reverse engineering précédent et le code associé ont été réalisés par Claude. Ne pas l'attribuer à Charles.

---

# 4. Ce qu'on sait du source Hammerfest

Le niveau actuellement chargé est stocké dans :

```text
GameMode.world.currentId
```

Source :

`class/hammer/mode/GameMode.mt`

contient notamment :

```mt
var world       : levels.GameMechanics;
var dimensions  : Array<levels.GameMechanics>;
var currentDim  : int;
var gi          : gui.GameInterface;
```

`nextLevel()` :

```mt
function nextLevel() {
    fakeLevelId++;
    goto(world.currentId+1);
}
```

`goto(id)` finit par appeler :

```mt
world.goto(id);
```

Dans :

```text
class/hammer/levels/SetManager.mt
```

on trouve :

```mt
private var current      : levels.Data;
private var currentId    : int;
private var _previous    : levels.Data;
private var _previousId  : int;
```

et :

```mt
function setCurrent(id:int) {
    _previous = current;
    _previousId = currentId;
    current = levels[id];
    currentId = id;
}
```

Donc le signal minimal recherché est :

```text
GameMode.world.currentId
```

---

# 5. Pourquoi ignorer `dimensions` pour l'instant

Adventure configure plusieurs mondes :

```mt
addWorld("xml_adventure");
addWorld("xml_deepnight");
addWorld("xml_hiko");
addWorld("xml_ayame");
addWorld("xml_hk");
```

`dimensions[0]` correspond donc au monde principal `xml_adventure`.

Le code de sauvegarde utilise :

```mt
$reachedLevel : dimensions[0].currentId
```

Ce qui montre que `dimensions[0].currentId` est la progression principale canonique.

Mais pour le speedrun ciblé actuellement, on reste dans le monde principal.

Donc :

```text
currentDim == 0
world == dimensions[0]

=> world.currentId == dimensions[0].currentId
```

Pour la V1, utiliser seulement :

```text
world.currentId
```

Ne pas implémenter la logique des dimensions maintenant.

---

# 6. Reverse déjà réalisé côté score

Le précédent RE Linux permet de retrouver dynamiquement le score à travers le heap AVM1.

Chaîne actuelle :

```text
scan "70dik"
→ String internée
→ propriété GameInterface
→ GameInterface.realScores
→ array
→ ["0"]
→ score encodé comme AVM1 integer
```

Clés obfusquées déjà observées :

```text
"70dik"   = GameInterface.realScores
"[t}LJ("  = GameInterface.fakeScores
```

D'autres clés observées incluent :

```text
"{8"
"8}I*]"
"+-(0"
```

Ne pas inventer leur signification sans preuve.

---

# 7. Ancre importante : GameInterface → GameMode

Dans le précédent RE, `GameInterface` est référencé via la propriété :

```text
"{8"
```

depuis un objet ayant notamment :

```text
_name = "$adventure"
duration ≈ durée de partie
~94 propriétés
```

Cet objet correspond au GameMode / Adventure.

Le source dit que GameMode possède :

```mt
var gi : gui.GameInterface;
```

Donc `{8` est très probablement le nom obfusqué de `GameMode.gi`.

Attention au sens :

```text
GameMode
   -- "{8}" -->
GameInterface
```

Le précédent code peut retrouver le GameMode en remontant les références inverses depuis GameInterface.

---

# 8. Graph AVM1 existant

Dans `hammerfest-re`, `graph.py` construit un graphe du heap AVM1 avec notamment :

```text
tables     : table addr -> [(key_atom, val_atom)]
so2tbl     : ScriptObject -> table
tbl2so     : table -> ScriptObject
rev        : références inverses
```

Il permet de :

* décoder les noms de propriétés ;
* remonter les parents d'un objet ;
* dumper récursivement les objets ;
* décoder les integers AVM1.

Ancien layout Linux prouvé :

## String

```text
+0x00 vtable
+0x08 ptr buffer UTF-16LE
+0x30 length
size 0x38
```

## ScriptObject

```text
ScriptObject +0x30 -> property table
```

## Property table

```text
table +0x08 -> capacity
table +0x18 -> atoms[2 * capacity]

pairs:
(value, key)
```

## Atom integer

Pour un entier `n` :

```text
atom = n << 3
```

donc :

```text
n = atom >> 3
```

Encore une fois : **layout Linux uniquement** tant qu'il n'a pas été vérifié sous Windows.

---

# 9. Obfuscation / Project Phoenix

Avant de chercher `world` et `currentId` par différences mémoire, vérifier si l'outillage Eternalfest permet de récupérer directement le mapping entre :

```text
GameMode.world
SetManager.currentId
```

et leurs noms obfusqués dans le SWF.

Ce qui serait idéal :

```text
GameMode.world
    -> "<clé obfusquée>"

SetManager.currentId
    -> "<clé obfusquée>"
```

Le SWF étant commun Windows/Linux, ce mapping serait réutilisable sur les deux OS.

Inspecter :

```text
eternalfest/obf
eternalfest/project-phoenix
```

Chercher notamment :

* mapping de symboles ;
* dictionnaire de renommage ;
* seed / clé ;
* génération de noms ;
* déobfuscation du bytecode AVM1 ;
* possibilité de produire un SWF avec noms restaurés ;
* scripts spécifiques à Hammerfest.

Si une “clé de déobfuscation” externe est nécessaire, on peut la demander à la communauté Eternalfest.

Ne pas passer des heures à retrouver `currentId` dynamiquement avant d'avoir vérifié Phoenix.

---

# 10. Si Phoenix ne donne pas directement les clés

Plan de fallback déjà identifié.

À partir de GameInterface :

```text
GameInterface
↑ reverse reference "{8}"
GameMode / Adventure
```

Ensuite chercher parmi les propriétés objet du GameMode l'objet correspondant à `world`.

Les GameMechanics / SetManager peuvent potentiellement être identifiés grâce à leur champ :

```mt
setName
```

qui devrait contenir des chaînes comme :

```text
xml_adventure
xml_deepnight
xml_hiko
xml_ayame
xml_hk
```

Donc on peut chercher un ScriptObject contenant la String :

```text
"xml_adventure"
```

Ce sera un excellent candidat pour le `GameMechanics` principal.

Une fois la table du bon GameMechanics trouvée, observer uniquement ses propriétés de type integer pendant :

```text
niveau 0
→ niveau 1
→ niveau 2
```

Le comportement attendu :

```text
currentId:
0 -> 1 -> 2

_previousId:
ancien -> 0 -> 1
```

Donc deux changements de niveau suffisent probablement à différencier `currentId` de `_previousId`.

---

# 11. Priorité Windows

La majeure partie de la communauté compétitive sera probablement sous Windows.

Il faut donc rapidement déterminer :

1. quel process EternalTwin contient réellement Pepper Flash ;
2. quel module Pepper Flash est chargé ;
3. version exacte du plugin ;
4. nom du DLL ;
5. si les structures AVM1 ont le même layout que sous Linux ;
6. sinon, retrouver les offsets Windows équivalents.

Ne pas supposer que :

```text
ScriptObject +0x30
String +0x08
String +0x30
```

restent identiques.

Il faut les vérifier.

---

# 12. Architecture Rust souhaitée

Rester simple.

Quelque chose dans cet esprit :

```rust
struct Hammerfest {
    // resolver state
}

impl Hammerfest {
    fn current_level(&mut self, process: &Process) -> Option<u32> {
        // locate / resolve / read GameMode.world.currentId
    }
}
```

Boucle autosplitter :

```rust
let mut old_level = None;

loop {
    let level = game.current_level(&process);

    if let (Some(old), Some(new)) = (old_level, level) {
        if new != old {
            // condition de split à préciser
            timer::split();
        }
    }

    old_level = level;

    next_tick().await;
}
```

Évidemment il faudra éviter les splits incorrects :

* chargement initial ;
* restart ;
* retour menu ;
* invalid pointer ;
* changement temporaire/recréation objet.

Ne pas implémenter ça naïvement avant d'observer précisément le lifecycle.

---

# 13. Important : ne pas cacher les observations

Pendant le reverse, toujours afficher explicitement :

```text
adresse
clé de propriété
atom brut
tag
valeur décodée
ancienne valeur
nouvelle valeur
```

Éviter les scripts qui encapsulent complètement les inputs/outputs derrière des helpers opaques.

On veut pouvoir auditer les observations.

Exemple de sortie utile :

```text
GameMode: 0x...
world property: "abc]"
world: 0x...

GameMechanics table:
  key="P]7"
  atom=0x10
  tag=int
  decoded=2

transition:
  1 -> 2
```

---

# 14. Robustesse

Le précédent RE du score a montré que les objets GameInterface peuvent être recréés entre les parties.

Les anciennes adresses peuvent rester lisibles ou être recyclées.

Donc :

```text
NE PAS stocker définitivement l'adresse finale.
```

Préférer :

```text
ancre stable
→ resolver
→ object graph
→ valeur courante
```

et re-résoudre quand :

* l'objet devient invalide ;
* une nouvelle partie démarre ;
* la valeur devient incohérente ;
* le process / SWF est recréé.

---

# 15. Première tâche concrète

Ordre recommandé :

1. Inspecter le repo existant `hammerfest-re`.
2. Comprendre le resolver actuel du score et le graph AVM1.
3. Inspecter `eternalfest/obf` et `eternalfest/project-phoenix`.
4. Déterminer si le mapping pour `GameMode.world` et `SetManager.currentId` est récupérable.
5. Identifier le process et Pepper Flash sous Windows.
6. Vérifier le layout AVM1 Windows.
7. Retrouver `world.currentId`.
8. Faire un petit programme/debugger Rust qui affiche :

```text
Current level: N
```

9. Vérifier les transitions sur plusieurs niveaux et après restart.
10. Seulement ensuite brancher ça sur :

```rust
timer::split()
```

---

# 16. Definition of Done V1

On considère le reverse V1 résolu quand, sous Windows, on peut lancer Hammerfest et obtenir de manière reproductible :

```text
level 0
level 1
level 2
...
```

depuis la mémoire du process, sans adresse hardcodée spécifique à une session.

On considère l'autosplitter V1 résolu quand LiveSplit split correctement sur les changements de niveau pertinents d'une run principale.

Tout le reste est hors scope pour le moment.

---

# 17. Références

## Hammerfest / Motion Twin

**Source originale Hammerfest**
Contient le code MotionTypes/AVM1 original. C'est la référence principale pour comprendre le modèle métier (`GameMode`, `SetManager`, `GameMechanics`, `Adventure`, etc.).

[motion-twin/hammerfest](https://github.com/motion-twin/hammerfest?utm_source=chatgpt.com)

Le README confirme notamment que Hammerfest a été développé pour Flash 7/8, AVM1, en MotionTypes.

## Reverse engineering existant

**Repo du reverse Hammerfest / score**

```text
cmnemoi/hammerfest-re (D:\code\hammerfest-re)
```

Commencer par lire en particulier :

```text
README.md
hammerfest-score-re.md
scripts/graph.py
scripts/resolve.py
scripts/hf_score.py
scripts/capture_reps.py
```

Ce repo contient les résultats du précédent RE Pepper Flash/AVM1 ainsi que les outils permettant de parcourir le heap.

## Obfuscation Eternalfest

**Obfuscateur Eternalfest**

[eternalfest/obf](https://gitlab.com/eternalfest/obf?utm_source=chatgpt.com)

**Project Phoenix — scripts de déobfuscation MTASC/Haxe**

[eternalfest/project-phoenix](https://gitlab.com/eternalfest/project-phoenix?utm_source=chatgpt.com)

Priorité : déterminer si Phoenix permet de récupérer directement la correspondance entre les noms du source et les noms de propriétés présents dans le SWF obfusqué.

Symboles qui nous intéressent en priorité :

```text
GameMode.world
SetManager.currentId
```

## LiveSplit

**LiveSplit principal**

[LiveSplit/LiveSplit](https://github.com/livesplit/livesplit?utm_source=chatgpt.com)

**Documentation / registre officiel des Auto Splitters**

[LiveSplit.AutoSplitters](https://github.com/LiveSplit/LiveSplit.AutoSplitters?utm_source=chatgpt.com)

Cette documentation décrit les différentes formes d'autosplitters et notamment les modules WebAssembly utilisant l'Auto Splitting Runtime. Elle indique également que le runtime sandboxé est cross-platform et que Rust dispose actuellement du meilleur support parmi les langages utilisables avec ASR.

**Registre XML utilisé par LiveSplit pour distribuer les autosplitters**

[LiveSplit.AutoSplitters.xml](https://github.com/LiveSplit/LiveSplit.AutoSplitters/blob/master/LiveSplit.AutoSplitters.xml?utm_source=chatgpt.com)

À regarder seulement plus tard, lorsque l'autosplitter sera suffisamment stable pour être distribué.

## ASR — Auto Splitting Runtime

**Crate Rust `asr`**

[LiveSplit/asr](https://github.com/LiveSplit/asr?utm_source=chatgpt.com)

C'est la bibliothèque principale à utiliser depuis le code Rust de l'autosplitter.

Elle fournit notamment les abstractions nécessaires pour attacher un process, récupérer l'adresse de modules et lire la mémoire du process.

**Documentation API officielle du crate `asr`**

[ASR Rust API documentation](https://livesplit.org/asr/asr/?utm_source=chatgpt.com)

À consulter notamment pour :

```text
Process
Address
PointerSize
signature scanning
watchers
timer
settings
runtime OS / architecture
```

## Template Rust officiel

**LiveSplit Rust Auto Splitter Template**

[LiveSplit/auto-splitter-template](https://github.com/LiveSplit/auto-splitter-template?utm_source=chatgpt.com)

Création d'un nouveau projet :

```sh
cargo generate LiveSplit/auto-splitter-template
```

Pour un autosplitter qui ne nécessite que l'accès aux process, à leur mémoire et au timer, le template recommande le target :

```text
wasm32-unknown-unknown
```

plutôt que WASI.

Compilation :

```sh
cargo b --release
```

Le `.wasm` est ensuite généré dans le répertoire `target/.../release`.

## Debugger ASR

**ASR Debugger**

[LiveSplit/asr-debugger](https://github.com/LiveSplit/asr-debugger?utm_source=chatgpt.com)

À privilégier pendant le développement plutôt que tester chaque changement directement dans LiveSplit.

Fonctionnalités utiles :

* hot reload du `.wasm` ;
* logs de l'autosplitter ;
* affichage des variables ;
* changement rapide des settings ;
* mesures de performance ;
* dump mémoire ;
* debug avec LLDB.

Pour notre travail de RE, les logs + hot reload + inspection mémoire devraient être particulièrement utiles.

---

# 18. Principe directeur pour commencer

Ne pas commencer par écrire tout l'autosplitter.

Résoudre d'abord cette question :

```text
Sous Windows, comment obtenir de façon reproductible :

GameMode.world.currentId
```

Le premier livrable utile peut être simplement :

```text
Attached to EternalTwin
Pepper Flash: ...
GameMode: 0x...
World: 0x...
Current level: 3
```

Si cette lecture fonctionne :

```text
0 → 1 → 2 → 3
```

et survit à un restart / une nouvelle partie sans dépendre d'une adresse hardcodée, alors le cœur difficile du problème est résolu.

Le branchement à LiveSplit vient après.
