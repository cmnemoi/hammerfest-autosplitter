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
| `GameMode.gameChrono` | `Chrono` | millisecondes, depuis la construction du `GameMode` |
| `GameMode.duration` | `Float` | `duration += Timer.tmod` par frame, `Data.SECOND = 32`, depuis l'apparition du niveau 0 |

Les deux s'arretent aux memes moments -- pause et transitions de niveau --
puisque `GameMode.lock()` arrete `gameChrono` et fait sortir `main()` avant
l'incrementation de `duration`. Ils ne different que par leur **origine**, et
c'est justement ce qui rend `duration` utile : voir §9. **[PROUVE]** : 26,2 s
de pause donnent `duration` +0,0 cycle et `gameChrono` +0 ms.

`Chrono` (`class/hammer/Chrono.mt`) ne stocke pas une duree mais deux instants :

```mt
function get() {
    if ( fl_stop )  return haltedTimer;
    else            return Math.floor( frameTimer-gameTimer );
}
```

`stop()` est appele par `GameMode.lock()`, `start()` par `unlock()`. C'est donc
un temps de jeu qui se fige a la pause -- ce que le jeu affiche, mais pas un
temps reel. Le chrono de la run est construit autrement, voir §9.

Un detail du constructeur compte : `suspendTimer` y est laisse a `null`, donc
le premier `start()` ne decale pas `gameTimer`. `gameChrono` compte ainsi
depuis la construction du `Chrono`, chargement du niveau 0 compris, et vaut
deja 0,55 s quand le joueur voit le niveau. **[PROUVE]** -- 535, 539, 547 et
562 ms sur quatre parties.

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

Ce point compte d'autant plus que **les objets AVM1 meurent avec la partie** :
le SWF recree son GameManager et ses chaines internees a chaque lancement, donc
aucune adresse du tas ne survit d'une partie a la suivante. Le process plugin,
lui, peut en porter plusieurs -- quatre parties consecutives observees dans un
meme process. Le balayage est inevitable une fois par partie ; tout ce qu'on
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

## 9. Dater le depart de la run, sans gagner de course

La regle de course fixe le depart : *the timer begins when the loading text
disappears and fades in to level 0*. Cote memoire, cet instant est celui ou
`GameMode.fl_lock` retombe.

```mt
GameMechanics.onViewReady()      la vue du niveau est attachee
    game.onLevelReady()
        unlock()                 fl_lock = false
            gameChrono.start()
```

La vue est attachee et le mode est deverrouille **dans la meme image**, donc
l'ecran noir se termine a `fl_lock = false` a une image pres, soit 31 ms.

### La course ne peut pas se gagner **[PROUVE]**

Le premier reflexe est de poser l'ancre avant le depart, pour voir la
transition en direct. C'est impossible, et pas par manque d'optimisation.

| | run 1 | run 2 | run 3 | run 4 |
| --- | --- | --- | --- | --- |
| ancre posee, apres le depart | +0,374 s | +0,601 s | +0,553 s | +0,657 s |
| `gameChrono` au deverrouillage | 535 ms | 539 ms | 562 ms | 547 ms |

Mesures de `scripts/hf_trace.py`. Ce que montrent les balayages successifs :

1. **Rien a trouver avant.** Jusqu'a la derniere seconde, le tas ne contient
   aucune chaine `fVersion` -- ni celle du `GameManager`, ni celle du `Loader`.
   Les objets AVM1 du SWF naissent tous en rafale, a la fin de
   l'initialisation. Il n'existe donc pas de fenetre anterieure.
2. **La fenetre utile vaut 0,55 s**, entre la construction du `GameMode` et le
   deverrouillage.
3. **Un balayage complet en coute 0,5 s** sur les 80 Mio du tas. C'est le prix
   de la copie hors process, pas celui de la comparaison : le rendre
   instantane n'est pas au programme.

La course se joue donc a quelques dizaines de millisecondes, et elle se perd.

### Il ne faut pas la gagner : le jeu porte l'instant du depart **[PROUVE]**

`GameMode.main()` sort sur `fl_lock` **avant** d'incrementer `duration` :

```mt
gameChrono.update();          // frameTimer = Std.getTimer()
if ( fl_pause ) { ... }
if ( fl_lock ) return;        // <- ecran noir, transitions, pause
...
duration += Timer.tmod;       // <- ne court que depuis le depart officiel
```

`duration` vaut donc exactement zero pendant tout l'ecran noir, puis mesure le
temps pendant lequel le jeu a tourne. Une seule formule couvre les deux cas :

```text
origine = frameTimer - duration          a la premiere lecture deverrouillee
temps reel = frameTimer - origine
```

Arrive a temps, `duration` est nulle et l'origine est `frameTimer`. Arrive en
retard -- le cas courant -- `duration` dit de combien.

Les deux compteurs qui rendent cela possible :

