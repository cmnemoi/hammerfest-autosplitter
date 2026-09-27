# Changelog

## [1.4.0](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v1.3.0...v1.4.0) (2026-09-27)


### Features

* find the game in Adobe's Flash projector under macOS ([b446574](https://github.com/cmnemoi/hammerfest-autosplitter/commit/b446574058f4fa551a93913add3306873eb9849d))


### Performance Improvements

* look at the first 128 MiB before the search by content reads the rest ([a087773](https://github.com/cmnemoi/hammerfest-autosplitter/commit/a08777335858f62637d90ab135d153d529e720af))
* look for no world in Ruffle while the GameManager is not born ([76db85a](https://github.com/cmnemoi/hammerfest-autosplitter/commit/76db85a1a1e2f921e907df2a27d61648a5692780))
* never try the seed of the Windows plugin on a Flash projector ([5cecb54](https://github.com/cmnemoi/hammerfest-autosplitter/commit/5cecb54d9d2936b380274a0210a60717d73d7b2c))
* start the plugin under Linux from the seed of its build ([cbf6dfa](https://github.com/cmnemoi/hammerfest-autosplitter/commit/cbf6dfa3f69b504aec3c0d92c1c6e5e0c3d5af37))
* start the plugin under macOS from the seed of its build ([57a99a6](https://github.com/cmnemoi/hammerfest-autosplitter/commit/57a99a641ddd97340f07ad4f5e9781b136880b83))
* sweep no range larger than 64 MiB under Rosetta 2 ([27ec753](https://github.com/cmnemoi/hammerfest-autosplitter/commit/27ec753545be0404909a14cfc0c0bdb4f038594b))
* sweep the ranges that follow each other as one under Rosetta 2 ([9efceaa](https://github.com/cmnemoi/hammerfest-autosplitter/commit/9efceaa23080ec84f70bee3db6a6078edbe30ad8))
* trust the known build of each Flash projector ([0c9504e](https://github.com/cmnemoi/hammerfest-autosplitter/commit/0c9504e76a21dd94d8f4f6a4afe7151e53698bc8))

## [1.3.0](https://github.com/cmnemoi/hammerfest-autosplitter/compare/v1.2.0...v1.3.0) (2026-09-26)


### Features

* find the game in Adobe's Flash projector under Linux ([58eea7f](https://github.com/cmnemoi/hammerfest-autosplitter/commit/58eea7f483f72cf22889d484a6005524895c0ac7))
* find the game in the Windows Flash projector ([5e27358](https://github.com/cmnemoi/hammerfest-autosplitter/commit/5e27358ee8dc0400c413ed3d8c8847037801aeda))
* read a 32-bit build of Flash Player, in words of four bytes ([61b0188](https://github.com/cmnemoi/hammerfest-autosplitter/commit/61b0188d72e0806a31e006e8541f81b7570bd636))

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
