//! Decodage des atomes AVM1.
//!
//! Un atome vaut `(valeur << 3) | tag`, les trois bits bas donnant le type.
//! Valeurs mesurees sur pepflashplayer.dll 32.0.0.465 (voir
//! `hammerfest-level-re.md`).

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

/// Pointeur porte par un atome, tag retire.
#[inline]
pub fn ptr(atom: u64) -> u64 {
    atom & !7
}

/// Atome -> entier **signe**.
///
/// Le decalage doit etre arithmetique : `portalId` vaut -1, encode
/// `0xfffffffffffffff8`. Un decalage logique en ferait 2305843009213693951.
#[inline]
pub fn as_int(atom: u64) -> Option<i64> {
    if tag(atom) != TAG_INT {
        return None;
    }
    Some((atom as i64) >> 3)
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
    fn decode_les_entiers_positifs() {
        assert_eq!(as_int(9 << 3), Some(9));
        assert_eq!(as_int(0), Some(0));
    }

    #[test]
    fn decode_les_entiers_negatifs() {
        // `portalId` = -1, tel qu'observe dans la table GameMode.
        assert_eq!(as_int(0xffff_ffff_ffff_fff8), Some(-1));
        assert_eq!(as_int((-42i64 as u64) << 3 & !7), Some(-42));
    }

    #[test]
    fn refuse_ce_qui_n_est_pas_un_entier() {
        for atom in [TRUE, FALSE, NULL, 0x4d6381a94dd] {
            assert_eq!(as_int(atom), None, "atome {atom:#x}");
        }
    }

    #[test]
    fn decode_les_booleens() {
        assert_eq!(as_bool(TRUE), Some(true));
        assert_eq!(as_bool(FALSE), Some(false));
        // null n'est ni vrai ni faux : le confondre avec faux ferait passer
        // une partie en pause pour une partie en cours.
        assert_eq!(as_bool(NULL), None);
        assert_eq!(as_bool(0), None);
    }

    #[test]
    fn separe_les_tags_des_pointeurs() {
        let atom = 0x4d6381a94d8 | TAG_OBJECT;
        assert_eq!(tag(atom), TAG_OBJECT);
        assert_eq!(ptr(atom), 0x4d6381a94d8);
    }
}
