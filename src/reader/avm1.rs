//! The AVM1 object model inside a Pepper Flash process.
//!
//! See `docs/reverse-engineering.md` for the measurements. In short: String
//! and ScriptObject have the same layout on Linux and on Windows, but
//! property tables do not.
//!
//! ```text
//!                   Linux x86-64        Windows x86-64
//!   entries         tbl+0x18            tbl+0x48
//!   stride          16 bytes            24 bytes
//!   entry           (value, key)        (value, _, key)
//! ```
//!
//! Both forms are tried, and a semantic check decides between them: a profile
//! that does not lead to a known Hammerfest world is rejected. On failure we
//! return nothing. We never return a wrong level.
//!
//! The vtables are not hard coded. They are found at run time, from a string
//! we know the SWF contains.

// Atom decoding lives in the core, where it is tested.
use hammerfest_core::atom;
pub use hammerfest_core::atom::{as_bool, as_int};

/// Everything the reader needs from a process.
///
/// One method, because the reader makes exactly four kinds of raw read and
/// all four are "give me these bytes". The contract is ours, so a test double
/// implements it without borrowing a word of the runtime ABI.
///
/// `read_into` fills `buf` whole, or returns `None`. It never fills a part of
/// it. The caller then has no partially true buffer to mistake for a value,
/// which is what `reader::refuses-rather-than-defaults` asks for.
///
/// `tests/contract/memory.rs` states that in four questions. The test heap and
/// the production adapter, `ProcessMemory`, both answer them.
///
/// @spec reader::refuses-rather-than-defaults
pub trait Memory {
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()>;
}

/// Atom -> number, integer or float.
///
/// `duration` is the integer 0 when the GameMode is built, then becomes a
/// float on the first frame played. Both forms are normal. Reading only one of
/// them would mean reading nothing during the black screen.
pub fn as_number(mem: &dyn Memory, atom: u64) -> Option<f64> {
    match atom::double_at(atom) {
        Some(addr) => read_u64(mem, addr).map(atom::decode_double),
        None => as_int(atom).map(|v| v as f64),
    }
}

/// The width of a pointer, of a length and of an atom in a build of Flash
/// Player.
///
/// The object model is the same on both widths. Measured on the Windows
/// projector 32.0.0.465, every offset of a 32-bit build is the one of Pepper
/// Flash under Linux counted in words of four bytes instead of eight.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Word {
    /// A 32-bit build: the Windows projector.
    Four,
    /// A 64-bit build: Pepper Flash, and the projector under Linux.
    #[default]
    Eight,
}

impl Word {
    pub const fn bytes(self) -> u64 {
        match self {
            Word::Four => 4,
            Word::Eight => 8,
        }
    }

    /// The word at `addr`: a pointer, a length or a capacity.
    #[inline]
    pub fn read(self, mem: &dyn Memory, addr: u64) -> Option<u64> {
        match self {
            Word::Eight => read_u64(mem, addr),
            Word::Four => read_u32(mem, addr),
        }
    }

    /// The atom at `addr`, in the 64-bit form the core decodes.
    ///
    /// An integer keeps its sign: `portalId` is -1, `0xfffffff8` in four
    /// bytes.
    ///
    /// @spec projector::words-of-four-bytes
    #[inline]
    pub fn atom(self, mem: &dyn Memory, addr: u64) -> Option<u64> {
        let raw = self.read(mem, addr)?;
        let is_short_integer = self == Word::Four && atom::tag(raw) == atom::TAG_INT;
        Some(if is_short_integer {
            raw as u32 as i32 as i64 as u64
        } else {
            raw
        })
    }

    /// The word at `offset` in a buffer already read, if it fits there.
    pub fn in_buffer(self, buf: &[u8], offset: usize) -> Option<u64> {
        crate::scan::word_at(buf, offset, self.bytes() as usize)
    }

