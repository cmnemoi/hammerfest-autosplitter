# About AVM1 values

**Read this when:** you need to turn eight bytes of Hammerfest memory into a
number, a word or an object.

**You need:** nothing. The page starts from the bits.

---

## The problem AVM1 had to solve

ActionScript has no types on its variables. One variable holds an integer
today and an object tomorrow.

So the virtual machine needs a slot that can hold **any** value, and it wants
that slot to be one machine word: 64 bits, 8 bytes.

A pointer already fills 64 bits. There is no room left to say *what* is
stored.

AVM1 found room anyway. Every value we read out of the game comes back in this
form.

---

## The three operators you need

If `&`, `>>` and `~` are familiar, go to the next section.

A number is a row of bits. `7` is three bits set:

```text
    7   =   ... 0000 0111
                     ^^^^
                     the three lowest bits
```

**`&` keeps a bit only when both sides have it.** So `x & 7` throws away
everything except the three lowest bits of `x`:

```text
    0x16    =   ... 0001 0110
    7       =   ... 0000 0111
    --------------------------  &
    result  =   ... 0000 0110   =  6
```

**`~` flips every bit.** So `~7` is everything *except* the three lowest:

```text
    7       =   ... 0000 0111
    ~7      =   ... 1111 1000
```

**`x & ~7` therefore clears the three lowest bits** and leaves the rest alone:

```text
    0x16    =   ... 0001 0110
    ~7      =   ... 1111 1000
    --------------------------  &
    result  =   ... 0001 0000   =  0x10
```

**`>> 3` slides every bit three places to the right.** Each slide halves the
number, so three slides divide it by 8:

```text
    0x10    =   ... 0001 0000     =  16
    >> 3    =   ... 0000 0010     =  2
```

Three operators, one purpose each: read the low bits, erase the low bits,
shift.

---

## The Atom

Everything AVM1 allocates sits at an address that is a multiple of 8. In
binary, a multiple of 8 always ends in three zero bits.

So in every pointer AVM1 holds, three bits are always zero, and therefore
wasted. AVM1 spends them on a type tag:

```text
     63                                          3   2 1 0
    +----------------------------------------------+-----+
    |  the value, or a pointer                     | tag |
    +----------------------------------------------+-----+
                                                    \___/
                                                  3 bits, 8 types
```

This packed word is called an **Atom**. Reading one is two operations:

```text
    tag   =  atom &  7      what kind of value is this?
    body  =  atom & ~7      the pointer, with the tag erased
```

---

## Worked example: a level number

The `currentId` property of a real game, captured on level 2:

```text
    atom  =  0x10
```

Step 1, the tag:

```text
    0x10    =   ... 0001 0000
    7       =   ... 0000 0111
    --------------------------  &
              =   ... 0000 0000   =  0   ->  tag 0 is "integer"
```

Step 2, the value. For an integer the whole word is the number, shifted up by
three places to make room for the tag. Slide it back:

```text
    0x10 >> 3   =   2
```

So the player is on level 2.

---

## Worked example: an object

The `world` property of the same game:

```text
    atom  =  0x4793b414566
```

Step 1, the tag. Only the last digit matters, because `& 7` throws the rest
away:

```text
    ...6    =   0110
    7       =   0111
    ----------------  &
              =   0110   =  6   ->  tag 6 is "object"
```

Step 2, the pointer. Erase the three low bits:

```text
    ...566  =   ... 0110 0110
    ~7      =   ... 1111 1000
    ----------------------------  &
              =   ... 0110 0000

    0x4793b414566  &  ~7   =   0x4793b414560
```

An integer needs a shift. A pointer needs a mask, because the address was
never shifted: the three bits it gave up were already zero.

---

## The eight tags

| tag | meaning | how to read the body |
| --- | --- | --- |
| 0 | integer | `atom >> 3` |
| 1 | double | the body points at 8 bytes of IEEE-754 |
| 2 | special | `0x0A` null, `0x12` false, `0x32` true |
| 3 | native | not used by us |
| 5 | string | the body points at a String object |
| 6 | object | the body points at a ScriptObject |
| 4, 7 | unidentified | never needed |

The code is `core/src/atom.rs`. It is the one part of the memory layer with no
memory access, which is why it carries unit tests.

---

## Where the value lives

An Atom of tag 5 or 6 is a pointer. Following it means knowing the shape of
what is on the other end. Those shapes are the subject of
[About AVM1 objects](avm1-objects.md).
