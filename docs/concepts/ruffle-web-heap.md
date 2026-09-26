# About the heap of Ruffle in a browser

> **Read in the source of Ruffle 0.6.0 and in its two released `.wasm`, then
> checked on a live game in Firefox under Linux, on 2026-09-26.** Chrome and
> Windows are not checked yet. Each line says where it comes from:
>
> - **[S]** read in the source;
> - **[B]** read in the released `.wasm`, disassembled with `wasm-tools`;
> - **[W]** read on the web: stores, npm;
> - **[H]** a hypothesis, to check in a live browser.
>
> The desktop layout is on [About the Ruffle heap](ruffle-heap.md). This page
> says what changes when the same source is compiled to WebAssembly.

**Read this when:** you work on reading Ruffle in Firefox or Chrome.

---

## Where Hammerfest runs in a browser

- [W] hammerfest.fr is closed.
- [S+W] eternalfest.net hosts no Ruffle. Its game is a plain
  `<embed type="application/x-shockwave-flash" src="/assets/loader.swf">`
  (`eternalfest/packages/website/src/app/runs/game.component.html:1-13`,
  checked in the live site too). In a browser, it is the **Ruffle extension**
  that replaces that embed.
- [S] The extension runs Ruffle in the main world of the page
  (`web/packages/extension/src/background.ts:212-242`). [H] So the `.wasm`
  lives in the process of the tab, not in the one of the extension.

## The build

- [S] `cargo build --target wasm32-unknown-unknown`, then `wasm-bindgen
  --target web`, then `wasm-opt -O -g`: `web/packages/core/tools/build_wasm.ts`.
- [S] Two builds ship. **Extensions** (bulk-memory, simd128,
  nontrapping-fptoint, sign-ext, reference-types) and **MVP**
  (`target-cpu=mvp`). The page picks one at run time with wasm-feature-detect
  (`load-ruffle.ts:38-75`). [H] Current Firefox and Chrome load extensions.
- [B] The extension and the self-hosted package ship **the same two binaries**:

  | build | sha256 |
  | --- | --- |
  | extensions | `e4ba64aa1dc9f7f2368602dd0fc2c51046f3e35baba8116d6cf3ae930a63aa02` |
  | MVP | `adabc1696a2f1f95715ede6be0ac00a73364895c8e599039e60fef3b2f52efa4` |

- [B] Both keep their `name` section (`wasm-opt -g`): every Rust function
  name can be read. rustc 1.98.1, wasm-bindgen 0.2.127.
- [S+B] The allocator is dlmalloc, the default of `std` on this target.
- [B] Memory: `(memory 73)`, 32 bits, no maximum, not shared. The stack runs
  from 0 to `0x1E8480`, the static data from `0x1E8480` to about `0x3FB303`,
  and the heap of dlmalloc starts above `0x490000` and grows with
  `memory.grow`.

**What it means for the reader.** The whole AVM1 heap is in one contiguous
block, and every pointer in it is a **u32 offset** from the base of that
block. The address in the process is `base + offset`.

---

## The layout in wasm32

[B] The same in both builds. Only the addresses of the statics differ.

### The garbage collector

- `Object::new_without_proto` calls `__rdl_alloc(88, 4)`: `next` at `+0`,
  `vtable | flags` at `+4`, and the `Gc` points at `+8`. **The header is 8
  bytes.**
- `CollectVtable`, aligned on 16, in the data segment: `align` (u32) at `+0`,
  `size` (u32) at `+4`, then two table indexes.

  | type | extensions | MVP | align, size |
  | --- | --- | --- | --- |
  | `RefLock<ObjectData>` | `0x2C69A0` | `0x253890` | 4, 80 |
  | `AvmStringRepr` | `0x315F30` | `0x316310` | 4, 20 |

  The whole vtable reads `(4, 80, 4417, 4418)` for the object of the
  extensions build, `(4, 20, 6388, 6389)` for its string.
- **Layout (4, 80) is not unique.** `RefLock<BitmapRawData>` has it too, and
  so does a `dyn` vtable of the tracing crate. The checks on the map tell them
  apart.

### A string, 20 bytes

```text
+0x00 ptr (the units)     +0x04 meta (length | wide << 31)
+0x08 capacity | interned << 31
+0x0C chars_used          +0x10 owner (0 = none)
```

The order is not the one of x86-64, where `owner` sat at `+0x08`.

### An object, 80 bytes

```text
+0x00 borrow flag of the RefCell (i32, 0 at rest)
+0x04 entries.cap     +0x08 entries.ptr     +0x0C entries.len
+0x10 indices.ctrl    +0x14 bucket_mask
```

### An entry, 40 bytes, aligned on 8

```text
+0x00 Value (16 bytes)
+0x10 to +0x1B getter, setter, id [H order]
+0x1C attributes (u16)
+0x20 key -> AvmStringRepr
+0x24 hash (u32)
```

