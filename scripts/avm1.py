"""Modele objet AVM1 dans un process Pepper Flash, layout *derive* a l'execution.

Le reverse precedent (Linux x86-64, libpepflashplayer.so 32.0.0.465) avait
prouve ce layout. Sous Windows (pepflashplayer.dll win32-x64, meme version
32.0.0.465) il n'est vrai qu'a moitie -- mesure, pas suppose :

                          Linux x86-64           Windows x86-64
    String   vtable       +0x00                  +0x00        identique
             buffer       +0x08                  +0x08        identique
             longueur     +0x30                  +0x30        identique
    ScriptObject table    +0x30                  +0x30        identique
    table    capacite     +0x08                  +0x08        identique
             entrees      +0x18                  +0x48        DIFFERENT
             pas          16 octets              24 octets    DIFFERENT
             entree       (valeur, clef)         (valeur, _, clef)

Autrement dit : les objets se ressemblent, mais une entree de table fait 24
octets et non 16, et la clef est le troisieme qword et non le second. Un port
qui aurait recopie les offsets Linux aurait lu des valeurs decalees d'un champ
-- plausibles, et fausses.

Ce module ne code donc aucun de ces offsets en dur. Il part d'une seule
certitude, le SWF est le meme partout, donc une chaine connue du SWF est
internee dans le tas :

    chaine connue du SWF        ex. "]=[]8", le nom obfusque de `world`
      -> son buffer UTF-16LE    scan du tas
      -> l'objet String         le qword module-pointant devant = la vtable
      -> l'offset longueur      le qword valant la longueur de la chaine
      -> les slots qui la citent scan des 8 encodages d'atome
      -> le pas des entrees     mesure sur les clefs voisines
      -> la base de la table    premier qword module-pointant avant les entrees
      -> l'offset des valeurs   vote : l'offset ou tous les atomes sont valides

Encodage des atomes, mesure sur ce process :

    tag 0   entier signe          valeur = atome >> 3, arithmetique
    tag 1   flottant              pointeur vers un double IEEE 8 octets
    tag 2   special               0x0a null/undefined, 0x12 faux, 0x32 vrai
    tag 5   String                pointeur vers un objet String
    tag 6   objet                 pointeur vers un ScriptObject
    tag 3   objet natif / MovieClip
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
    """Les offsets retrouves pour ce process."""

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
            "  String        vtable %-18s buffer +0x%02x   longueur +0x%02x\n"
            "  ScriptObject  vtable %-18s table  +0x%02x\n"
            "  table         vtable %-18s capacite +0x%02x  entrees +0x%02x\n"
            "  entree        %d octets, clef a +0x00, valeur a %+#04x"
            % (self._rel(self.str_vt), self.str_buf, self.str_len,
               self._rel(self.so_vt), self.so_tbl,
               self._rel(self.tbl_vt), self.tbl_cap, self.tbl_keys,
               self.tbl_stride, self.tbl_value)
        )


class Avm1:
    """Vue AVM1 d'un process, une fois le layout derive."""

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
        """Decode l'objet String a `addr`, ou None si ce n'en est pas un."""
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
        """Nom de la clef stockee a `addr`, ou None."""
        return self.string_at((self.p.u64(addr) or 0) & ~7)

    # -- decodage des atomes ----------------------------------------------
    @staticmethod
    def tag(atom):
        return None if atom is None else atom & 7

    @staticmethod
    def as_int(atom, bound=1 << 31):
        """Atome -> entier signe, ou None si ce n'est pas un entier plausible."""
        if atom is None or atom & 7:
            return None
        v = atom - (1 << 64) if atom >> 63 else atom
        v >>= 3                      # decalage arithmetique: les negatifs marchent
        return v if -bound < v < bound else None

    def as_double(self, atom):
        if atom is None or atom & 7 != TAG_DOUBLE:
            return None
        b = self.p.read(atom & ~7, 8)
        return struct.unpack("<d", b)[0] if b and len(b) == 8 else None

    def as_number(self, atom):
        """Entier ou flottant, selon le tag."""
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
        """L'atome est-il interpretable ? Sert au vote sur l'offset valeur."""
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
            return "<illisible>"
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
        """[(nom de clef, atome valeur, adresse de la clef)] pour la table."""
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
        """Adresse du slot *valeur* de `key`, ou None.

        Les clefs sont internees : une fois l'atome connu, la recherche
        suivante est une comparaison d'entiers, pas un decodage de chaine.
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
        """Atome objet -> sa table de proprietes, ou None."""
        if (atom is None or self.L.so_tbl is None
                or atom & 7 not in (TAG_OBJECT, TAG_NATIVE, TAG_INT)):
            return None
        so = atom & ~7
        if not self.in_module(self.p.u64(so)):
            return None
        t = self.p.u64(so + self.L.so_tbl)
        return t if t and self.p.u64(t) == self.L.tbl_vt else None

    def child(self, tbl, key):
        """Raccourci : table de l'objet stocke sous `key`."""
        return self.table_of(self.get(tbl, key))

    # -- derivation du layout ---------------------------------------------
    def derive(self, anchor, verbose=False):
        """Retrouve tous les offsets a partir d'une clef connue du SWF.

        -> liste des tables possedant cette clef.
        """
        log = print if verbose else (lambda *_: None)
        strobj = self._derive_string(anchor)
        if strobj is None:
            log("  chaine %r introuvable dans le tas" % anchor)
            return []
        log("  String %r a 0x%x : vtable %s, buffer +0x%02x, longueur +0x%02x"
            % (anchor, strobj, self.L._rel(self.L.str_vt),
               self.L.str_buf, self.L.str_len))

        tables = []
        for ks in self.p.scan_tagged(strobj, regions=self.heaps):
            if self.key_at(ks) != anchor:
                continue
            t = self._derive_table(ks)
            if t is not None and t not in tables:
                tables.append(t)
                log("  table 0x%x : capacite %d, %d entrees lisibles"
                    % (t, self.capacity(t) or 0, len(self.entries(t))))
        return tables

    def _derive_string(self, text):
        """Trouve l'objet String de `text` et en deduit le layout String."""
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
        """Deduit pas, base, capacite et offset des valeurs depuis une clef.

        La geometrie n'est deduite qu'une fois : les tables suivantes sont
        simplement localisees avec, sinon la derniere table examinee
        imposerait sa mesure a toutes les autres.
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
        """Base de la table contenant `keyslot`, geometrie deja connue."""
        L = self.L
        first = keyslot
        while self.key_at(first - L.tbl_stride) is not None:
            first -= L.tbl_stride
        t = first - L.tbl_keys
        return t if self.p.u64(t) == L.tbl_vt and self.capacity(t) else None

    def _measure_stride(self, keyslot):
        """Le pas est l'ecart minimal ou les deux voisins sont aussi des clefs."""
        for d in STRIDE_CANDS:
            if (self.key_at(keyslot + d) is not None
                    and self.key_at(keyslot + 2 * d) is not None):
                return d
        return None

    def _derive_value_offset(self, tbl):
        """Vote : le bon offset est celui ou les atomes sont valides ET varies.

        La validite seule ne suffit pas. Une entree contient un qword inutilise
        toujours nul, et zero est un atome entier parfaitement valide : cette
        colonne-la obtient donc un score parfait sans rien contenir. C'est le
        critere de diversite qui l'elimine, et lui seul.
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
            if score > best_score:
                best, best_score = d, score
        self.L.tbl_value = best
        return best is not None and best_score > 0.95

    def derive_script_object(self, atom, expect_key=None):
        """Deduit so_vt / so_tbl depuis un atome connu pour etre un objet.

        `expect_key` est une propriete que l'objet vise possede a coup sur.
        Sans elle plusieurs offsets peuvent mener a une table plausible, et
        le mauvais serait alors mis en cache pour toutes les lectures
        suivantes -- donc on exige la contrainte quand on en a une.
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
