# About the Ruffle heap

> **Read in the source of Ruffle 0.6.0 and in its Linux binary, then checked
> on a live game under Linux, on 2026-09-26.** Windows is not checked yet. Each
> line says where it comes from:
>
> - **[S]** read in the source;
> - **[B]** read in the Linux 0.6.0 binary, by disassembly;
> - **[L]** seen on a live game, under Linux;
> - **[H]** a hypothesis, still to check.
>
> See [Ruffle support](../specs/ruffle-support.md) for why this page exists.

**Read this when:** you work on `RuffleHeap`, or you want to know how Ruffle
keeps an AVM1 object in memory.

**You need:** [About AVM1 objects](avm1-objects.md) and
[About finding the game](../internals/finding-the-game.md). This page says what
changes from Pepper Flash.

---

## The sources

- Ruffle: tag `v0.6.0`, commit `cac5c99ce4a17e606f4ee3090389bb878f852055`.
- Crates, at the versions of its `Cargo.lock`: `gc-arena` 0.7.0, `indexmap`
  2.14.0, `hashbrown` 0.17.1.
- Binary: the official `linux-x64` release, as Eternalfest Desktop fetches it
  (`eng/ruffle.json`). Built by rustc 1.98.1 and LLD 22.

File paths below are relative to the Ruffle repository, or to the crate named.

---

## Strings

- [S] `AvmString<'gc>(Gc<'gc, AvmStringRepr<'gc>>)`:
  `core/common/src/avm_string/avm_string.rs:12`. `AvmAtom` has the same repr,
  marked as interned: `interner.rs:18`.
- [S] `AvmStringRepr`, `core/common/src/avm_string/repr.rs:12-33`:
  `ptr: *mut ()`, `meta: WStrMetadata(u32)`, `capacity: Cell<WStrMetadata>`
  (bit 31 = interned), `chars_used: Cell<u32>`, `owner: Option<Gc<Self>>` (set
  for a substring that depends on another).
- [S] `WStrMetadata(u32)`: bits 0 to 30 are the length in units, bit 31 is
  "wide" (UTF-16). `wstr/src/ptr.rs:42-51,74-87`. Not wide is `[u8]` Latin-1;
  wide is `[u16]` UTF-16LE.
- [S] A UTF-8 text from the SWF becomes Latin-1 when every character is below
  256, UTF-16 otherwise: `wstr/src/buf.rs:157-170`.

  So `]=[]8` is **5 bytes**, `5d 3d 5b 5d 38`. Pepper Flash stores it as 10
  bytes of UTF-16.

- [B] The layout, from `AvmString::new_utf8_bytes` at `0x1102f68..0x1102faa`.
  The allocation is `malloc(0x30)`: a GC header of 16 bytes, then the repr of
  32 bytes.

  ```text
  repr +0x00 ptr (the units)     +0x08 owner (0 = none)
       +0x10 meta (u32)          +0x14 capacity | interned << 31 (u32)
       +0x18 chars_used (u32)    size 32
  ```

  `get_index_of` confirms it: `mov (%rax),%rdi ; mov 0x10(%rax),%r8d`.

- [S] The interner, `AvmStringInterner { interned: WeakSet<AvmStringRepr> }`, is
  a `hashbrown::HashTable<GcWeak>` with an FNV hasher:
  `interner.rs:54-59,128-134`. It is **weak**: an atom dies when nothing cites
  it any more.
- [S] The AVM1 constant pool interns each of its strings (`intern_wstr`) and
  keeps a `Gc<Vec<Value>>`: `core/src/avm1/activation.rs:919-936`,
  `context.rs:41-51`. While the pool or a key cites it, there is one single
  interned repr of `]=[]8`.
- [S] But `ActionPush` of a literal string outside the pool makes an
  `AvmString::new` that is **not** interned: `activation.rs:1868`. A property
  key can therefore be a repr that nothing else shares.
- [S] The key a map keeps is the `AvmString` of the first `insert`:
  `property_map.rs:33-57,82-90`. `SetMember` passes
  `name_val.coerce_to_string()` as it is: `activation.rs:1937-1946`.

---

## Objects

- [S] `Object<'gc>(Gc<'gc, RefLock<ObjectData<'gc>>>)`:
  `core/src/avm1/object/script_object.rs:60`. `RefLock` is a
  `#[repr(transparent)]` over `RefCell`: `gc-arena/src/lock.rs:40,225`.
