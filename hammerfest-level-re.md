# Niveau et chrono Hammerfest, sous Windows

Notes de reverse. Chaque affirmation est marquee **[PROUVE]** (verifie sur le
process vivant, avec la mesure qui le montre) ou **[HYPOTHESE]**.

Cible : EternalTwin sur Windows, `pepflashplayer.dll` win32-x64 **32.0.0.465**,
AVM1 / ActionScript 2, lecture seule.

Le travail precedent (`cmnemoi/hammerfest-re`, realise par Claude) avait resolu
le *score* sous Linux. Ces notes-ci portent sur le niveau et le temps, sous
Windows.

---

## 1. Ce que le jeu traque vraiment

Deux compteurs de temps, tous deux remontes par le jeu en fin de partie
(`mode/Adventure.mt`, `onGameOver`) :

```mt
manager.logAction( "$t=" + Math.round(duration/Data.SECOND) );
manager.history = [ "F="+$version, "T="+gameChrono.get() ];
```

| champ | type | nature |
| --- | --- | --- |
| `GameMode.gameChrono` | `Chrono` | millisecondes, **arrete pendant les pauses** |
| `GameMode.duration` | `Float` | `duration += Timer.tmod` par frame, `Data.SECOND = 32` |

`Chrono` (`class/hammer/Chrono.mt`) ne stocke pas une duree mais deux instants :

```mt
function get() {
    if ( fl_stop )  return haltedTimer;
    else            return Math.floor( frameTimer-gameTimer );
}
```

`stop()` est appele par `GameMode.lock()`, `start()` par `unlock()`. C'est donc
un temps de jeu qui se fige a la pause : exactement ce qu'on veut comme *game
time* pour LiveSplit. **[PROUVE]** -- voir la verification croisee en §5.

Le niveau courant est `GameMode.world.currentId`, ou `world : GameMechanics`
herite de `SetManager` (`levels/SetManager.mt`) :

```mt
function setCurrent(id:int) {
    _previous = current;  _previousId = currentId;
    current = levels[id]; currentId = id;
}
```

---

## 2. Les noms obfusques ne se cherchent pas, ils se lisent

Le SWF distribue est obfusque, mais la correspondance est **publique** :
`eternalfest/game-types`, fichier `src/lib/hf.map.json`, MIT, 2952 entrees. Elle
sert a `eternalfest/project-phoenix`, le decompilateur du jeu. Copiee dans
`vendor/hf.map.json`.

Attention : l'obfuscateur d'Eternalfest (`eternalfest/obf`) n'est **pas** celui
qui a produit ce SWF -- il genere des noms `md5(sel+nom)[0:10]`, purement
hexadecimaux, alors que les noms du SWF Hammerfest ressemblent a `70dik` ou
`{8`. La table, elle, decrit bien le SWF de Motion Twin.

Elle se verifie contre le reverse precedent, qui avait observe ces valeurs en
memoire sans la connaitre : **[PROUVE]**

| clair | obfusque | source |
| --- | --- | --- |
| `realScores` | `70dik` | observe en 2025 dans le tas |
| `fakeScores` | `[t}LJ(` | observe en 2025 dans le tas |
| `gi` | `{8` | observe en 2025, l'identification restait une hypothese |

Les clefs utilisees ici :

| clair | obfusque |
| --- | --- |
| `world` | `]=[]8` |
| `currentId` | `-BBEO` |
| `_previousId` | `(VT Q` |
| `setName` | ` h;+A(` |
| `gameChrono` | `8qkdA` |
| `frameTimer` | `]);5(` |
| `gameTimer` | `{DfiG` |
| `haltedTimer` | `]IEAU` |
| `fl_stop` | `*9gvn` |
| `xml_adventure` | `]R;5E` |

`duration` n'est pas renomme : c'est un identifiant de l'API AS2 standard
(`Sound.duration`), donc protege par la liste `as2.map.json`. Il garde son nom
en clair dans le SWF. **[PROUVE]** -- lu tel quel dans la table GameMode.

Les noms de mondes sont eux aussi obfusques : `addWorld("xml_adventure")` passe
une chaine qui ressemble a un identifiant, donc renommee en `]R;5E`. Chercher
`"xml_adventure"` dans le tas ne donne rien. **[PROUVE]**

---

## 3. Layout memoire : Windows n'est pas Linux

