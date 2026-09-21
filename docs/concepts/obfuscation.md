# About the obfuscation

**Read this when:** you found a property name like `]=[]8` and want to know
what it means.

**You need:** nothing.

---

## What was done to the names

The `.swf` was shipped obfuscated. Every property name was replaced by a short,
meaningless, unique string:

```text
   in the source            in memory
   currentId          ->    -BBEO
   world              ->    ]=[]8
   setName            ->     h;+A(         (that first character is a space)
   gameChrono         ->    8qkdA
   fl_lock            ->    [8_on
   fl_gameOver        ->    6NDsC(
   endModeTimer       ->    ;iaa}
```

They are short and unique, and not meant to be read.

---

## The table is public

`eternalfest/project-phoenix`, the decompiler of the game, depends on
`game-types/src/lib/hf.map.json`: 2952 `clear → obfuscated` pairs, under
the MIT licence. It is vendored here as `vendor/hf.map.json`, with its notice
in `vendor/NOTICE.md`.

So we look a name up rather than search for it:

```text
   we want              currentId
   hf.map.json says     -BBEO
   we scan for          "-BBEO" in UTF-16
```

Without that table, each name would have to be identified by observing the
game, and a rename in a later build would be invisible until something broke.

---

## How it reaches the Rust

`build.rs` reads `vendor/hf.map.json` at compile time and writes the constants
we need into `keys.rs`:

```rust
   pub const CURRENT_ID: &str = "-BBEO";
   pub const END_MODE_TIMER: &str = ";iaa}";
```

Only the names listed in `build.rs` are emitted. Adding one is a single line
there.

---

## The name that is not obfuscated

`duration` belongs to the standard ActionScript API. Renaming it would break
the Flash player itself, so the obfuscator left it alone, and it is absent from
`hf.map.json`.

`build.rs` checks that it stays absent. If it appeared, the table would have
changed its convention, and we would silently read the wrong field. The check
turns that into a build failure.

---

## The world names too

`xml_adventure` and the four parallel worlds are obfuscated as well, because
they look like identifiers to the obfuscator.

We use them as a validity test: a `GameMechanics` whose `setName` is not one of
the five known worlds is not a Hammerfest world, and the read is rejected. See
[About stale memory](../internals/stale-memory.md).

---

## Looking a name up yourself

```sh
uv run python -c "import sys; sys.path.insert(0,'scripts'); import hfmap; print(hfmap.obf('endModeTimer'))"
```

`hfmap.clear` goes the other way.
