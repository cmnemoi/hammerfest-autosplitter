# Analyse du délai d'affichage dans LiveSplit

Analyse du 21 septembre 2026. Objet : le délai entre le départ dans le jeu et
le premier affichage du chrono. Le calcul du temps reste hors de cette analyse.

**Mise à jour après expérimentation :** le cache d'une seconde est confirmé
dans `Components/x64/asr_capi.dll`, réellement chargé par LiveSplit. Un accès
temporaire au même PID fournit une carte indépendante. Les mesures et le
protocole A/B sont dans [diagnostic-affichage.md](diagnostic-affichage.md).
Les réserves ci-dessous décrivent l'état de l'analyse avant cette expérience.

Sources examinées : `src/`, `core/src/policy.rs`, les scripts de recherche,
`recap-temps-reel.md`, `hammerfest-level-re.md`, le journal local et les sources
ASR / livesplit-core présentes dans le cache Cargo.

Il s'agit d'une analyse du code. Aucun nouveau départ de partie n'a été mesuré.
Le journal local ne donne pas la version du module pour chaque série de mesures.
Ses résultats ne sont donc pas attribués sans réserve au code actuel.

## 1. Première piste : un cache d'une seconde dans le runtime

Dans la copie locale de livesplit-core, révision `377f598`, le fichier
`crates/livesplit-auto-splitting/src/process/mod.rs` contient :

```rust
fn refresh_memory_ranges(&mut self) -> Result<(), ModuleError> {
    let now = Instant::now();
    if now >= self.next_memory_range_check {
        // Acquisition de la carte mémoire du processus.
        // ...
        self.next_memory_range_check = now + Duration::from_secs(1);
    }
    Ok(())
}
```

`get_memory_range_count()` appelle cette fonction. Les requêtes de module
utilisent aussi ce cache. La liste des processus a un autre cache d'une seconde
dans `runtime/mod.rs`, méthode `ProcessList::refresh()`.

Conséquence pour `src/lib.rs:176` : appeler `heap_size()` à chaque cycle ne
garantit pas une carte récente. Si une région devient accessible juste après
le relevé, le module peut ne pas la voir pendant presque une seconde. Le seuil
de croissance de 4 Mio ne supprime pas cette attente.

Cette durée correspond à l'ordre de grandeur du problème. Cela ne prouve pas
encore sa contribution dans le LiveSplit installé : il faut identifier la
version de son runtime. Le cache Cargo n'est pas une preuve de cette version.

Solutions à comparer :

- Modifier le runtime hôte pour réduire le délai du cache pendant la recherche,
  ou ajouter une API de rafraîchissement explicite. Modifier seulement la
  dépendance Rust `asr` du module ne change pas ce cache.
- Pour une expérience limitée, ouvrir un second accès au même PID, relever sa
  carte mémoire, puis fermer cet accès. Dans le code hôte examiné, un nouvel
  accès possède un cache vide. Vérifier ce comportement sur l'hôte installé.
  Ne pas faire cette opération à chaque cycle : elle ajoute des ouvertures et
  des messages dans le journal. Conserver la résolution et l'état de partie.
- Pour une solution autonome Windows, utiliser un lecteur natif qui demande
  directement la carte mémoire et transmet l'état à LiveSplit. Cette option
  demande davantage de développement et un protocole de communication.

## 2. Le filtre des régions peut retarder une découverte valide

Dans `src/hammerfest.rs:548`, la recherche compare les couples `(début, fin)`.
En l'absence de changement, elle sort avant `scan_for_manager()`, sauf une fois
sur huit. Cette sortie empêche aussi la recherche des tables lorsque la chaîne
`fVersion` est déjà connue mais que le manager n'était pas encore prêt.

La forme d'une région ne décrit pas son contenu. Un objet peut être créé dans
une page déjà engagée sans changer les bornes de cette région.

Scénario possible avec le code actuel :

1. Une région est lue avant la création de l'objet recherché.
2. L'objet apparaît dans cette région, sans changement des bornes.
3. Les recherches suivantes ne lisent pas cette région.
4. Une passe complète finit par trouver l'objet.

