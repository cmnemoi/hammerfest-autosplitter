# About AVM1 objects

**Read this when:** an Atom told you "this is an object" and you need to follow
the pointer.

**You need:** [About AVM1 values](avm1-values.md).

---

## Reading an offset

Every shape below is described as a list of offsets. `+0x08` means *eight
bytes after the start of the object*. To get an address, you add:

```text
    the object starts at   0x4793b597430
    the field is at        +0x08
    so you read at         0x4793b597438
```

That is the only arithmetic on this page. The addresses are hexadecimal, so
`0x430 + 0x08` is `0x438`.

---

## Telling one shape from another

The first 8 bytes of every AVM1 object are a **vtable pointer**: the address
of a table of functions that the plugin's own code uses on that object.

Every object of the same shape points at the same vtable. So the first qword
works as a type label:

```text
    MODULE+0x1756db8     String
    MODULE+0x1749ed8     ScriptObject
    MODULE+0x174a460     property table
```

`MODULE+` because the plugin is loaded at a random address every run. We
measure the base once, then add.

We measure these three numbers at run time. They differ between builds and
between platforms.

---

## Shape 1: the String

A String does not hold its characters. It holds a pointer to them.

```text
   String object                          the characters
   @ 0x4793b597430                        @ 0x5ff20321250
  +---------------------------+
  | +0x00  vtable             |          5d 00  3d 00  5b 00  5d 00  38 00
  | +0x08  buffer  ---------- | ------>    ]      =      [      ]      8
  | +0x30  length  =  5       |
  +---------------------------+
```

The characters live somewhere else. `0x5ff20321250` is not near
`0x4793b597430`; it is in a different region of the heap. That is why the
address in the left column jumps when you follow the buffer. A pointer is an
address, and an address can be anywhere.

The characters are UTF-16: two bytes each, and for plain ASCII the second byte
is zero. Five characters, ten bytes, which matches the length field.

So this String reads `]=[]8`.

That is an obfuscated property name, not corruption. [About the
obfuscation](obfuscation.md) says where the real name comes from: `]=[]8` means
`world`.

---

## Shape 2: the property table

An ActionScript object has named properties. AVM1 stores them in a flat array
of slots:

```text
   property table @ 0x4793b413030
  +--------------------------------------------------+
  | +0x00  vtable                                    |
  | +0x08  capacity  =  128 slots                    |
  | +0x58  slot 0    key, value                      |
  |        slot 1    key, value      24 bytes apart  |
  |        ...                                       |
  +--------------------------------------------------+
```

The slots start at `+0x58` and each one is 24 bytes long. Inside a slot:

```text
    the key    at the slot address
    the value  16 bytes BEFORE it
```

The value sits before the key, which looks wrong. The slot is a larger
structure, and we measure only the two fields we use. Where they land inside it
is the allocator's business.

We measure these numbers too. On Linux the same table uses 16 bytes per slot.
Nothing here is hard coded.

---

## Shape 3: the ScriptObject

A ScriptObject is the header of an object. Its only job, for us, is to point
at the property table:

```text
   ScriptObject @ 0x4793b414560
  +---------------------------+        property table
  | +0x00  vtable             |        @ 0x47939aca850
  | +0x30  table   ---------- | ----->  ...
  +---------------------------+
```

So an Atom of tag 6 costs two hops before you can read a property: the Atom
points at the ScriptObject, the ScriptObject points at the table.

---

## One real chain

Putting the three shapes together, on the game we captured:

```text
   GameMode table @ 0x4793b413030
       slot 68     key   -> String ']=[]8'    = world
                   value -> Atom 0x4793b414566, tag 6
                                |
                                v
   ScriptObject @ 0x4793b414560      +0x30
                                |
                                v
   world table @ 0x47939aca850
       slot 3      key   -> String ' h;+A('   = setName
                   value -> String ']R;5E'    = xml_adventure
       slot 31     key   -> String '-BBEO'    = currentId
                   value -> Atom 0x10, tag 0  -> 2
```

The level is 2, and the capture's `metadata.json` recorded `"level": 2`.

---

## Next

| you want | read |
| --- | --- |
| why the names are gibberish | [About the obfuscation](obfuscation.md) |
| how we find the first address | [About finding the game](../internals/finding-the-game.md) |
| why a valid read can still lie | [About stale memory](../internals/stale-memory.md) |
