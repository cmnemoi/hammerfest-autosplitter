# vendor/hf.map.json

Table de correspondance entre les identifiants du source Hammerfest et les noms
obfusques presents dans le SWF distribue.

- Origine : <https://gitlab.com/eternalfest/game-types>, `src/lib/hf.map.json`
- Commit  : `1991f15a2df6ed36a097304ca04a76db38f35fc6`
- Licence : MIT, Copyright (c) 2019 Eternalfest

Utilisee par `scripts/hfmap.py`. Le sens du fichier est `clair -> obfusque` :

```json
{ "realScores": "70dik", "world": "]=[]8", "currentId": "-BBEO" }
```

Trois de ces entrees sont verifiables independamment : le reverse engineering
precedent (`cmnemoi/hammerfest-re`, realise par Claude) avait observe en memoire
`realScores` = `70dik`, `fakeScores` = `[t}LJ(` et une propriete `{8` menant a
GameInterface depuis le GameMode. La table donne exactement ces trois valeurs,
dont `gi` = `{8`, ce qui confirme l'hypothese laissee ouverte a l'epoque.

Les identifiants de l'API AS2 standard (`duration`, `length`, `x`...) ne sont
pas renommes par l'obfuscateur et sont donc absents de la table : ils gardent
leur nom en clair dans le SWF.
