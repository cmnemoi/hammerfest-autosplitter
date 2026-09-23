"""The AVM1 object model in a Pepper Flash process, layout *derived* at run time.

The earlier reverse work (Linux x86-64, libpepflashplayer.so 32.0.0.465) had
proved this layout. On Windows (pepflashplayer.dll win32-x64, same version
32.0.0.465) only half of it holds -- measured, not assumed:

                          Linux x86-64           Windows x86-64
    String   vtable       +0x00                  +0x00        same
             buffer       +0x08                  +0x08        same
             length       +0x30                  +0x30        same
    ScriptObject table    +0x30                  +0x30        same
    table    capacity     +0x08                  +0x08        same
             entries      +0x18                  +0x48        DIFFERENT
             stride       16 bytes               24 bytes     DIFFERENT
             entry        (value, key)           (value, _, key)

In other words: the objects look alike, but a table entry is 24 bytes and not
16, and the key is the third qword and not the second. A port that copied the
Linux offsets would have read values shifted by one field -- plausible, and
wrong.

So this module hard codes none of these offsets. It starts from one certainty:
the SWF is the same everywhere, so a string known from the SWF is interned in
the heap:

    string known from the SWF   e.g. "]=[]8", the obfuscated name of `world`
      -> its UTF-16LE buffer    heap scan
      -> the String object      the qword in front that points into the module
                                is the vtable
      -> the length offset      the qword that holds the length of the string
      -> the slots citing it    scan of the 8 atom encodings
      -> the entry stride       measured on the neighbouring keys
      -> the table base         first qword pointing into the module, before
                                the entries
      -> the value offset       a vote: the offset where all atoms are valid

Atom encoding, measured on this process:

    tag 0   signed integer        value = atom >> 3, arithmetic
    tag 1   float                 pointer to an 8 byte IEEE double
    tag 2   special               0x0a null/undefined, 0x12 false, 0x32 true
    tag 5   String                pointer to a String object
    tag 6   object                pointer to a ScriptObject
    tag 3   native object / MovieClip
"""
import struct

STRIDE_CANDS = tuple(range(0x08, 0x41, 0x08))
VAL_DELTA_CANDS = (-0x10, -0x08, 0x08, 0x10)
STR_BUF_CANDS = (0x08, 0x10, 0x18, 0x20, 0x00)
STR_LEN_CANDS = tuple(range(0x08, 0x80, 8))
SO_TBL_CANDS = tuple(range(0x08, 0x80, 8))
TBL_CAP_CANDS = (0x08, 0x10, 0x18)

MAX_TABLE_CAP = 1 << 16
MAX_STRING_LEN = 512

TAG_INT, TAG_DOUBLE, TAG_SPECIAL, TAG_NATIVE, TAG_STRING, TAG_OBJECT = 0, 1, 2, 3, 5, 6
ATOM_NULL, ATOM_FALSE, ATOM_TRUE = 0x0A, 0x12, 0x32


class Layout:
    """The offsets found for this process."""

    def __init__(self, module_lo=0):
        self.module_lo = module_lo
        self.str_vt = self.str_buf = self.str_len = None
        self.tbl_vt = self.tbl_cap = self.tbl_keys = None
        self.tbl_stride = self.tbl_value = None
        self.so_vt = self.so_tbl = None

    def complete(self):
        return None not in (self.str_vt, self.str_buf, self.str_len,
                            self.tbl_vt, self.tbl_cap, self.tbl_keys,
                            self.tbl_stride, self.tbl_value, self.so_tbl)

    def _rel(self, v):
        return "MODULE+0x%x" % (v - self.module_lo) if v else "?"

    def __str__(self):
        return (
            "  String        vtable %-18s buffer +0x%02x   length +0x%02x\n"
            "  ScriptObject  vtable %-18s table  +0x%02x\n"
            "  table         vtable %-18s capacity +0x%02x  entries +0x%02x\n"
            "  entry         %d bytes, key at +0x00, value at %+#04x"
            % (self._rel(self.str_vt), self.str_buf, self.str_len,
               self._rel(self.so_vt), self.so_tbl,
               self._rel(self.tbl_vt), self.tbl_cap, self.tbl_keys,
               self.tbl_stride, self.tbl_value)
        )