Le layout Linux etait connu. Il ne tient qu'a moitie sous Windows -- mesure, pas
suppose : **[PROUVE]**

```
                        Linux x86-64        Windows x86-64
String   vtable         +0x00               +0x00            identique
         buffer UTF-16  +0x08               +0x08            identique
         longueur       +0x30               +0x30            identique
ScriptObject -> table   +0x30               +0x30            identique
table    vtable         +0x00               +0x00            identique
         capacite       +0x08               +0x08            identique
         entrees        +0x18               +0x48            DIFFERENT
         pas            16 octets           24 octets        DIFFERENT
         entree         (valeur, clef)      (valeur, _, clef) DIFFERENT
```

Autrement dit une entree fait 24 octets et la clef est le *troisieme* qword, pas
le second. Un port qui aurait recopie les offsets Linux aurait lu, pour chaque
propriete, la valeur du champ **suivant** : des valeurs parfaitement plausibles,
et fausses. C'est exactement l'erreur que j'ai faite en premier, et que le
controle semantique a rattrapee.

Vtables observees sur cette session (ASLR : relatives a la base du module, elles
sont stables pour ce binaire) :

```
String        MODULE+0x1756db8
ScriptObject  MODULE+0x1749ed8
table         MODULE+0x174a460
```

Le code ne les code pas en dur : il les derive a l'execution (`scripts/avm1.py`).
Seule la base du module vient de l'OS.

### Encodage des atomes **[PROUVE]**

`atome = (valeur << 3) | tag`, les 3 bits bas etant le type :

| tag | sens | decodage |
| --- | --- | --- |
| 0 | entier **signe** | `atome >> 3`, decalage arithmetique |
| 1 | flottant | pointeur vers un `double` IEEE 8 octets |
| 2 | special | `0x0a` null, `0x12` faux, `0x32` vrai |
| 3 | objet natif / MovieClip | pointeur |
| 5 | String | pointeur vers un objet String |
| 6 | objet | pointeur vers un ScriptObject |

Le signe compte : `portalId` valait `0xfffffffffffffff8`, soit `-1`. Un
decodage non signe en aurait fait 2305843009213693951. **[PROUVE]**

---

## 4. Chaine de resolution

Aucune adresse en dur, aucun chemin de pointeurs statique -- il n'en existe pas,
les objets sont crees a l'execution par un SWF telecharge. L'ancre est une
chaine internee du SWF :

```
process --type=ppapi         le plugin Flash, cree au chargement du SWF
pepflashplayer.dll           ASLR -> base du module
scan "]=[]8" dans le tas     `world`, clef connue par hf.map.json
  -> objet String            le qword module-pointant devant = la vtable
  -> offset de longueur      le qword valant 5
  -> slots citant la chaine  scan des 8 encodages d'atome
  -> pas des entrees         mesure sur les clefs voisines
  -> base de la table        premier qword module-pointant avant les entrees
  -> offset des valeurs      vote (voir ci-dessous)
GameMode["]=[]8"]            -> world : GameMechanics
world[" h;+A("]              -> setName == "]R;5E" = xml_adventure   verification
world["-BBEO"]               -> currentId : le niveau
GameMode["8qkdA"]            -> gameChrono
```

Deux pieges rencontres, tous deux reels :

**Le vote sur l'offset des valeurs ne peut pas se faire sur la seule validite.**
Chaque entree contient un qword inutilise toujours nul, et zero est un atome
entier parfaitement valide : cette colonne obtient donc un score de validite
parfait sans rien contenir. Il faut exiger de la **diversite** dans la colonne.

**`world` ne suffit pas a identifier le GameMode.** Les objets `View` en portent
un aussi, et pointent vers le meme `GameMechanics` -- j'ai observe jusqu'a trois
candidats simultanes, dont un pointant vers `xml_deepnight`. Le discriminant est
structurel : seul le GameMode possede en plus un `gameChrono` contenant un
`frameTimer`. **[PROUVE]**

---

## 5. Verification croisee

Les deux compteurs du jeu sont independants : `gameChrono` compte des
millisecondes reelles, `duration` accumule `Timer.tmod` par frame. Ils doivent
concorder. Releve sur le process vivant :

```
duration   14815.6 cycles / 32 = 463.0 s
gameChrono                       462898 ms = 462.9 s
```

**[PROUVE]** -- accord a 0,1 %, sur deux chemins memoire totalement disjoints.