| compteur | ce qu'il mesure | mesure |
| --- | --- | --- |
| `frameTimer` | `Std.getTimer()`, millisecondes reelles depuis le lancement du plugin. `Chrono.update()` tourne avant le test de pause et avant le `return` sur `fl_lock`, donc il n'est jamais arrete | +13 843 ms pour 13,9 s de pause |
| `duration` | temps pendant lequel le jeu a tourne, en cycles de `Data.SECOND` = 32 | 67,20 s pour 67,12 s reelles, soit 0,1 % |

Verification de bout en bout : origine posee a la premiere lecture, puis
comparaison avec une horloge exterieure une minute plus tard. **Ecart de
6 ms sur 60,9 s.** **[PROUVE]**

Le balayage du tas sort donc du chemin critique. Sa duree ne fait plus que
retarder l'*affichage* du chrono, elle n'entre plus dans le chronometrage.

### Ce qui reste en retard

Le *real time* de LiveSplit, lui, part de l'appel a `timer_start()` et l'API
ASR ne sait pas reculer un chrono deja demarre -- elle n'expose que `start`,
`split`, `reset`, `set_game_time` et `pause_game_time`. Il accuse donc le
retard du balayage, 0,4 a 0,7 s selon les mesures ci-dessus.

C'est pourquoi le temps reel exact est pose dans le canal *game time*, qui
accepte une valeur absolue. `gameChrono` passe en variable a cote du chrono :
c'est le chiffre que le jeu affiche lui-meme en fin de partie
(`"T="+gameChrono.get()`), mais il exclut les pauses et les transitions de
niveau, donc il ne peut pas servir de temps reel.

### Pourquoi pas les autres criteres

| critere | defaut |
| --- | --- |
| apparition du process plugin | de 2,8 a 4,5 s avant le depart selon le temps de chargement du SWF, donc inutilisable |
| `chrono < 5 s` | `gameChrono` court depuis la construction du `GameMode` : il vaut deja 0,55 s quand le niveau apparait |
| `currentId == 0` | un joueur rapide quitte le niveau 0 avant la fin du balayage |
| fin du fondu du `Loader` | le `Loader` du SWF d'origine n'existe pas dans EternalTwin : aucune table portant `fVersion` ne porte `gameInst` |

## 10. Pourquoi il n'y a pas de chemin de pointeurs statique

Un autosplitter ordinaire suit `module + offset -> +offset -> +offset`. Ici,
non -- et ce n'est pas faute d'avoir cherche. La raison est structurelle.

**Ce qui a ete mesure**, dans l'ordre :

1. Tout ce qui est AVM1 est recree a chaque lancement de partie, dans le meme
   process plugin : GameMode, GameManager, objet de classe portant la statique
   `SELF`, et jusqu'aux chaines internees du pool de constantes du SWF. Aucune
   adresse du tas ne peut servir d'ancre d'une partie a l'autre. **[PROUVE]**
   (`anchors.py`, deux relevés consecutifs)

2. Les adresses qui pointent vers le film courant sont elles aussi recreees :
   le lecteur ne garde pas de pointeur a adresse fixe vers le film a ce
   niveau-la. **[PROUVE]** (`stable_slots.py`)

3. Sur 550 pointeurs des donnees du module, **109 seulement sont referencees
   par du code**, les 441 autres par aucune instruction en adressage relatif --
   ce sont des tableaux et des buckets d'allocateur. **[PROUVE]** (`xrefs.py`,
   desassemblage des 22 Mio de `.text`)

4. Les globals que consultent les **methodes des objets AVM1** -- celles listees
   dans les vtables String, ScriptObject, table et MovieClip -- se reduisent a
   trois choses : **[PROUVE]** (`vtable_globals.py`, 141 fonctions)

   | global | role |
   | --- | --- |
   | `module+0x1e02e90` | `__security_cookie`, confirme par le `LoadConfig` du PE |
   | `module+0x1e583a8/b0/c0` | allocateurs MMgc, ils pointent vers les bases des regions du tas |
   | `module+0x1f2b058` | valeur non pointeur, bruit de desassemblage |

**Conclusion.** Le contexte de l'interpreteur AVM1 n'est pas un global. Il est
transmis en parametre, ou atteint depuis l'objet lui-meme -- ce qui est la
maniere propre d'ecrire une VM, et ce qui explique qu'aucune racine statique
n'apparaisse. Les recherches de chaine echouaient donc pour une bonne raison,
pas par manque de profondeur ou de filtres.

Ce qui resterait a tenter, dans un autre cadre : identifier le contexte comme
un champ de l'objet AVM1 lui-meme, puis chercher qui detient ce contexte cote
lecteur -- vraisemblablement l'instance PPAPI, enregistree dans une structure
indexee, c'est-a-dire precisement le genre de tableau que le point 3 a ecarte.

## 11. Ce qui reste ouvert

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
