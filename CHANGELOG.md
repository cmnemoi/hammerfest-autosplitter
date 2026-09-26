# Changelog

## [1.2.0](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v1.1.0...v1.2.0) (2026-09-26)


### Features

* find the game in Ruffle, in a Firefox tab ([09655a0](https://github.com/cmnemoi/hammerfest-autosplitter/commit/09655a0d154159f99ff6920c462a67465935729b))
* read a linear memory by offset, and recognise the build of Ruffle in it ([eca624d](https://github.com/cmnemoi/hammerfest-autosplitter/commit/eca624d984a5949a1b93d8a1e99a5cd25df5bca4))


### Bug Fixes

* keep a Ruffle tab while its game loads ([bc81408](https://github.com/cmnemoi/hammerfest-autosplitter/commit/bc8140885a14f2df28d56254f3ab7dc8f18aa0d1))


### Performance Improvements

* search at once and in full while the SWF loads ([f324625](https://github.com/cmnemoi/hammerfest-autosplitter/commit/f324625916f2b530a3c031a340fad049da3abe6a))
* sweep a linear memory by blocks, and what grew first ([329fc19](https://github.com/cmnemoi/hammerfest-autosplitter/commit/329fc19d87adff649dc1324dd3cc566f9b23021f))
* sweep Pepper Flash in a tight loop, on the first unit of a pattern ([0fc205d](https://github.com/cmnemoi/hammerfest-autosplitter/commit/0fc205d5d08c035fb6b8eec9bc39dddfcb87bafe))
* test the value of a word in a tight loop, and call back only on a match ([c212413](https://github.com/cmnemoi/hammerfest-autosplitter/commit/c212413bb60586e239e9df6083efc7bbd17bfbd4))

## [1.1.0](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v1.0.0...v1.1.0) (2026-09-26)


### Features

* find the game in Ruffle desktop ([534bb49](https://github.com/cmnemoi/hammerfest-autosplitter/commit/534bb49cc51f9d35faf41861cf99420edf00dde4))
* read the game in the heap of Ruffle 0.6.0 ([ea8804a](https://github.com/cmnemoi/hammerfest-autosplitter/commit/ea8804a07b336bfc6954038d92df899ab3f4f3f0))


### Bug Fixes

* keep waiting between scans while no game is held ([9bc9ea8](https://github.com/cmnemoi/hammerfest-autosplitter/commit/9bc9ea8d4a363cd3c2d5de8651a1ea35c11378cd))

## [1.0.0](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v0.3.0...v1.0.0) (2026-09-25)


### Miscellaneous Chores

* release 1.0.0 ([1565bfd](https://github.com/cmnemoi/hammerfest-autosplitter/commit/1565bfd3d2a2667d88c25677811fe0756877cede))

## [0.3.0](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v0.2.0...v0.3.0) (2026-09-24)


### Features

* Support Linux and macOS ([d673173](https://github.com/cmnemoi/hammerfest-autosplitter/commit/d673173f35463e95bcca743acbacc74761422934))

## [0.2.0](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v0.1.2...v0.2.0) (2026-09-23)


### Features

* show the version of the autosplitter in its settings ([a76c59d](https://github.com/cmnemoi/hammerfest-autosplitter/commit/a76c59d56e5f054c03d3d4dd3ee117c9c625bef7))


### Bug Fixes

* show the dimension in the World variable ([db2cf14](https://github.com/cmnemoi/hammerfest-autosplitter/commit/db2cf147ad8efc9bb5f6ff0759734b95ff9fcc54))

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