    /// The table geometries a build of this width may use.
    pub fn profiles(self) -> &'static [Profile] {
        match self {
            Word::Four => PROFILES_32_BITS,
            Word::Eight => PROFILES,
        }
    }

    /// Candidate offsets for the buffer pointer of a String object.
    pub fn string_buffer_candidates(self) -> [u64; 5] {
        let word = self.bytes();
        [word, 2 * word, 3 * word, 4 * word, 0]
    }

    /// Candidate offsets for `ScriptObject -> table`, from one word to
    /// fifteen. Six words first: it is the offset on every known build, but
    /// we still derive it.
    fn table_pointer_candidates(self) -> [u64; 15] {
        const MEASURED_WORDS: u64 = 6;
        let word = self.bytes();
        let mut candidates = [MEASURED_WORDS * word; 15];
        let others = (1..=15).filter(|&words| words != MEASURED_WORDS);
        for (candidate, words) in candidates[1..].iter_mut().zip(others) {
            *candidate = words * word;
        }
        candidates
    }
}
/// Entries searched back from a key, looking for the header of its table.
///
/// The header is `i * stride + keys` bytes in front of entry `i`, and `i` is
/// unknown. Measured on a running game, the anchor keys sat at entries 2, 61,
/// 70, 97 and 570 of their tables, the largest of which held 793 entries.
///
/// The rule used to be "walk back while the previous qword decodes as a
/// string, then subtract the key offset". It held on Windows and it does not
/// hold on macOS: the heap there packs String pointers on both sides of a
/// table, so the walk ran up to 95 entries past the base and never landed on
/// a header.
///
/// It does not hold on Linux either, for the opposite reason: a table there
/// can hold a key that is not a String object. Measured on a running game, the
/// entry in front of `world` held one, so the walk stopped short of the base.
const MAX_BACK: u64 = 2048;
/// Bytes read at a time while searching back.
const BACK_CHUNK: usize = 8192;
const MAX_CAPACITY: u64 = 1 << 16;
/// Maximum length of a decoded key. Obfuscated names are 2 to 8 characters.
const MAX_KEY: usize = 64;

/// The geometry of a property table.
#[derive(Copy, Clone, Debug)]
pub struct Profile {
    pub name: &'static str,
    /// Offset of the first key from the base of the table.
    pub keys: u64,
    /// Size of one entry.
    pub stride: u64,
    /// Position of the value, relative to the key.
    pub value: i64,
}

pub const PROFILES: &[Profile] = &[
    Profile {
        name: "windows-x64",
        keys: 0x58,
        stride: 24,
        value: -0x10,
    },
    Profile {
        name: "linux-x64",
        keys: 0x20,
        stride: 16,
        value: -0x08,
    },
];

/// The geometry of a property table on the Windows projector, measured under
/// Wine: the one of `linux-x64`, in words of four bytes.
pub const PROFILES_32_BITS: &[Profile] = &[Profile {
    name: "windows-x86",
    keys: 0x10,
    stride: 8,
    value: -0x04,
}];

#[derive(Copy, Clone, Debug)]
pub struct Layout {
    pub module: (u64, u64),
    pub str_vt: u64,
    pub str_buf: u64,
    pub str_len: u64,
    pub tbl_vt: u64,
    pub profile: Profile,
    pub so_tbl: u64,
    pub word: Word,
}

/// Both the Flash player and the host are x86-64, so the bytes are little
/// endian on each side of this call.
#[inline]
pub fn read_u64(mem: &dyn Memory, addr: u64) -> Option<u64> {
    if addr == 0 || addr >= 1 << 47 {
        return None;
    }
    let mut bytes = [0u8; 8];
    mem.read_into(addr, &mut bytes)?;
    Some(u64::from_le_bytes(bytes))
}

/// A 32-bit word, for a 32-bit build.
#[inline]
fn read_u32(mem: &dyn Memory, addr: u64) -> Option<u64> {
    if addr == 0 || addr >= 1 << 47 {
        return None;
    }
    let mut bytes = [0u8; 4];
    mem.read_into(addr, &mut bytes)?;
    Some(u64::from(u32::from_le_bytes(bytes)))
}

impl Layout {
    #[inline]
    pub fn in_module(&self, v: u64) -> bool {
        v >= self.module.0 && v < self.module.1
    }

