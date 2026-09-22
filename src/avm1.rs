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

use asr::{Address, Process};

// Atom decoding lives in the core, where it is tested.
pub use hammerfest_core::atom::{as_bool, as_int};
use hammerfest_core::atom;

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
/// `crate::memory_contract` states that in four tests and runs them against
/// every implementation, so the test heap and the adapter below cannot drift
/// apart.
pub trait Memory {
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()>;
}

impl Memory for Process {
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()> {
        self.read_into_slice(Address::new(address), buf).ok()
    }
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

/// Table capacity, at `tbl + 0x08` on both platforms.
const TBL_CAPACITY: u64 = 0x08;
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
    Profile { name: "windows-x64", keys: 0x58, stride: 24, value: -0x10 },
    Profile { name: "linux-x64", keys: 0x20, stride: 16, value: -0x08 },
];

/// Candidate offsets for `ScriptObject -> table`. The value is 0x30 on both
/// known platforms, but we still derive it.
const SO_TBL_CANDIDATES: [u64; 15] = [
    0x30, 0x08, 0x10, 0x18, 0x20, 0x28, 0x38, 0x40, 0x48, 0x50, 0x58, 0x60,
    0x68, 0x70, 0x78,
];
/// Candidate offsets for the buffer pointer of a String object.
pub const STR_BUF_CANDIDATES: [u64; 5] = [0x08, 0x10, 0x18, 0x20, 0x00];

#[derive(Copy, Clone, Debug)]
pub struct Layout {
    pub module: (u64, u64),
    pub str_vt: u64,
    pub str_buf: u64,
    pub str_len: u64,
    pub tbl_vt: u64,
    pub profile: Profile,
    pub so_tbl: u64,
}

/// Both the Flash player and the host are x86-64, so the bytes are little
/// endian on each side of this call.
#[inline]
pub fn read_u64(mem: &dyn Memory, addr: u64) -> Option<u64> {
    if addr == 0 || addr >= 1 << 47 {
        return None;
    }
    let mut bytes = [0u8; 8];
    let result = mem.read_into(addr, &mut bytes);
    crate::diagnostics::validation_read(8, result.is_some());
    result?;
    Some(u64::from_le_bytes(bytes))
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
        if read_u64(mem, addr)? != self.str_vt {
            return None;
        }
        let buf = read_u64(mem, addr + self.str_buf)?;
        let n = read_u64(mem, addr + self.str_len)? as usize;
        if n == 0 || n > MAX_KEY {
            return None;
        }
        let mut bytes = [0u8; MAX_KEY * 2];
        let bytes = &mut bytes[..n * 2];
        let result = mem.read_into(buf, bytes);
        crate::diagnostics::validation_read(n * 2, result.is_some());
        result?;
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
        let cap = read_u64(mem, tbl + TBL_CAPACITY)?;
        (cap > 0 && cap <= MAX_CAPACITY).then_some(cap)
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
        match read_u64(mem, addr) {
            Some(raw) => self.string_eq(mem, raw & !7, want),
            None => false,
        }
    }

    pub fn key_len(&self, mem: &dyn Memory, addr: u64) -> Option<usize> {
        let raw = read_u64(mem, addr)?;
        let mut buf = [0u16; MAX_KEY];
        self.read_string(mem, raw & !7, &mut buf)
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
    pub fn get_cached(
        &self,
        mem: &dyn Memory,
        tbl: u64,
        key: &str,
        hint: &mut u64,
    ) -> Option<u64> {
        let cap = self.capacity(mem, tbl)?;
        let value_at = |k: u64| read_u64(mem, (k as i64 + self.profile.value) as u64);

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
        if !self.in_module(read_u64(mem, so)?) {
            return None;
        }
        let t = read_u64(mem, so + self.so_tbl)?;
        (read_u64(mem, t)? == self.tbl_vt).then_some(t)
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
        if !self.in_module(read_u64(mem, so)?) {
            return None;
        }
        for off in SO_TBL_CANDIDATES {
            let Some(t) = read_u64(mem, so + off) else {
                continue;
            };
            if read_u64(mem, t) != Some(self.tbl_vt) {
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
        let mut first = keyslot;
        while first > self.profile.stride {
            let prev = first - self.profile.stride;
            if self.key_len(mem, prev).is_none() {
                break;
            }
            first = prev;
        }
        let tbl = first.checked_sub(self.profile.keys)?;
        let vt = read_u64(mem, tbl)?;
        if self.tbl_vt == 0 {
            if !self.in_module(vt) {
                return None;
            }
            self.tbl_vt = vt;
        } else if vt != self.tbl_vt {
            return None;
        }
        self.capacity(mem, tbl)?;
        Some(tbl)
    }
}
