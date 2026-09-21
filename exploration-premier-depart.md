# Retards rares : observations et limites

La priorité est de comprendre les retards extrêmes, pas de réduire quelques
millisecondes sur les départs ordinaires.

## Ce qui a été observé

La série du module normal après redémarrage contient 12 départs : 449 ms pour
le premier observé, puis 11 à 0 ms. L'export ne contient pas le début du module ;
il ne permet donc pas de prouver à lui seul l'état initial de ses caches.

La capture `logs/premier-depart.txt` démarre avec un module neuf, avant le
lancement d'EternalTwin. Elle s'arrête au premier départ automatique : les
parties suivantes ne sont pas dans cette capture. Retard reconstruit : 135 ms.

| Étape juste avant ce départ | Durée |
| --- | ---: |
| Recherche infructueuse complète | 787,6 ms |
| Recherche par contenu de `fVersion` | 297,1 ms |
| Recherche par contenu de `world` | 305,5 ms |
| Tentative suivante, réussie | 45,1 ms |

La tentative infructueuse lit environ 272,4 Mio en quatre passes. Les deux
recherches par contenu occupent environ 76,5 % de ses 787,6 ms. Le départ
reconstruit tombe pendant cette tentative. Ces durées incluent les attentes du
runtime ; elles ne mesurent pas uniquement le temps CPU ou la copie mémoire.

La capture utilise la DLL ASR installée avec un faux timer. Ce lecteur ajoute
une charge de lecture pendant l'essai. Un seul essai ne donne pas une distribution
représentative des retards visuels dans LiveSplit.

## Condition de la recherche supplémentaire

`Binary::proven()` reste faux jusqu'à l'apprentissage du layout. Tant qu'il est
faux, l'absence d'une chaîne à la vtable attendue déclenche une recherche par
contenu. Le layout est normalement appris à la première résolution réussie du
manager ou du jeu. Il survit ensuite au processus Flash, mais pas au rechargement
du module WASM.

Il s'agit donc d'une condition liée à la première résolution du module. Ce n'est
pas automatiquement le premier départ après chaque redémarrage d'EternalTwin.
Le travail supplémentaire est conditionnel ; le retard qu'il provoque dépend
du moment où les objets du jeu apparaissent pendant la recherche.

Cela explique un mécanisme de retard au démarrage. Cela ne prouve pas que tous
les retards extrêmes lui sont dus : le filtre des régions, les lectures et les
validations restent aussi présents après l'apprentissage.

## Correction d'une hypothèse précédente

`Policy::no_game()` demande `drop_resolution` même quand aucune résolution
n'était acquise. `run()` remet alors `cooldown` à zéro et `backoff` à 20.
La capture montre des annonces `retry_wait ticks=20` immédiatement suivies
d'une recherche avec `cooldown=0`.

Les délais théoriques calculés à partir des temporisations 20/40/60 ne décrivent
donc pas le comportement actuel. Le filtre d'une passe complète sur huit ne
signifie pas ici une attente systématique de quatre secondes. Cette interaction
reste inchangée pendant l'expérience afin de ne modifier qu'un facteur.

## Variante expérimentale préparée

`mise run build-cold-start` produit une variante dans `target/cold-start/`.
Elle reconnaît le binaire Flash mesuré par son en-tête PE et quatre adresses
relatives de méthodes, avant de chercher les objets du jeu. Si la reconnaissance
réussit, elle évite le repli par contenu. Les validations des objets restent
actives. Si elle échoue, le chemin habituel reste utilisé.

Cette variante n'est pas activée dans le module normal. Son effet sur les
retards extrêmes n'est pas encore mesuré.

## Suivi des prochains cas extrêmes

Le module normal écrit désormais une ligne `HF_START` par départ automatique :
retard reconstruit, premier départ depuis le chargement du module, premier départ
dans ce processus Flash. Ces indicateurs décrivent les départs observés, pas
l'instant exact d'apprentissage du layout.

Le script de résumé classe ces contextes et compte les retards supérieurs à
100 et 500 ms. Ces seuils servent au diagnostic ; ce ne sont pas des garanties.
Les anciens journaux restent lisibles, avec un contexte déclaré inconnu.

Ne pas annoncer un P99 fiable sur 12 départs. Avec moins de 100 départs par
contexte, le P99 empirique par rang supérieur est simplement le maximum observé.
Même cent départs documentent peu les événements qui n'arrivent qu'une fois
sur cent. Si un gros retard apparaît en relance, conserver le journal : ce sera
un cas distinct à examiner, même si les premiers départs sont acceptés comme
exception connue.

## Relance à 581 ms et pauses de recherche (21 septembre 2026)

La capture `logs/relances-20260921-133017-233701.txt` contient 25 départs :
24 à zéro et une relance à 581 ms. Le layout était déjà appris (`proven=true`).
Ce retard ne peut donc pas être expliqué seulement par le premier lancement.

Une recherche infructueuse dure 1 118 086 microsecondes. Elle parcourt une carte
de 651 régions, lit 71 870 016 octets, effectue 2 158 appels de lecture et rend
la main 136 fois. Le départ reconstruit se situe pendant cette recherche.
Les pauses toutes les huit portions lues pénalisent les nombreuses petites
régions. À 120 Hz, 136 périodes représentent environ 1,13 seconde : cet ordre
de grandeur correspond à la durée observée. La trace ne mesure toutefois pas
séparément le temps passé en lecture et le temps passé à attendre.

`mise run build-scan-budget` produit une variante avec diagnostics dans
`target/scan-budget/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm`.
Elle rend la main après au moins 8 Mio demandés ou 128 appels de lecture,
au prochain contrôle dans la boucle. Les échecs comptent dans ces seuils.
Le budget se cumule entre les phases de recherche et repart à zéro après
chaque pause. Ces seuils ne constituent pas une limite stricte de durée :
le traitement des données et les validations ont aussi un coût.

Cette variante conserve le rafraîchissement de carte à 100 ms. Elle n'active
pas la reconnaissance expérimentale du binaire Flash. Le module normal garde
son comportement de pause précédent. L'effet sur les retards reste à mesurer.
La capture externe ajoute une charge ; son retard reconstruit ne mesure pas
directement l'instant d'affichage dans LiveSplit.

### Résultat de la variante à budget de lecture

La capture `logs/relances-20260921-134114-583790.txt` est terminée :
25 départs, 24 à zéro, un à 374 ms. Le retard concerne le premier départ du
module et du processus ; les 24 relances sont toutes à zéro.

Lors du premier départ, une recherche infructueuse dure 998 206 microsecondes,
avec `proven=false`, environ 419 Mio lus et 54 pauses. Elle comprend encore
les recherches par contenu et par références. Le départ reconstruit se situe
pendant cette passe ; la suivante trouve le jeu en 36 057 microsecondes.
Le budget de lecture ne supprime donc pas le coût de la recherche initiale.

Le résultat des relances est encourageant, mais les deux séries ne rejouent
pas les mêmes états mémoire. Elles ne prouvent ni un gain causal de taille
précise, ni l'absence de futurs retards en relance, ni un P99 fiable.
La variante reste séparée du module normal. Pour mesurer l'affichage réel,
il faut ensuite charger cette variante dans LiveSplit et observer son timer.