Les temporisations de `src/lib.rs:47` sont de 20, 40, puis 60 cycles. ASR indique
120 cycles/s par défaut, confirmé par les sources locales. Le dernier délai
vaut donc environ 500 ms à cette cadence, hors travail et retard du runtime.
Huit tentatives espacées de 60 cycles représentent environ quatre secondes.
Les commentaires qui supposent 60 cycles/s ne fixent pas la cadence réelle.

Solution : utiliser les régions nouvelles comme priorité de recherche. Garder
aussi une recherche circulaire des régions existantes, avec un budget par
cycle. Une chaîne déjà connue doit permettre de rechercher ses tables sans
attendre une modification de la carte mémoire. Relire brièvement les pages
récentes pendant le chargement, même si leurs bornes ne changent plus.

Réduire `FULL_SWEEP` peut masquer le défaut au prix de lectures supplémentaires.
Ne lire que l'extension des régions conserve le même défaut de découverte.

## 3. Le manager trouvé trop tôt est jeté

`scan_for_manager()`, dans `src/hammerfest.rs:670`, exige la chaîne complète :

```text
GameManager.current -> Mode.manager -> GameManager
```

Si la table existe mais que `current` ne désigne pas encore un mode exploitable,
la recherche rejette cette table et continue à parcourir le tas. Une ressource
utile pour le cycle suivant vient pourtant d'être trouvée.

Solution : conserver une petite liste de managers candidats, puis vérifier
`current` à chaque cycle. Le candidat ne doit autoriser aucun départ tant que
les validations actuelles ne sont pas satisfaites. Conserver aussi les
candidats GameMode encore incomplets lorsque leurs propriétés apparaissent
progressivement. Réexaminer ces adresses entre les blocs du balayage.

Le script `scripts/hf_trace.py:191` accepte déjà une dérivation du layout sans
`current`, par vote sur les propriétés. Le module Rust n'a pas cette voie.
Cette différence doit être prise en compte dans toute comparaison Python/Rust.
Le vote seul ne suffit pas à autoriser le départ.

Le gain dépend de la durée réelle entre l'apparition du candidat et sa
validation. Cette durée reste à mesurer.

## 4. Les coûts du balayage sont mal mesurés

`Cost::read()` augmente un nombre de blocs. `Cost::mib()` le multiplie par un
Mio. Pourtant, les fonctions de balayage demandent `n` octets, avec `n` parfois
très inférieur à un Mio. Le compteur augmente avant de connaître le résultat
de la lecture.

Exemple : seize lectures de 64 Kio donnent un Mio demandé, mais le journal
annonce seize Mio. Une lecture échouée compte aussi comme un Mio lu.

Les petites lectures de validation ne sont pas comptées. Le message
`rien trouve` est émis avant la recherche de repli par `world` : il ne couvre
pas le coût total de la tentative. Certaines sorties par `?` n'émettent aucun
bilan final.

Les affirmations « 700 Mio/s », « lecture dominante » et « neuf à douze
relectures du tas » ne sont donc pas établies par ces compteurs. Cela ne
signifie pas que le balayage est rapide ; son coût doit être mesuré autrement.

Solution : compter séparément les octets demandés, les octets lus avec succès,
les appels, les échecs, les candidats et les lectures de validation. Émettre
un bilan de tentative après toutes ses étapes, avec la cause de sortie.
Mesurer aussi la durée de chaque étape et le temps entre deux cycles.

Le module actuel cible `wasm32-unknown-unknown` et n'a pas d'horloge monotone
exposée par son API utilisée. La copie locale du runtime accepte WASI : une
variante de mesure avec WASI et `Instant` est une option, sous réserve de
compatibilité avec le LiveSplit installé. Une instrumentation de l'hôte est
une autre option. L'absence d'horloge n'est pas une limite générale de WASM.

## 5. Rendre la recherche interruptible par une découverte

Le code fait déjà `next_tick().await` pendant les balayages. Mais la boucle
de `run()` attend toujours la fin de `resolve().await`. Rendre la main au
runtime ne fait pas exécuter le reste de cette boucle.