Read in `get_index_of` (`idx * 40`, `load offset=32`) and in `insert_full`
(`load offset=36`).

### A value, 16 bytes, aligned on 8

The same tags as desktop, 0 to 6. The boolean at `+1`, the f64 at `+8`, but
**a string or an object pointer at `+4`**, not `+8` (`coerce_to_f64`).

### The hash is 32 bits

- [S] indexmap stores `HashValue(h as usize)` (`map.rs:811`), and `usize` is
  32 bits here.
- [B] rustc kept only the low half of the FNV constants: basis `0x84222325`,
  multiplier `0x1b3`.

So the hash is `key_hash(key) as u32`: `]=[]8` gives `0xa609b714`, `world`
`0x5ad275e8`, `__proto__` `0xf9df01ac`.

---

## The version, and how often it moves

- [W] Firefox add-ons: 0.6.0 since 2026-09-06, about 84 500 users a day. Before
  it, 0.5.0 on 08-03, 0.4.0 on 07-18. About one stable release a month.
- [S] A nightly is published every day on GitHub and npm. The stores get the
  stable releases only (`release.yml:521-560`).
- A site that hosts Ruffle itself may pin any version. eternalfest.net does
  not host it, so the extension decides.

---

## Reading it from outside the browser

Measured on this machine with Firefox ESR 140 under Linux, on a small test
page, not yet on Ruffle.

- **Rights.** A process of the same user reads a sandboxed `Isolated Web Co`
  with `process_vm_readv`. The sandbox creates a user namespace the user owns,
  so the read is allowed. It fails with `ptrace_scope` 2 or 3, and may fail
  under Snap. [H] Windows allows reading a lower integrity process.
- **Which process.** Firefox spreads a site over up to four `Isolated Web Co`
  processes (`dom.ipc.processCount.webIsolated`). Two held the test module,
  one per tab. Every candidate process must be looked at.
- **The shape of the memory.** SpiderMonkey reserves one anonymous range: a
  header page of 4 KiB, the base of the linear memory, the committed part in
  `rw-p`, then a reserve in `---p`, 4 GiB + 32 MiB + 64 KiB from the base in
  all. The base is the start of the range plus `0x1000`, aligned on 4 KiB
  only. It **does not move** when the memory grows.
- **The false positives.** A search for bytes of the module also finds the
  copies of the `.wasm` on the JavaScript heap. The shape of the range is what
  tells the real linear memory.

---

## A proposed search

**Find the base, once per process.** Keep the ranges that are `rw` and
followed by a reserve of at least 4 GiB. Base candidate: start plus `0x1000`.
Confirm it by the vtables of a known build: `(4, 80, 4417, 4418)` at
`base + 0x2C69A0` and `(4, 20, 6388, 6389)` at `base + 0x315F30` for
extensions, the MVP constants otherwise. The build is then known too.

**Pass A.** Sweep the aligned u32 of the linear memory for the 32-bit hash of
the key. A hit at offset `h` gives an entry `b = h - 0x24`: it must be aligned
on 8, its tag at most 6, and its key `[b + 0x20]` must read as the key.

**Pass B.** Sweep the aligned u32 for a `p` whose next u32 is a length `len`
such that `p <= b < p + 40 * len` and `(b - p) % 40 == 0`. The object is
`o = offset(p) - 8`. It must carry a vtable of layout (4, 80) in the data
segment, a borrow flag at 0, and a length not above its capacity.

**Then the fast path.** Read `o + 4 .. o + 16`, read `len * 40` bytes of
entries, pick the entry whose hash at `+0x24` is the hash of the name, and
decode its value.

The sweeps read only the linear memory, a few tens to a few hundred MiB,
instead of the 300 MiB of the desktop heap.

---

## Seen on a live game

On 2026-09-26, Firefox ESR 140 under Linux, the Ruffle extension 0.6.0, a
game at level 2 of `xml_adventure` on eternalfest.net.
`mise run ruffle-web-state` reads it, and `fixtures/replay/ruffle-web-main-world`
keeps it: 140 pages, 164 KiB.

- **Firefox loads the extensions build.** Its two vtables are where the binary
  says, which proves the base.
- **The linear memory is 99 MiB**, found by its shape: an `rw` range followed
  by the reserve.
- **Every offset of this page holds**: the header of 8 bytes, the entry of 40,
  the hash of 32 bits, the pointers at `+4` in a value, the string as `ptr,
  meta`. Pass A found 5 entries of `]=[]8`, pass B 4 objects that own them,
  and one of them is the `GameMode`: level `2.0`, previous `1.0`.
- **The search took 0.78 s**, in Python, against about 20 s for the 300 MiB
  of Ruffle desktop.

## Still to check

- Chrome, and Windows.
- Whether a tab of another site in the same process can hide the game.
- The order of getter, setter and id in an entry, and where a clip reference
  sits. The reader needs neither.