- [S] `ObjectData { native, properties: PropertyMap<Property>, interfaces,
  watchers }`: `script_object.rs:96-101`.

  There is no prototype field. `__proto__` is an ordinary property, with
  `DONT_ENUM | DONT_DELETE`: `script_object.rs:128-150`.

- [S] An instance of an AS2 class, such as `new GameMode()`, is an `Object`
  with `NativeObject::None`. A clip instance would carry
  `NativeObject::MovieClip`, and its properties stay in the same map.
- [S] `PropertyMap<'gc, V>(IndexMap<PropertyName, V, FnvBuildHasher>)`:
  `property_map.rs:13-18`, with `PropertyName(AvmString)` at `:190`.

  The hash **never** depends on case. Equality does when the SWF version is
  above 6: `:153-201`, and `cmp $0x6,%cl` in `get_data`.

- [S] The hash of a key: FNV-1a 64 over each unit, lowered by
  `swf_to_lowercase` (`wstr/src/utils.rs:89-98`) and written as a u16 LE, then
  one byte `0xff`: `property_map.rs:198-201`.

  [B] `get_index_of` at `0x12158d0` confirms it: the constants
  `0xcbf29ce484222325` and `0x100000001b3`, and `xor $0xff`.

- [S] IndexMap 2.14: `Core { indices: HashTable<usize>, entries:
  Vec<Bucket<K, V>> }` (`indexmap/src/inner.rs:21-34`), and
  `Bucket { hash: HashValue(usize), key, value }` (`indexmap/src/lib.rs:137-151`).
- [S] `Property { id: u32, data: Value, getter: Option<Object>, setter:
  Option<Object>, attributes: Attribute(u16) }`:
  `core/src/avm1/property.rs:50-57`.
- [B] The layout, from `Object::get_data` at `0x11fceb0`, `get_index_of` at
  `0x12158d0` and `insert_full` at `0x1217430`:

  ```text
  Object (the value a Gc points at, which is the RefCell)
   +0x00 borrow flag of the RefCell (isize, 0 at rest)
   +0x08 ObjectData.properties (the IndexMap, 56 bytes):
         +0x08 entries.cap    +0x10 entries.ptr    +0x18 entries.len
         +0x20 indices.ctrl   +0x28 indices.bucket_mask
         +0x30, +0x38 growth_left and items [H]
   RefLock<ObjectData> is 160 bytes (the Layout in its CollectVtable)

  Bucket (a stride of 0x38 = 56 bytes, in the entries Vec)
   +0x00 Property.data, a Value (16 bytes)
   +0x10 to +0x27 getter, setter, id, attributes
         [H] getter +0x10, setter +0x18, id +0x20, attributes +0x24
   +0x28 key -> AvmStringRepr
   +0x30 hash (the u64 FNV above)
  ```

- [S] Flash iterates in the reverse order of insertion (`iter().rev()`):
  `property_map.rs:92-95`. `remove` is `shift_remove`, so indexes move after a
  removal.

---

## Values

- [S] `enum Value { Undefined, Null, Bool(bool), Number(f64),
  String(AvmString), Object(Object), MovieClip(MovieClipReference) }`, at most
  16 bytes: `core/src/avm1/value.rs:20-31`.
- [B] From `coerce_to_f64` at `0x11fcf40`, and `get_data`:

  ```text
  +0x00 u8 tag: 0 Undefined, 1 Null, 2 Bool, 3 Number, 4 String, 5 Object,
                6 MovieClip
  +0x01 u8 the boolean (tag 2)
  +0x08 f64 (tag 3) | *AvmStringRepr (tag 4) | *Object (tag 5)
        | *MovieClipReferenceData (tag 6)
  ```

  `Option<Value>::None` uses the tag `0xff`.

- [S] Ruffle has no integer in AVM1: `SwfValue::Int(v) => v.into()` gives a
  `Number(f64)` (`activation.rs:1865`). So `currentId` is the f64 `2.0`, where
  Pepper Flash keeps an integer atom.

---

## The garbage collector: gc-arena 0.7.0

- [S] A `Gc<T>` is one pointer to the **value**. The header sits just before
  it, in the same allocation: `gc-arena/src/types.rs:13-17,40-58`.
- [S] `GcHeader { next: Cell<Option<GcPtr>>, tagged_vtable: Cell<*const
  CollectVtable> }`, 16 bytes: `types.rs:153-163`. The low bits of the vtable
  are flags: 0 and 1 the colour, 2 needs_trace, 3 is_live. Mask them with
  `& !0xF`.