Solution : conserver un curseur de recherche et traiter un budget de travail
par cycle. Entre deux budgets, vérifier les candidats déjà trouvés. Dès qu'un
candidat permet une lecture valide, abandonner les recherches inutiles et
passer à la surveillance du départ.

Le budget doit inclure les validations distantes, pas seulement les blocs de
mémoire : une validation peut lire beaucoup de propriétés une par une.
Lire une table par blocs puis chercher les clés dans le tampon local peut
réduire ce coût. Les indices déjà calculés doivent rester disponibles.

Cette organisation ne suffit pas si aucun candidat utile n'est encore connu.
Elle doit accompagner les corrections du cache et du filtre des régions.

## 6. La recherche d'une ancre native reste ouverte

La section 10 de `hammerfest-level-re.md` présente l'absence de chemin statique
comme une conclusion structurelle. Les expériences décrites ne prouvent que
l'échec des chemins explorés, avec leurs profondeurs et leurs filtres.

Un tableau global peut contenir un pointeur vers un objet recréé à chaque
partie. Le changement d'adresse de cet objet ne rend pas le chemin inutilisable.
L'absence de référence directe depuis les méthodes virtuelles ne permet pas
d'écarter un registre géré par le lecteur PPAPI.

Deux recherches ciblées restent utiles :

- Retrouver le registre des instances PPAPI et suivre l'instance active jusqu'au
  contexte du film. Une telle racine permettrait de résoudre chaque nouvelle
  partie avec quelques lectures.
- Exploiter les structures d'allocateur déjà repérées pour cibler les pages
  AVM1 et les objets String ou tables. Cela pourrait réduire le domaine du
  balayage même sans chemin direct vers GameManager.

Ce sont des hypothèses de recherche, dépendantes du binaire Flash. Aucun de ces
deux chemins n'est validé dans cette analyse.

## 7. Autre défaut à corriger : rejet définitif d'un processus

Dans `attach_plugin()`, `src/hammerfest.rs:1062`, un PID sans module Flash au
premier examen reste dans `rejected` jusqu'à sa disparition. Le module peut
être chargé plus tard. Le cache de la carte mémoire peut aussi masquer sa
présence au premier examen.

Il faut permettre un nouvel examen des PID rejetés après un délai. Ce défaut
peut empêcher toute attache ; il n'explique pas à lui seul les départs tardifs
une fois le bon processus attaché.

## 8. Ordre de travail proposé

1. Identifier le runtime réellement chargé par LiveSplit. Corriger les compteurs
   et dater les étapes : processus vu, carte obtenue, chaîne trouvée, manager
   candidat, manager validé, première lecture verrouillée, départ demandé.
2. Comparer le cache normal avec une carte rafraîchie plus souvent. Garder le
   même balayage pour isoler l'effet de ce changement.
3. Supprimer l'exclusion durable des régions inchangées et conserver les
   candidats incomplets. Corriger le rejet définitif des PID.
4. Répartir les recherches et les validations entre les cycles. Mesurer ensuite
   l'intérêt de regrouper les lectures et de chercher plusieurs clés par passe.
5. Si la dispersion reste trop grande, explorer l'ancre PPAPI ou l'allocateur.

Pour chaque variante, mesurer plusieurs dizaines de départs : premier
lancement, relances dans le même processus, puis redémarrages de l'application.
Comparer médiane, 95e percentile et maximum. Le log `depart date` ne mesure pas
le rafraîchissement de l'interface LiveSplit. Une capture simultanée du jeu et
de LiveSplit est nécessaire pour mesurer le délai visuel complet. Un log à
0 ms ne prouve pas que les deux affichages sont apparus en même temps.

## Références externes

- [Cadence ASR](https://livesplit.org/asr/asr/fn.set_tick_rate.html) : 120 cycles/s
  par défaut.
- [VirtualQueryEx](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualqueryex) :
  les régions regroupent des pages selon leurs attributs mémoire. Elles ne
  décrivent pas la création des objets dans ces pages.

Les constats sur les caches du runtime viennent de la copie locale
`C:/Users/Charles-Meldhine/.cargo/git/checkouts/livesplit-core-4e7b0b6a2b35495e/377f598/`.
Ils restent à relier à la version du runtime chargé dans l'application.