Le modele `Chrono` se verifie aussi tout seul. Partie en pause :

```
fl_stop      = true            (et fl_pause = true cote GameMode)
frameTimer   = 280135
gameTimer    =  19419          frameTimer - gameTimer = 260716
suspendTimer = 274444          280135 - 274444 = 5691 ms depuis l'arret
haltedTimer  = 255025          255025 + 5691  = 260716   OK
```

`stop()` pose `haltedTimer = get()` et `suspendTimer = frameTimer` : l'identite
se referme exactement. **[PROUVE]**

---

## 6. Les niveaux ne se suivent pas

`currentId` n'avance pas de 1 en 1. **[PROUVE]** -- par la source, et confirme
en jeu par l'absence de split au raccourci du niveau 0.

`mode/Adventure.mt` :

```mt
function nextLevel() {
    super.nextLevel();          // goto(currentId+1)  ->  currentId = 1
    if ( fl_warpStart ) {
        world.currentId = 0;    // affectation directe, hors setCurrent
        unlock();
        world.view.detach();
        forcedGoto(10);         // ->  currentId = 10
    }
}
```

Le raccourci du niveau 0 fait donc `0 -> 10`. Ce n'est pas un cas isole :
`SpecialManager.warpZone(w)` avance de 1 a 3 d'un coup via `forcedGoto`, en
s'arretant avant un boss ou un niveau vide.

Une regle de split en `currentId + 1` rate tous ces passages. Le critere correct
est **tout progres vers l'avant**, `currentId` strictement croissant.

Deux consequences moins visibles :

**Une course de lecture.** Ces trois ecritures ont lieu dans la meme frame de
jeu, et rien ne synchronise une lecture faite depuis un autre process avec la
frame. On peut donc observer `0 -> 1`, puis `1 -> 0`, puis `0 -> 10`, et
produire deux splits au lieu d'un, de maniere non deterministe. D'ou la
confirmation sur deux lectures consecutives avant d'agir.

**`_previousId` n'est pas fiable comme temoin de transition.** Il n'est mis a
jour que par `setCurrent` ; l'affectation `world.currentId = 0` le court-circuite.
Il reste utile pour lever une ambiguite lors du reverse, pas pour valider un
split.

## 7. Savoir qu'un GameMode est mort

Un GameMode abandonne reste lisible longtemps : la table est intacte, le monde
est toujours `xml_adventure`, le niveau et le chrono sont plausibles -- ils sont
simplement ceux d'avant. Aucune verification de coherence ne les distingue d'un
objet vivant. C'est le piege annonce par le reverse du score, et il se voit
directement : le timer met du temps a demarrer, et ne s'arrete pas a la fin
d'une partie.

Deux signaux le resolvent. **[PROUVE]** par la source.

**`fl_gameOver`**, pose par `GameMode.onGameOver()`, est le signal exact de fin
de partie -- c'est dans la surcharge d'`Adventure` que le jeu envoie
`"T="+gameChrono.get()`. Une resolution qui tombe sur un GameMode deja en game
over doit etre rejetee, sans quoi on lit une partie terminee au lieu d'attendre
la suivante.

**`Chrono.frameTimer`** sert de battement de coeur. Dans `GameMode.main()` :

```mt
// Chrono
gameChrono.update();      // inconditionnel, et AVANT le test de pause

// Pause
if ( fl_pause ) { ... }
```

`Chrono.update()` fait `frameTimer = Std.getTimer()`. Ce compteur avance donc a
chaque frame tant que ce GameMode-la est celui que le jeu fait tourner -- y
compris en pause, y compris chrono arrete. Fige, l'objet est mort.

Le seuil doit rester genereux : Flash tourne a une trentaine d'images par
seconde, et Chromium ralentit encore une fenetre en arriere-plan. Un seuil trop
court declencherait des re-resolutions inutiles, chacune coutant un balayage
complet du tas.

## 8. L'ancre stable : GameManager.current

Balayer le tas pour retrouver le GameMode coute une centaine de Mio de lectures.
Le refaire a chaque partie est deja penible ; le refaire en boucle entre deux
parties -- ce que fait tout autosplitter qui attend la suivante -- agite le
process pour rien.

Le jeu offre pourtant l'ancre qu'il faut. `GameManager` est cree une fois au
chargement du SWF et survit aux parties, et il designe le mode en cours
(`GameManager.mt`) : **[PROUVE]**

