# Mesure du délai de départ

Les réglages utilisateur ont été supprimés. Toutes les compilations utilisent
désormais le rafraîchissement de carte à 100 ms et le temps réel corrigé.
Les instructions A/B ci-dessous décrivent les anciennes expériences ; le
choix A/B n'est plus disponible. Le script de résumé reste compatible avec
leurs journaux. La capture automatique actuelle utilise toujours le mode B.

## Capture automatique de plusieurs parties

La capture conserve le même module et ses caches pour toute la série. Son timer
privé suit les appels de départ et de remise à zéro. Il ne commande pas le chrono
ouvert dans LiveSplit. Le journal est écrit après chaque cycle. Chaque capture
utilise un nouveau fichier, sans écraser les journaux précédents.

```powershell
mise run build-diagnostics
mise run capture-startup --dll D:/Downloads/LiveSplit_1.8.37/Components/x64/asr_capi.dll
```

La capture s'arrête après 25 départs ou 15 minutes. `--runs` et `--seconds`
permettent de changer ces limites. `logs/capture-status.json` indique l'état,
le chemin du journal et le nombre de départs. Son horodatage est actualisé
toutes les cinq secondes environ, hors blocage du runtime. Aucun export manuel
n'est nécessaire.

Ce lecteur supplémentaire ajoute une charge. Les mesures servent d'abord à
identifier les étapes des relances tardives, pas à estimer le P99 en usage normal.

## Résultat des parties et intégration

La série `logs/departs-ab.txt` contient 11 départs par mode. En mode A,
6 départs sont à 0 ms et le maximum est de 603 ms. En mode B, les 11 départs
sont à 0 ms. Ces valeurs décrivent la détection, pas le rendu de l'interface.

Le module normal utilise maintenant le rafraîchissement à 100 ms, sans réglage.
Les traces `HF_DIAG` et `HF_SCAN` sont désactivées dans cette version. Les traces
d'attache émises par le runtime lui-même peuvent rester. Le calcul du temps et
les règles de départ restent identiques.

Les modes A/B restent disponibles dans la compilation de diagnostic. Le module
normal utilise désormais aussi l'horloge WASI.

Après le redémarrage d'EternalTwin à 13:03, la série de 13:06 à 13:07 contient
12 départs : le premier à 449 ms, puis 11 à 0 ms. Le rafraîchissement améliore
donc cette série, mais ne supprime pas tout retard au premier départ. Les traces
ordinaires ne permettent pas d'attribuer ces 449 ms à une étape précise.

## Résultat du test du runtime installé

Le 21 septembre 2026, le processus LiveSplit charge le composant :

```text
D:\Downloads\LiveSplit_1.8.37\Components\x64\asr_capi.dll
SHA256 4e2629f9ed25233b2198768a3bbe5b4d0241dcaf8b128a2fcf743be2c2039dd8
```

`x64/livesplit_core.dll` est aussi chargé, mais ce n'est pas la DLL qui exécute
le module WASM. Les sources dans le cache Cargo ne suffisaient pas à identifier
ce composant. Le test utilise directement `asr_capi.dll`, sans modifier LiveSplit.
L'interface C est celle du [composant ASR](https://github.com/LiveSplit/LiveSplit.AutoSplittingRuntime/blob/master/src/asr-capi/src/runtime.rs).

Le script réserve une page dans son propre processus. Le module de test relève
la carte, puis le script rend la page lisible après un délai connu. Trois voies
observent cette page : lecture directe, carte de l'accès initial, carte d'un
nouvel accès ouvert toutes les 100 ms. L'horloge monotone WASI date les résultats.

| Création après le relevé initial | Retard de la carte normale par rapport à la lecture directe | Retard avec nouvel accès toutes les 100 ms |
| ---: | ---: | ---: |
| 100 ms | 885,1 ms | 2,6 ms |
| 350 ms | 643,9 ms | 77,0 ms |
| 650 ms | 342,7 ms | 101,2 ms |
| 900 ms | 93,7 ms | 68,6 ms |

La carte normale voit la page vers 1 000 ms après le relevé initial dans les
quatre essais. Le cache d'une seconde est donc confirmé dans le binaire utilisé.
Le nouvel accès dispose bien d'une carte indépendante. Ces chiffres concernent
une page de test ; ils ne mesurent pas encore le gain sur le départ d'une partie.

Résultat brut : `logs/runtime-probe.json`. Pour refaire le test avec la DLL
chargée par LiveSplit :

```powershell
mise run probe-runtime
```

Pour tester une DLL précise, sans LiveSplit ouvert :

```powershell
mise run probe-runtime --dll D:/Downloads/LiveSplit_1.8.37/Components/x64/asr_capi.dll
```