    /// Reads a String object into `out`, and returns the number of UTF-16 units.
    pub fn read_string(
        &self,
        mem: &dyn Memory,
        addr: u64,
        out: &mut [u16; MAX_KEY],
    ) -> Option<usize> {
        if self.word.read(mem, addr)? != self.str_vt {
            return None;
        }
        let buf = self.word.read(mem, addr + self.str_buf)?;
        let n = self.word.read(mem, addr + self.str_len)? as usize;
        if n == 0 || n > MAX_KEY {
            return None;
        }
        let mut bytes = [0u8; MAX_KEY * 2];
        let bytes = &mut bytes[..n * 2];
        mem.read_into(buf, bytes)?;
        let (pairs, _) = bytes.as_chunks::<2>();
        for (unit, pair) in out[..n].iter_mut().zip(pairs) {
            *unit = u16::from_le_bytes(*pair);
        }
        Some(n)
    }

    /// Is the string at `addr` exactly `want`?
    ///
    /// The comparison allocates nothing. The keys are known at compile time,
    /// so we encode `want` to UTF-16 as we go.
    pub fn string_eq(&self, mem: &dyn Memory, addr: u64, want: &str) -> bool {
        let mut buf = [0u16; MAX_KEY];
        let Some(n) = self.read_string(mem, addr, &mut buf) else {
            return false;
        };
        let mut it = want.encode_utf16();
        for &c in &buf[..n] {
            if it.next() != Some(c) {
                return false;
            }
        }
        it.next().is_none()
    }

    // -- tables ------------------------------------------------------------
    pub fn capacity(&self, mem: &dyn Memory, tbl: u64) -> Option<u64> {
        let cap = self.word.read(mem, tbl + self.capacity_offset())?;
        (cap > 0 && cap <= MAX_CAPACITY).then_some(cap)
    }

    /// Where a table keeps its capacity: right after its vtable.
    #[inline]
    fn capacity_offset(&self) -> u64 {
        self.word.bytes()
    }

    /// Address of key number `i` in the table.
    #[inline]
    pub fn key_addr(&self, tbl: u64, i: u64) -> u64 {
        tbl + self.profile.keys + i * self.profile.stride
    }

    /// The name of the key stored at `addr`. It masks the tag, because a key
    /// can be stored as a raw pointer or as an atom, depending on the
    /// platform.
    pub fn key_is(&self, mem: &dyn Memory, addr: u64, want: &str) -> bool {
        match self.word.read(mem, addr) {
            Some(raw) => self.string_eq(mem, raw & !7, want),
            None => false,
        }
    }

    /// The atom of property `key`, or None.
    pub fn get(&self, mem: &dyn Memory, tbl: u64, key: &str) -> Option<u64> {
        let mut ignored = 0;
        self.get_cached(mem, tbl, key, &mut ignored)
    }

    /// Like `get`, but it remembers the index of the entry.
    ///
    /// Walking the 128 entries of a table on every read would cost hundreds of
    /// memory accesses per tick. So we keep the index -- but we check it again
    /// before use: if the key is no longer there, we search again. The thing
    /// we must never keep is the *final* address, not the path.
    pub fn get_cached(&self, mem: &dyn Memory, tbl: u64, key: &str, hint: &mut u64) -> Option<u64> {
        let cap = self.capacity(mem, tbl)?;
        let value_at = |k: u64| self.word.atom(mem, (k as i64 + self.profile.value) as u64);

        if *hint < cap {
            let k = self.key_addr(tbl, *hint);
            if self.key_is(mem, k, key) {
                return value_at(k);
            }
        }
        for i in 0..cap {
            let k = self.key_addr(tbl, i);
            if self.key_is(mem, k, key) {
                *hint = i;
                return value_at(k);
            }
        }
        None
    }

    pub fn get_int(&self, mem: &dyn Memory, tbl: u64, key: &str) -> Option<i64> {
        as_int(self.get(mem, tbl, key)?)
    }

