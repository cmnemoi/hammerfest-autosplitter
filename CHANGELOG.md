# Changelog

## [0.1.2](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v0.1.1...v0.1.2) (2026-09-23)


### Bug Fixes

* split once into a parallel dimension and once out of it ([b198ce5](https://github.com/cmnemoi/hammerfest-autosplitter/commit/b198ce525f95c880d388af8fc0aa0f16125f6087))

## [0.1.1](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v0.1.0...v0.1.1) (2026-09-23)


### Bug Fixes

* **scripts:** find the game again after a new game starts ([512eee9](https://github.com/cmnemoi/hammerfest-autosplitter/commit/512eee9b343e7341d819a33c9b1a3b6fb5470d8f))
* split when the player enters a parallel dimension ([0d8d096](https://github.com/cmnemoi/hammerfest-autosplitter/commit/0d8d09653e87a48c412c4952fb7c46138d2e46a4))

## 0.1.0 (2026-09-22)


### Features

* end the run at the frame the player enters the elevator ([3071eab](https://github.com/cmnemoi/hammerfest-autosplitter/commit/3071eab94a0959aeeca57b9f2ab58d26ce0b6bb3))
* keep the splits in step when a warp zone skips levels ([f4e173c](https://github.com/cmnemoi/hammerfest-autosplitter/commit/f4e173cd1a2909c01527076b812294419d5f83de))
* name the LiveSplit variables in English ([231f049](https://github.com/cmnemoi/hammerfest-autosplitter/commit/231f049e9d1e0a6dcbeccb7af2f0505f2b02ce9f))
* say which version is running, in the first line of the log ([7325ab7](https://github.com/cmnemoi/hammerfest-autosplitter/commit/7325ab72702fc828819a839a5e3b2f9cf65f6af1))
* split on level changes, timed by the game's own clock ([c0d95c7](https://github.com/cmnemoi/hammerfest-autosplitter/commit/c0d95c76c626611b1be77feb389aec2536128a9a))
* split when the player enters or leaves a parallel dimension ([b61eacc](https://github.com/cmnemoi/hammerfest-autosplitter/commit/b61eacc38af739b6daed3375e605ed719c28a37c))


### Bug Fixes

* date the start of the run after the fact, so a late scan costs nothing ([3d5e396](https://github.com/cmnemoi/hammerfest-autosplitter/commit/3d5e396fa6aa7fd10b482d1156413fd300774bb1))
* never end a run at the elevator of a parallel dimension ([08d7ded](https://github.com/cmnemoi/hammerfest-autosplitter/commit/08d7ded21ec4465276bb8974b5a94a9bc84b8a07))
* never time a game that is already over ([ea2a038](https://github.com/cmnemoi/hammerfest-autosplitter/commit/ea2a03887db770db9f5a3110d9d44e6f569fed52))
* show the timer at once, on a memory map the runtime has not cached ([c0d4a8c](https://github.com/cmnemoi/hammerfest-autosplitter/commit/c0d4a8c68dffd033e1e22a586d145bf41dc14b5e))
* time the next game without waiting out the retry delay ([78711b9](https://github.com/cmnemoi/hammerfest-autosplitter/commit/78711b91ba8f719a8426ac5b7d7bde13c6f88dcc))