```mt
var current : Mode;

function transition(prev:Mode,next:Mode) {
    next.init();
    if ( prev==null )  current = next;
    else            {  prev.destroy(); current = next; }
}
```

Reciproquement, tout `Mode` porte `manager : GameManager`. Cette reference
croisee suffit a identifier le GameManager sans connaitre aucun nom qui lui
soit propre : **c'est l'objet dont `current` designe un mode qui, par son champ
`manager`, redesigne cet objet-la**. `current` seul ne vaudrait rien comme
critere -- chaque `SetManager` en a un.

```text
balayage, une fois      -> GameManager          (retenu)
GameManager.current     -> le mode qui tourne   (relu a volonte)
```

L'interet de chercher le GameManager plutot que le GameMode : **il existe des
le chargement du SWF**. Le balayage aboutit donc deja dans les menus, avant
toute partie, et la partie qui demarre ensuite se trouve en suivant un
pointeur. Chercher un GameMode, au contraire, ne peut reussir qu'une fois la
partie lancee -- c'est-a-dire au pire moment, celui ou le delai se voit.

Ce point compte d'autant plus que **le process plugin meurt avec la partie** :
EternalTwin en cree un par partie, donc rien d'appris ne survit d'une partie a
la suivante. Le balayage est inevitable une fois par partie ; tout ce qu'on
peut choisir, c'est de le faire tot.

C'est bien la forme recommandee -- ancre stable, puis resolution par le graphe
d'objets -- et non une adresse finale mise en cache : `current` est relu a chaque fois, et le
GameMode obtenu repasse par toutes les verifications (monde connu, niveau
plausible, `gameChrono` present, pas en game over).

Ce qui peut invalider l'ancre : la table de proprietes d'un objet AVM1 est
reallouee quand elle grandit. Les champs de `GameManager` sont tous poses dans
le constructeur, donc elle ne devrait pas bouger -- mais ce n'est pas garanti,
d'ou la verification de sa vtable avant chaque usage, et le repli sur le
balayage complet si elle ne repond plus.

## 9. Savoir qu'une partie commence, sans lire la memoire

Le signal le plus fiable ne se trouve pas dans le tas : **le process plugin
nait avec la partie et meurt avec elle.** EternalTwin en cree un par partie --
les menus du site sont du HTML, il n'y a pas de Flash avant. Son apparition est
donc binaire, tombe en un tick, et ne coute pas une lecture.

Tout critere pris en memoire, lui, depend d'une resolution prealable, qui
balaye le tas : de l'ordre de la seconde, et **variable**, parce que le cout
depend de l'endroit ou les objets sont tombes. Faire dependre le depart du
chrono de cette resolution, c'est importer cette variance dans le
chronometrage.

Les criteres memoire essayes avant, et pourquoi ils ne valent pas :

| critere | defaut |
| --- | --- |
| `chrono < 5 s` | le chrono court depuis la construction du GameMode, chargement et intro compris : il n'est jamais nul quand le joueur prend la main |
| `currentId == 0` | un joueur rapide quitte le niveau 0 en deux secondes, souvent avant la fin du balayage |
| premiere lecture du process | correct, mais toujours suspendu au delai de resolution |

Le decalage que laisse l'apparition du process -- le temps de chargement du SWF
-- ne se voit pas sur le *game time*, qui vient de `gameChrono` et que
`set_game_time` pose en absolu : la premiere lecture le corrige. Il se verrait
sur un chronometrage en temps reel.

## 10. Ce qui reste ouvert

- **[HYPOTHESE]** Les vtables relatives sont stables pour ce binaire precis.
  Elles sont derivees a l'execution, donc une autre version echouerait
  proprement (`None`) plutot que de renvoyer un faux niveau -- mais ce n'a pas
  ete teste sur une autre version.
- **[HYPOTHESE]** Le layout mesure ici vaut pour tout `pepflashplayer.dll`
  win32-x64 32.0.0.465. Mesure sur une seule machine.
- Les tags 4 et 7 n'ont pas ete identifies. Tag 7 apparait sur des objets dont
  la vtable est `MODULE+0x178a300`, distincte du ScriptObject ordinaire.
- Les dimensions paralleles ne sont pas traitees : on lit `world`, qui suit
  `currentDim`. `currentDim` est expose dans le releve pour pouvoir plus tard
  ignorer les splits hors du monde principal.
