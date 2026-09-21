# Les scripts

Deux familles, qui ne se lisent pas de la meme facon.

Les **outils** servent encore : lire une partie sans passer par LiveSplit,
mesurer un demarrage, verifier une hypothese avant de toucher au Rust.

Les **releves** ont servi une fois. Chacun a repondu a une question, et cette
reponse est ecrite ailleurs -- ils ne sont gardes que pour qu'on puisse la
refaire. Leur code n'a pas a etre beau, et il ne le sera pas.

Aucun n'a de dependance hors de `scripts/`, sauf mention contraire. Les
releves qui desassemblent la DLL demandent le groupe `analysis` de
`pyproject.toml` (capstone, pefile, numpy).

---

## Bibliotheques

Elles ne s'executent pas seules. Tout le reste s'appuie dessus.

| fichier | role |
| --- | --- |
| `winmem.py` | lecture seule d'un process Windows : `ReadProcessMemory` et `VirtualQueryEx` par ctypes, sans dependance. C'est l'equivalent Windows de `memlib.py` / `heap.py` du depot `hammerfest-re`, qui lisaient `/proc/<pid>/mem` |
| `avm1.py` | le modele objet AVM1 -- atomes, chaines, tables de proprietes. Le layout est **derive a l'execution**, jamais suppose : il differe entre Linux et Windows |
| `hfmap.py` | noms du source Hammerfest vers noms obfusques du SWF, depuis `vendor/hf.map.json` |

## Outils

| commande | fichier | ce qu'il fait |
| --- | --- | --- |
| `mise run state` | `hf_state.py` | un releve : niveau, chrono, monde |
| `mise run watch` | `hf_state.py --watch` | suit les changements de niveau en direct |
| `mise run dump` | `hf_state.py --dump` | affiche les tables `GameMode`, `world` et `Chrono`, noms en clair |
| `mise run trace` | `hf_trace.py` | horodate un demarrage de partie, du process au depart officiel, et ecrit un CSV. C'est lui qui a etabli que le depart se reconstruit apres coup |
| `mise run capture-startup` | `capture_startup.py` | enchaine plusieurs parties et releve leur delai d'affichage, sans export manuel |
| `mise run summarize-startup` | `summarize_startup.py` | resume les departs d'un journal exporte |
| `mise run probe-runtime` | `runtime_probe.py` | mesure le cache de la DLL ASR que LiveSplit charge reellement. `cache_probe.rs` est le programme de test qu'il compile |

Les trois derniers demandent la compilation `diagnostics` -- voir
[diagnostic-affichage.md](../diagnostic-affichage.md).

## Releves

Ils portent tous sur la meme question, posee en 2026-09 : **existe-t-il un
chemin de pointeurs statique vers les objets du jeu ?** La reponse est non, et
elle est demontree en [section 10 de
`hammerfest-level-re.md`](../hammerfest-level-re.md) -- le contexte de
l'interpreteur AVM1 n'est pas un global, il est transmis en parametre.

Les quatre premiers ont produit les mesures citees dans cette section. Les
trois derniers sont des tentatives qui n'ont rien donne : la section conclut
sans eux, et rien ne s'y refere. Ils sont gardes parce qu'une piste epuisee
merite d'etre retrouvable -- pas parce qu'elle a servi.

| fichier | question | ou la reponse est ecrite |
| --- | --- | --- |
| `anchors.py` | quelles adresses survivent au relancement d'une partie ? | section 10, point 1 : aucune |
| `stable_slots.py` | quels emplacements le lecteur reutilise-t-il d'une partie a l'autre ? | section 10, point 2 : aucun ne pointe vers le film courant |
| `xrefs.py` | combien de pointeurs des donnees du module sont cites par du code ? | section 10, point 3 : 109 sur 550 |
| `vtable_globals.py` | quels globals les methodes des objets AVM1 consultent-elles ? | section 10, point 4 : trois, dont deux allocateurs et un cookie de securite |
| `ptrscan.py` | bibliotheque des deux suivants : cherche une chaine de pointeurs vers un objet | -- |
| `findchain.py` | existe-t-il une chaine courte des donnees du module au film courant ? | non cite : la section 10 conclut sans lui |
| `checkchain.py` | une chaine candidate mene-t-elle vraiment au film courant ? | non cite : rien ne s'y refere, et aucun autre script ne l'importe |

Ils dependent des bibliotheques ci-dessus, donc ils ne se deplacent pas sans
elles. C'est la raison pour laquelle ils restent ici plutot que de rejoindre
`hammerfest-re` : les emporter obligerait a y copier `winmem`, `avm1`, `hfmap`
et `hf_state`, qui vivent ici et continuent de changer.

`hammerfest-re` porte un autre travail : la lecture du **score** sous Linux,
par `/proc/<pid>/mem`. Meme jeu, autre plateforme, autre question.