- [B] An allocation is `malloc(16 + size_of::<T>())`: `next` at `chunk+0`, the
  vtable and its flags at `chunk+8`, the value at `chunk+0x10`.
- [S] `CollectVtable` is `#[repr(align(16))]`, `{ trace_value, drop_value,
  value_layout: Layout }`, one static per type: `types.rs:68-76`.

  [B] In the binary the order is `+0 align`, `+8 size`, `+0x10 trace`,
  `+0x18 drop`. So two reads in the module tell the type of any GC object,
  with no symbol.

  The vtables of Linux 0.6.0, as offsets in the module:

  | type | vtable | align | size |
  | --- | --- | --- | --- |
  | `RefLock<ObjectData>`, the AVM1 object | `0x1d79120` | 8 | 160 |
  | `AvmStringRepr`, the `ruffle_common` copy | `0x1d66cd0` | 8 | 32 |
  | `AvmStringRepr`, the second copy | `0x1d79300` | 8 | 32 |
  | `Vec<Value>`, the constant pool | `0x1d79140` | 8 | 24 |
  | `MovieClipReferenceData` | `0x1d795c0` | 8 | 40 |

  Both string vtables are used (`lea ... 1d66cdc` and `lea ... 1d7930c`). A
  reader must accept both.

- [S] Nothing moves: the global allocator allocates, the sweep frees, and
  nothing compacts. **But** the buffers inside a map, the `entries` Vec and the
  `indices` table, are allocated again when the map grows. Anchor on the
  `Object`, and read `entries.ptr` again on every read.
- [S] Every GC object is on one global list, through `next`. The list is too
  long to walk.

---

## What Rust promises, and what it does not

- [S] None of these types is `#[repr(C)]`. Only `RefLock` is
  `repr(transparent)`, and `CollectVtable` is `repr(align(16))`.

  The order of the fields is rustc's choice. Here it put `Property` before
  `key` and `hash` in `Bucket`, `Layout.align` before `size`, and `entries`
  before `indices`.

- [B] The offsets on this page come from the Linux 0.6.0 binary. [H] Windows
  builds the same source for the MSVC target, with the same rustc, so the
  layout should be the same.
- Probes at run time, the equivalent of the Pepper Flash layout profiles:

  1. The vtable of an object is the qword at `value - 8`, masked with `& !0xF`.
     It points into the module, and `[vt+0] = 8` with `[vt+8] = 160` for an
     object, or `32` for a string. That proves the type, with no symbol.
  2. A string: `[r+0x10] & 0x7fffffff` is the length, bit 31 is wide,
     `[r+8]` is 0 or a heap pointer, and the bytes at `[r]` are readable.
  3. A bucket: `[b+0x30]` is the FNV of the key that `[b+0x28]` points at, and
     the tag `[b]` is at most 6.
  4. An object: `[o] == 0` (not borrowed), `[o+0x18] <= [o+0x08]` (the length
     is not above the capacity), and `[o+0x10]` points at the bucket.

  A profile is then: the offsets of the repr, the offsets of the map in the
  object, the bucket stride, the offsets of key, hash and value, and the
  Value tags. When a probe fails, try the plausible permutations, as the
  Pepper Flash profiles do.

---

## A proposed search

The `hash` field of a bucket is a u64 that can be **computed beforehand**, from
the obfuscated name alone:

```text
FNV(']=[]8')     = 0xd7ae320ea609b714    little endian: 14 b7 09 a6 0e 32 ae d7
FNV('world')     = 0xb24665a85ad275e8
FNV('__proto__') = 0x49a69182f9df01ac
FNV('fVersion')  = 0xe7c0917e6f3bb74a    if the key really is 'fVersion'; hash
                                         the obfuscated key otherwise
```

**Pass A**, one sweep of the heap over aligned qwords: look for `FNV(key)`,
several keys at once. Each hit `h` gives a bucket `b = h - 0x30`. Check that
`[b+0x28]` leads to a repr whose bytes are the key, and that the tag `[b]` is
at most 6. The string need not be found first, and a key that is not interned
is found too.

**Pass B**, one sweep: find the owner. Look for a qword `p` such that
`p <= b < p + 56 * len`, with `len` the next qword and `(b - p) % 56 == 0`.
The object is then `o = address(p) - 0x10`. Check its vtable, `[o-8] & !0xF`,
for the Layout 8 and 160.

A variant with no second rule: walk back from `b` in steps of 56 while the
buckets stay coherent, which gives `S = entries.ptr`, then look for the exact
qword `S`. The cost is the same, and the comparison is simpler.

