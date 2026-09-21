# vendor/hf.map.json

A mapping table between the identifiers of the Hammerfest source and the
obfuscated names present in the distributed SWF.

- Origin  : <https://gitlab.com/eternalfest/game-types>, `src/lib/hf.map.json`
- Commit  : `1991f15a2df6ed36a097304ca04a76db38f35fc6`
- Licence : MIT, Copyright (c) 2019 Eternalfest

Used by `scripts/hfmap.py`. The file reads `clear -> obfuscated`:

```json
{ "realScores": "70dik", "world": "]=[]8", "currentId": "-BBEO" }
```

Three of these entries can be checked independently. The earlier reverse
engineering (`cmnemoi/hammerfest-re`, done by Claude) observed in memory
`realScores` = `70dik`, `fakeScores` = `[t}LJ(` and a property `{8` leading to
GameInterface from the GameMode. The table gives exactly those three values,
including `gi` = `{8`, which confirms the assumption left open at the time.

The identifiers of the standard AS2 API (`duration`, `length`, `x`...) are
not renamed by the obfuscator and are therefore missing from the table: they
keep their clear name in the SWF.