Le script utilise un faux timer, des allocations locales et des lectures mémoire.
Il n'envoie aucune commande au chrono de la fenêtre LiveSplit.

## Module A/B prêt à charger

Compilation :

```powershell
mise run build-diagnostics
```

Fichier produit :

```text
target/diagnostics/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
```

Ce module ajoute le réglage **Diagnostic B : rafraichir la carte memoire toutes
les 100 ms**, désactivé par défaut.

- **A : case décochée.** Carte normale du runtime.
- **B : case cochée.** Carte obtenue par un nouvel accès au même PID. L'accès
  principal et les ancres restent conservés. L'accès temporaire est fermé après
  le relevé. En cas d'échec d'ouverture, le module utilise la carte normale et
  écrit `map_refresh_failed`.

Seule cette source de carte change entre A et B. Les temporisations, le filtre
des régions, les validations et les règles de chronométrage restent identiques.
Les deux modes utilisent les mêmes mesures. Le rafraîchissement a lieu lorsque
la boucle de recherche reprend la main : un balayage long peut espacer les
relevés de plus de 100 ms. Ce n'est pas une garantie de délai maximal.

## Série de parties à faire

1. Charger le fichier de diagnostic à la place du module habituel dans LiveSplit
   ou asr-debugger. Garder un seul autosplitter actif pendant la série.
2. Faire cinq départs en mode A, puis cinq en mode B. Attendre que le niveau 0 et
   le chrono apparaissent avant de quitter chaque partie.
3. Refaire cinq départs B, puis cinq A. Cela donne dix départs par mode et réduit
   l'effet de l'ordre des essais. Pour estimer le 95e percentile, faire ensuite
   plusieurs dizaines de départs par mode.
4. Changer le réglage entre les parties, jamais pendant un chargement. Attendre
   la fin de la partie précédente avant de relancer.
5. Exporter le journal dans `logs/departs-ab.txt`. Dans asr-debugger, utiliser le
   bouton **Save** du panneau **Logs**. Séparer les essais de premier lancement
   des relances dans un même processus.

Résumé automatique du journal :

```powershell
mise run summarize-startup logs/departs-ab.txt
```

Le script accepte aussi le journal du module normal : il associe `depart date`
à `partie lancee` et affiche une ligne **Sans A/B**. Ce format ne contient pas
le mode de cache. Fournir un seul export par session : `auto_splitter_logs.txt`
et `logs/departs-ab.txt` peuvent contenir les mêmes départs.

Le tableau donne le nombre de départs automatiques, le nombre de valeurs nulles,
la médiane, le 95e percentile et le maximum du retard reconstruit à la première
lecture. Les résolutions en cours de partie, sans appel de départ, sont exclues.

Pour mesurer l'affichage complet, enregistrer le jeu et LiveSplit dans la même
vidéo. Le journal mesure la détection et l'appel à `start()`, pas le rendu de
l'interface. Une valeur nulle dans le journal ne prouve pas un affichage simultané.

## Lecture des traces

- `HF_DIAG event=mode` : mode A (`fresh=false`) ou B (`fresh=true`).
- `map_refresh` : durée et taille de la carte obtenue par le nouvel accès.
- `stage` : durée d'une étape, avec compteurs cumulés du balayage.
- `HF_SCAN` : bilan complet, même si la recherche sort tôt ou utilise le repli.
  `requested_bytes` compte les octets demandés ; `read_bytes` ne compte que les
  blocs lus avec succès. `validation_*` compte les lectures auxiliaires AVM1.
- `read` : apparition ou perte d'un état lisible, ou changement de verrouillage.
- `origin` : retard reconstruit et indication de départ automatique.
- `start_called` : instant après l'appel à `timer::start()`.
- `loop_gap` : intervalle de plus de 50 ms entre deux passages de la boucle
  principale. Cet intervalle peut inclure un balayage avec plusieurs pauses.

Les dates `t_us` sont monotones, en microsecondes. Elles ne sont pas des heures
civiles. Les durées de balayage incluent les attentes entre les cycles. Le module
normal, compilé sans la fonction `diagnostics`, n'émet pas ces mesures.

Le module normal reste dans `target/wasm32-unknown-unknown/release/`. La commande
de diagnostic construit un fichier séparé pour éviter son rechargement automatique.

## Suite selon les résultats

Si B réduit les grands retards, conserver une solution de rafraîchissement et
mesurer son coût. Si des retards restent, les traces permettront de distinguer
les temporisations, les régions ignorées et les validations tardives. Corriger
ensuite un mécanisme à la fois. La série de parties ci-dessus a permis d'activer
le rafraîchissement dans le module normal. Elle ne garantit pas un délai nul
sur chaque machine.
