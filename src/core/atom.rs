//! AVM1 atom decoding.
//!
//! An atom is `(value << 3) | tag`. The three low bits give the type. The
//! values below were measured on pepflashplayer.dll 32.0.0.465. See
//! `docs/reverse-engineering.md`.

pub const TAG_INT: u64 = 0;
pub const TAG_DOUBLE: u64 = 1;
pub const TAG_SPECIAL: u64 = 2;
pub const TAG_NATIVE: u64 = 3;
pub const TAG_STRING: u64 = 5;
pub const TAG_OBJECT: u64 = 6;

pub const NULL: u64 = 0x0A;
pub const FALSE: u64 = 0x12;
pub const TRUE: u64 = 0x32;

#[inline]
pub fn tag(atom: u64) -> u64 {
    atom & 7
}

/// The pointer an atom carries, with the tag removed.
#[inline]
pub fn ptr(atom: u64) -> u64 {
    atom & !7
}

/// Atom -> **signed** integer.
///
/// The shift must be arithmetic. `portalId` is -1, encoded as
/// `0xfffffffffffffff8`. A logical shift would turn it into
/// 2305843009213693951.
#[inline]
pub fn as_int(atom: u64) -> Option<i64> {
    if tag(atom) != TAG_INT {
        return None;
    }
    Some((atom as i64) >> 3)
}

/// Float atom -> the address of the eight bytes of the double.
///
/// A float does not fit inside the atom, unlike an integer. The atom points to
/// the value. The core reads no memory, so it returns the address and
/// [`decode_double`] does the rest.
#[inline]
pub fn double_at(atom: u64) -> Option<u64> {
    (tag(atom) == TAG_DOUBLE).then(|| ptr(atom))
}

/// The eight bytes read at that address, as a float.
#[inline]
pub fn decode_double(raw: u64) -> f64 {
    f64::from_bits(raw)
}

#[inline]
pub fn as_bool(atom: u64) -> Option<bool> {
    match atom {
        TRUE => Some(true),
        FALSE => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_positive_integers() {
        assert_eq!(as_int(9 << 3), Some(9));
        assert_eq!(as_int(0), Some(0));
    }

    #[test]
    fn decodes_negative_integers() {
        // `portalId` = -1, as observed in the GameMode table.
        assert_eq!(as_int(0xffff_ffff_ffff_fff8), Some(-1));
        assert_eq!(as_int((-42i64 as u64) << 3 & !7), Some(-42));
    }

    #[test]
    fn rejects_what_is_not_an_integer() {
        for atom in [TRUE, FALSE, NULL, 0x4d6381a94dd] {
            assert_eq!(as_int(atom), None, "atom {atom:#x}");
        }
    }

    #[test]
    fn decodes_booleans() {
        assert_eq!(as_bool(TRUE), Some(true));
        assert_eq!(as_bool(FALSE), Some(false));
        // null is neither true nor false. If we read it as false, a paused
        // game would look like a running game.
        assert_eq!(as_bool(NULL), None);
        assert_eq!(as_bool(0), None);
    }

    #[test]
    fn decodes_floats() {
        // `duration` as read at the end of a game: 14815.6 cycles.
        let raw = 14815.6f64.to_bits();
        assert_eq!(decode_double(raw), 14815.6);

        let atom = 0x4d6381a94d8 | TAG_DOUBLE;
        assert_eq!(double_at(atom), Some(0x4d6381a94d8));
        // An integer is not a float. If we read it as one, we would read eight
        // bytes at an address that is not an address.
        assert_eq!(double_at(9 << 3), None);
        assert_eq!(double_at(TRUE), None);
    }

    #[test]
    fn separates_tags_from_pointers() {
        let atom = 0x4d6381a94d8 | TAG_OBJECT;
        assert_eq!(tag(atom), TAG_OBJECT);
        assert_eq!(ptr(atom), 0x4d6381a94d8);
    }
}