    /// The property table of the object that `atom` points to.
    pub fn table_of(&self, mem: &dyn Memory, atom: u64) -> Option<u64> {
        let so = atom & !7;
        if !self.in_module(self.word.read(mem, so)?) {
            return None;
        }
        let t = self.word.read(mem, so + self.so_tbl)?;
        (self.word.read(mem, t)? == self.tbl_vt).then_some(t)
    }

    /// The table of the object stored under `key`.
    pub fn child(&self, mem: &dyn Memory, tbl: u64, key: &str) -> Option<u64> {
        self.table_of(mem, self.get(mem, tbl, key)?)
    }

    pub fn child_cached(
        &self,
        mem: &dyn Memory,
        tbl: u64,
        key: &str,
        hint: &mut u64,
    ) -> Option<u64> {
        self.table_of(mem, self.get_cached(mem, tbl, key, hint)?)
    }

    /// Finds `so_tbl` from an object whose property we know.
    ///
    /// Without the `expect_key` constraint, several offsets lead to a
    /// plausible table, and the wrong one would be cached for every read that
    /// follows.
    pub fn derive_so_tbl(&mut self, mem: &dyn Memory, atom: u64, expect_key: &str) -> Option<u64> {
        let so = atom & !7;
        if !self.in_module(self.word.read(mem, so)?) {
            return None;
        }
        for off in self.word.table_pointer_candidates() {
            let Some(t) = self.word.read(mem, so + off) else {
                continue;
            };
            if self.word.read(mem, t) != Some(self.tbl_vt) {
                continue;
            }
            if self.capacity(mem, t).is_none() {
                continue;
            }
            self.so_tbl = off;
            if self.get(mem, t, expect_key).is_some() {
                return Some(t);
            }
        }
        None
    }

    /// The base of the table that contains `keyslot`, for the current profile.
    ///
    /// We walk back while the previous qword decodes as a string, then we
    /// subtract the key offset. A wrong profile gives a base whose vtable is
    /// not a vtable, so it rejects itself.
    ///
    /// If `tbl_vt` is unknown (zero), any pointer into the module will do and
    /// becomes the reference vtable. That is how it is derived instead of hard
    /// coded.
    pub fn table_base(&mut self, mem: &dyn Memory, keyslot: u64) -> Option<u64> {
        let stride = self.profile.stride;
        let mut buf = alloc::vec![0u8; BACK_CHUNK + 8];
        let mut i = 0u64;

        while i < MAX_BACK {
            // The headers of the candidates in this chunk, from the nearest
            // one down. One read covers them all.
            let n = (BACK_CHUNK as u64 / stride).min(MAX_BACK - i);
            let hi = keyslot.checked_sub(i * stride + self.profile.keys)?;
            let lo = hi.checked_sub((n - 1) * stride)?;
            let span = (hi - lo) as usize + 16;
            let chunk = mem.read_into(lo, &mut buf[..span]).map(|_| &buf[..span]);

            for k in 0..n {
                let tbl = hi - k * stride;
                // A chunk that would not read is served one address at a
                // time. A range near the edge of a region refuses as a whole,
                // and the header we want can sit inside it.
                let vt = match chunk {
                    Some(bytes) => self.word.in_buffer(bytes, (tbl - lo) as usize)?,
                    None => match self.word.read(mem, tbl) {
                        Some(vt) => vt,
                        None => continue,
                    },
                };
                if self.tbl_vt == 0 {
                    if !self.in_module(vt) {
                        continue;
                    }
                } else if vt != self.tbl_vt {
                    continue;
                }
                // A table holds our key, so its capacity must cover its index.
                let cap = match chunk {
                    Some(bytes) => self
                        .word
                        .in_buffer(bytes, (tbl - lo + self.capacity_offset()) as usize)
                        .filter(|&c| c > 0 && c <= MAX_CAPACITY),
                    None => self.capacity(mem, tbl),
                };
                let Some(cap) = cap else { continue };
                if i + k >= cap {
                    continue;
                }
                self.tbl_vt = vt;
                return Some(tbl);
            }
            i += n;
        }
        None
    }
}