**Then the fast path**, two reads per hop: read `o+0x08..0x20` (capacity,
pointer, length), read `len * 56` bytes, pick the bucket whose `[+0x30]` is
`FNV(name)`, and decode its Value at `+0x00`.

```text
GameManager.current -> GameMode.world -> currentId (f64), setName (string)
```

**A dead object.** glibc writes over `chunk+0` (the tcache next) and `chunk+8`
(the tcache key) when it frees, which is the GC header. The vtable check then
fails.

**Cost.** Two full sweeps, as for Pepper Flash, which searches bytes and then
pointers. But pass A compares one u64 per qword, with no unaligned byte search,
and lands on the buckets at once. The differential scan still applies.

**A slower alternative.** Search the Latin-1 bytes of `]=[]8` (hits: the
decompressed SWF, buffers), then a qword equal to that buffer (the repr, with
`meta = 5`), then a qword equal to the repr (buckets at `+0x28`, the pool
`Vec<Value>` at `+8` after a tag 4, the interner), then pass B. Three or four
sweeps.

---

## The binary and the allocator

- [B] Linux: `ELF 64-bit PIE, not stripped`. Its `.symtab` has 28 732 symbols,
  with Rust names (v0 mangling), 1 435 of them in `ruffle_core::avm1`,
  including the closures `CollectVtable::vtable_for<T>`. There is no
  `.debug_info`. The vtables are found through the `R_X86_64_RELATIVE`
  relocations that point at those closures.
- [B] Windows: `PE32+`, with a separate PDB (`ruffle_desktop.pdb`) that is not
  shipped. No symbols: probe at run time.
- [S] The allocator. The `#[global_allocator]` exists only with the `tracy`
  feature: `desktop/src/main.rs:46-49`. [B] The binary has no tracy, mimalloc
  or jemalloc symbol, and imports `malloc@GLIBC`. So it is `std::alloc::System`:
  glibc malloc on Linux, `HeapAlloc(GetProcessHeap())` on Windows.
- [H] Linux: the small objects of the main loop in `[heap]`, the main brk
  arena. Allocations above the mmap threshold (128 KiB, dynamic), and the
  arenas of other threads (heaps of 64 MiB), in anonymous RW mappings.
  Windows: the private RW regions of the process heap.

---

## Seen on a live game

[L] Under Linux, Eternalfest Desktop 0.1.0, Ruffle 0.6.0, a game at level 2 of
`xml_adventure`. `mise run ruffle-state` reads it, and
`fixtures/replay/linux-ruffle` keeps it.

- **The search above works as written, on the first try.** Pass A found 18
  buckets of `]=[]8`, pass B found 19 objects that own them, and one of them is
  the `GameMode`: its `manager` names it back as `current`.
- **Every offset of the bucket, the object and the value holds**: the stride
  of 56, the key at `+0x28`, the hash at `+0x30`, the map at `+0x08`, the
  Value tags, the vtable Layout 8 and 160, the Latin-1 strings.
- **`currentId` is the f64 `2.0`**, and `_previousId` the f64 `1.0`.
- **The whole game lives in `[heap]`**, the brk heap of glibc, and in two
  mappings of the module: the vtables, and the static strings such as
  `__proto__`. The 110 anonymous mappings hold none of it.
- **`GameMode.duration` follows real time.** Over 60.13 s of wall clock,
  `duration` advanced 1924 cycles, that is 60.12 s: 0.01 %. Pepper Flash gives
  0.1 %.

What it costs, and it is the one bad news: the heap is **300 MiB in 111
ranges**, against about 80 MiB for Pepper Flash. The two sweeps read 601 MiB.
Since the game lives in `[heap]` alone, sweeping `[heap]` first is the obvious
lead. That is for the design of `RuffleHeap`, and the read cost baseline will
say what it saves.

## Still to check

- The Windows `.exe` has the same layout. Probe it: the Layout in a vtable,
  the meta and bytes of a string, a bucket hash equal to the FNV of its key,
  a RefCell flag at 0. And which regions hold the heap there.
- The order of getter, setter, id and attributes in `Property`. The reader
  needs only the Value at `+0x00`, so it may never matter.
- Whether the keys are interned. Pass A does not need them to be.
- Finding a dead object by its overwritten header is safe under glibc, and
  uncertain under the Windows heap (LFH).
- The real key of `fVersion`. Its hash above assumes it is not obfuscated.