class Avm1:
    """An AVM1 view of a process, once the layout is derived."""

    def __init__(self, proc, module, heaps=None):
        self.p = proc
        self.lo, self.hi = module
        self.heaps = proc.regions() if heaps is None else heaps
        self.L = Layout(module[0])
        self._key_atoms = {}

    # -- primitives --------------------------------------------------------
    def in_module(self, v):
        return v is not None and self.lo <= v < self.hi

    def string_at(self, addr):
        """Decodes the String object at `addr`, or None if it is not one."""
        L = self.L
        if addr is None or self.p.u64(addr) != L.str_vt:
            return None
        buf, n = self.p.u64(addr + L.str_buf), self.p.u64(addr + L.str_len)
        if not buf or n is None or n > MAX_STRING_LEN:
            return None
        b = self.p.read(buf, n * 2)
        if not b or len(b) != n * 2:
            return None
        try:
            return b.decode("utf-16-le")
        except UnicodeDecodeError:
            return None

    def key_at(self, addr):
        """The name of the key stored at `addr`, or None."""
        return self.string_at((self.p.u64(addr) or 0) & ~7)

    # -- atom decoding -----------------------------------------------------
    @staticmethod
    def tag(atom):
        return None if atom is None else atom & 7

    @staticmethod
    def as_int(atom, bound=1 << 31):
        """Atom -> signed integer, or None if it is not a plausible one."""
        if atom is None or atom & 7:
            return None
        v = atom - (1 << 64) if atom >> 63 else atom
        v >>= 3                      # arithmetic shift: negatives work
        return v if -bound < v < bound else None

    def as_double(self, atom):
        if atom is None or atom & 7 != TAG_DOUBLE:
            return None
        b = self.p.read(atom & ~7, 8)
        return struct.unpack("<d", b)[0] if b and len(b) == 8 else None

    def as_number(self, atom):
        """Integer or float, depending on the tag."""
        d = self.as_double(atom)
        return d if d is not None else self.as_int(atom)

    @staticmethod
    def as_bool(atom):
        if atom == ATOM_TRUE:
            return True
        if atom == ATOM_FALSE:
            return False
        return None

    def as_string(self, atom):
        if atom is None or atom & 7 not in (TAG_STRING, TAG_INT):
            return None
        return self.string_at(atom & ~7)

    def well_formed(self, atom):
        """Can the atom be read? Used by the vote on the value offset."""
        if atom is None:
            return False
        tag = atom & 7
        if tag == TAG_SPECIAL:
            return atom in (ATOM_NULL, ATOM_FALSE, ATOM_TRUE)
        if tag == TAG_DOUBLE:
            return self.as_double(atom) is not None
        if tag == TAG_STRING:
            return self.string_at(atom & ~7) is not None
        if tag in (TAG_OBJECT, TAG_NATIVE):
            return self.in_module(self.p.u64(atom & ~7))
        if tag == TAG_INT:
            return self.as_int(atom) is not None
        return False

    def describe(self, atom):
        if atom is None:
            return "<unreadable>"
        tag = atom & 7
        if tag == TAG_SPECIAL:
            return {ATOM_NULL: "null", ATOM_FALSE: "false",
                    ATOM_TRUE: "true"}.get(atom, "special 0x%x" % atom)
        if tag == TAG_DOUBLE:
            return "float %r" % self.as_double(atom)
        if tag == TAG_STRING:
            return "String %r" % self.string_at(atom & ~7)
        if tag in (TAG_OBJECT, TAG_NATIVE):
            return "objet 0x%x" % (atom & ~7)
        v = self.as_int(atom)
        return "int %d" % v if v is not None else "0x%x" % atom

    # -- tables ------------------------------------------------------------
    def capacity(self, tbl):
        cap = self.p.u64(tbl + self.L.tbl_cap)
        return cap if cap and 0 < cap <= MAX_TABLE_CAP else None

    def entries(self, tbl):
        """[(key name, value atom, key address)] for the table."""
        cap = self.capacity(tbl)
        if cap is None:
            return []
        L, out = self.L, []
        for i in range(cap):
            k = tbl + L.tbl_keys + i * L.tbl_stride
            name = self.key_at(k)
            if name is not None:
                out.append((name, self.p.u64(k + L.tbl_value), k))
        return out

    def slot(self, tbl, key):
        """The address of the *value* slot of `key`, or None.

        The keys are interned: once the atom is known, the next search is an
        integer comparison, not a string decode.
        """
        cap = self.capacity(tbl)
        if cap is None:
            return None
        L = self.L
        want = self._key_atoms.get(key)
        for i in range(cap):
            k = tbl + L.tbl_keys + i * L.tbl_stride
            raw = self.p.u64(k)
            if not raw:
                continue
            if want is not None:
                if raw == want:
                    return k + L.tbl_value
            elif self.string_at(raw & ~7) == key:
                self._key_atoms[key] = raw
                return k + L.tbl_value
        return None

    def get(self, tbl, key):
        s = self.slot(tbl, key)
        return None if s is None else self.p.u64(s)

    def table_of(self, atom):
        """Object atom -> its property table, or None."""
        if (atom is None or self.L.so_tbl is None
                or atom & 7 not in (TAG_OBJECT, TAG_NATIVE, TAG_INT)):
            return None
        so = atom & ~7
        if not self.in_module(self.p.u64(so)):
            return None
        t = self.p.u64(so + self.L.so_tbl)
        return t if t and self.p.u64(t) == self.L.tbl_vt else None

    def child(self, tbl, key):
        """Shortcut: the table of the object stored under `key`."""
        return self.table_of(self.get(tbl, key))

    # -- layout derivation -------------------------------------------------
    def derive(self, anchor, verbose=False):
        """Finds every offset from one key known from the SWF.

        -> the list of tables that own this key.
        """
        log = print if verbose else (lambda *_: None)
        strobj = self._derive_string(anchor)
        if strobj is None:
            log("  string %r not found in the heap" % anchor)
            return []
        log("  String %r at 0x%x: vtable %s, buffer +0x%02x, length +0x%02x"
            % (anchor, strobj, self.L._rel(self.L.str_vt),
               self.L.str_buf, self.L.str_len))

        tables = []
        for ks in self.p.scan_tagged(strobj, regions=self.heaps):
            if self.key_at(ks) != anchor:
                continue
            t = self._derive_table(ks)
            if t is not None and t not in tables:
                tables.append(t)
                log("  table 0x%x: capacity %d, %d readable entries"
                    % (t, self.capacity(t) or 0, len(self.entries(t))))
        return tables

    def _derive_string(self, text):
        """Finds the String object of `text` and derives the String layout."""
        pat = text.encode("utf-16-le")
        for buf in self.p.scan(pat, align=2, regions=self.heaps):
            for ref in self.p.scan(struct.pack("<Q", buf), align=8,
                                   regions=self.heaps):
                for ob in STR_BUF_CANDS:
                    so, vt = ref - ob, self.p.u64(ref - ob)
                    if not self.in_module(vt):
                        continue
                    for ol in STR_LEN_CANDS:
                        if self.p.u64(so + ol) != len(text):
                            continue
                        self.L.str_vt, self.L.str_buf, self.L.str_len = vt, ob, ol
                        if self.string_at(so) == text:
                            return so
        return None

    def _derive_table(self, keyslot):
        """Derives stride, base, capacity and value offset from one key.

        The geometry is derived only once. The tables that follow are simply
        located with it. Otherwise the last table examined would force its own
        measurement on all the others.
        """
        if self.L.tbl_stride is not None:
            return self._table_base(keyslot)
        stride = self._measure_stride(keyslot)
        if stride is None:
            return None
        first = keyslot
        while self.key_at(first - stride) is not None:
            first -= stride
        n = 0
        while self.key_at(first + n * stride) is not None:
            n += 1

        for back in range(8, 0x101, 8):
            t = first - back
            if not self.in_module(self.p.u64(t)):
                continue
            for co in TBL_CAP_CANDS:
                cap = self.p.u64(t + co)
                if cap is None or not (n <= cap <= MAX_TABLE_CAP):
                    continue
                self.L.tbl_vt, self.L.tbl_cap = self.p.u64(t), co
                self.L.tbl_keys, self.L.tbl_stride = back, stride
                if self._derive_value_offset(t):
                    return t
        return None

    def _table_base(self, keyslot):
        """The base of the table holding `keyslot`, geometry already known."""
        L = self.L
        first = keyslot
        while self.key_at(first - L.tbl_stride) is not None:
            first -= L.tbl_stride
        t = first - L.tbl_keys
        return t if self.p.u64(t) == L.tbl_vt and self.capacity(t) else None

    def _measure_stride(self, keyslot):
        """The stride is the smallest gap where both neighbours are keys too."""
        for d in STRIDE_CANDS:
            if (self.key_at(keyslot + d) is not None
                    and self.key_at(keyslot + 2 * d) is not None):
                return d
        return None

    def _derive_value_offset(self, tbl):
        """A vote: the right offset is where atoms are valid AND varied.

        Validity alone is not enough. An entry holds an unused qword that is
        always zero, and zero is a perfectly valid integer atom. So that
        column scores perfectly while holding nothing. The diversity test is
        what rejects it, and nothing else.
        """
        best, best_score = None, 0
        for d in VAL_DELTA_CANDS:
            self.L.tbl_value = d
            ents = self.entries(tbl)
            if not ents:
                continue
            vals = [v for _, v, _ in ents]
            diversity = len(set(vals)) / len(vals)
            if diversity < 0.25:
                continue
            score = sum(1 for v in vals if self.well_formed(v)) / len(vals)
            # The column of the next entry holds the same values, shifted by
            # one: it differs by one atom at most, and can win by that atom
            # alone. So a later candidate must win by more than one entry.
            if score > best_score + 1 / len(vals):
                best, best_score = d, score
        self.L.tbl_value = best
        return best is not None and best_score > 0.95

    def derive_script_object(self, atom, expect_key=None):
        """Derives so_vt / so_tbl from an atom known to be an object.

        `expect_key` is a property the target object certainly owns. Without
        it, several offsets can lead to a plausible table, and the wrong one
        would then be cached for every read that follows. So we demand the
        constraint whenever we have one.
        """
        so = (atom or 0) & ~7
        vt = self.p.u64(so)
        if not self.in_module(vt):
            return None
        for x in SO_TBL_CANDS:
            t = self.p.u64(so + x)
            if not t or self.p.u64(t) != self.L.tbl_vt or not self.capacity(t):
                continue
            if expect_key is not None and self.slot(t, expect_key) is None:
                continue
            self.L.so_vt, self.L.so_tbl = vt, x
            return t
        return None
