//! Modele objet AVM1 dans un process Pepper Flash.
//!
//! Voir `reverse-engineering.md` pour les mesures. En resume : les objets
//! String et ScriptObject ont le meme layout sous Linux et sous Windows, mais
//! pas les tables de proprietes.
//!
//! ```text
//!                   Linux x86-64        Windows x86-64
//!   entrees         tbl+0x18            tbl+0x48
//!   pas             16 octets           24 octets
//!   entree          (valeur, clef)      (valeur, _, clef)
//! ```
//!
//! Les deux formes sont donc essayees et departagees par validation
//! semantique : un profil qui ne mene pas a un monde Hammerfest connu est
//! rejete. En cas d'echec on ne renvoie rien, jamais un niveau faux.
//!
//! Les vtables, elles, ne sont pas codees en dur : elles sont retrouvees a
//! l'execution a partir d'une chaine connue du SWF.

use asr::{Address, Process};

// Le decodage des atomes vit dans le coeur, ou il est teste.
pub use hammerfest_core::atom::{as_bool, as_int};
use hammerfest_core::atom;

/// Atome -> nombre, entier ou flottant.
///
/// `duration` vaut l'entier 0 a la construction du GameMode, puis devient un
/// flottant des la premiere image jouee. Les deux formes sont donc normales, et
/// n'en lire qu'une reviendrait a ne rien lire pendant l'ecran noir.
pub fn as_number(process: &Process, atom: u64) -> Option<f64> {
    match atom::double_at(atom) {
        Some(addr) => read_u64(process, addr).map(atom::decode_double),
        None => as_int(atom).map(|v| v as f64),
    }
}

/// Capacite d'une table, a `tbl + 0x08` sur les deux plateformes.
const TBL_CAPACITY: u64 = 0x08;
const MAX_CAPACITY: u64 = 1 << 16;
/// Longueur max d'une clef decodee. Les noms obfusques font 2 a 8 caracteres.
const MAX_KEY: usize = 64;

/// Geometrie d'une table de proprietes.
#[derive(Copy, Clone, Debug)]
pub struct Profile {
    pub name: &'static str,
    /// Offset de la premiere clef depuis la base de la table.
    pub keys: u64,
    /// Taille d'une entree.
    pub stride: u64,
    /// Position de la valeur, relative a la clef.
    pub value: i64,
}

pub const PROFILES: &[Profile] = &[
    Profile { name: "windows-x64", keys: 0x58, stride: 24, value: -0x10 },
    Profile { name: "linux-x64", keys: 0x20, stride: 16, value: -0x08 },
];

/// Offsets candidats pour `ScriptObject -> table` (0x30 sur les deux
/// plateformes connues, mais derive tout de meme).
const SO_TBL_CANDIDATES: [u64; 15] = [
    0x30, 0x08, 0x10, 0x18, 0x20, 0x28, 0x38, 0x40, 0x48, 0x50, 0x58, 0x60,
    0x68, 0x70, 0x78,
];
/// Offsets candidats pour le pointeur de buffer d'un objet String.
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

#[inline]
pub fn read_u64(process: &Process, addr: u64) -> Option<u64> {
    if addr == 0 || addr >= 1 << 47 {
        return None;
    }
    let result = process.read::<u64>(Address::new(addr));
    crate::diagnostics::validation_read(8, result.is_ok());
    result.ok()
}

impl Layout {
    #[inline]
    pub fn in_module(&self, v: u64) -> bool {
        v >= self.module.0 && v < self.module.1
    }

    /// Lit un objet String dans `out`, et renvoie le nombre d'unites UTF-16.
    pub fn read_string(
        &self,
        process: &Process,
        addr: u64,
        out: &mut [u16; MAX_KEY],
    ) -> Option<usize> {
        if read_u64(process, addr)? != self.str_vt {
            return None;
        }
        let buf = read_u64(process, addr + self.str_buf)?;
        let n = read_u64(process, addr + self.str_len)? as usize;
        if n == 0 || n > MAX_KEY {
            return None;
        }
        let result = process.read_into_slice(Address::new(buf), &mut out[..n]);
        crate::diagnostics::validation_read(n * 2, result.is_ok());
        result.ok()?;
        Some(n)
    }

    /// La chaine a `addr` est-elle exactement `want` ?
    ///
    /// Comparer sans allouer : les clefs sont connues a la compilation, on
    /// encode `want` en UTF-16 a la volee.
    pub fn string_eq(&self, process: &Process, addr: u64, want: &str) -> bool {
        let mut buf = [0u16; MAX_KEY];
        let Some(n) = self.read_string(process, addr, &mut buf) else {
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
    pub fn capacity(&self, process: &Process, tbl: u64) -> Option<u64> {
        let cap = read_u64(process, tbl + TBL_CAPACITY)?;
        (cap > 0 && cap <= MAX_CAPACITY).then_some(cap)
    }

    /// Adresse de la clef numero `i` de la table.
    #[inline]
    pub fn key_addr(&self, tbl: u64, i: u64) -> u64 {
        tbl + self.profile.keys + i * self.profile.stride
    }

    /// Nom de la clef stockee a `addr` -- masque le tag, une clef pouvant etre
    /// stockee en pointeur brut ou en atome selon la plateforme.
    pub fn key_is(&self, process: &Process, addr: u64, want: &str) -> bool {
        match read_u64(process, addr) {
            Some(raw) => self.string_eq(process, raw & !7, want),
            None => false,
        }
    }

    pub fn key_len(&self, process: &Process, addr: u64) -> Option<usize> {
        let raw = read_u64(process, addr)?;
        let mut buf = [0u16; MAX_KEY];
        self.read_string(process, raw & !7, &mut buf)
    }

    /// Atome de la propriete `key`, ou None.
    pub fn get(&self, process: &Process, tbl: u64, key: &str) -> Option<u64> {
        let mut ignored = 0;
        self.get_cached(process, tbl, key, &mut ignored)
    }

    /// Comme `get`, mais en retenant l'indice de l'entree.
    ///
    /// Parcourir les 128 entrees d'une table a chaque lecture couterait des
    /// centaines d'acces memoire par tick. L'indice est donc memorise -- mais
    /// re-verifie avant usage : si la clef n'est plus la, on rebalaye. C'est
    /// l'adresse *finale* qu'il ne faut jamais garder, pas le chemin.
    pub fn get_cached(
        &self,
        process: &Process,
        tbl: u64,
        key: &str,
        hint: &mut u64,
    ) -> Option<u64> {
        let cap = self.capacity(process, tbl)?;
        let value_at = |k: u64| read_u64(process, (k as i64 + self.profile.value) as u64);

        if *hint < cap {
            let k = self.key_addr(tbl, *hint);
            if self.key_is(process, k, key) {
                return value_at(k);
            }
        }
        for i in 0..cap {
            let k = self.key_addr(tbl, i);
            if self.key_is(process, k, key) {
                *hint = i;
                return value_at(k);
            }
        }
        None
    }

    pub fn get_int(&self, process: &Process, tbl: u64, key: &str) -> Option<i64> {
        as_int(self.get(process, tbl, key)?)
    }

    /// Table de proprietes de l'objet designe par `atom`.
    pub fn table_of(&self, process: &Process, atom: u64) -> Option<u64> {
        let so = atom & !7;
        if !self.in_module(read_u64(process, so)?) {
            return None;
        }
        let t = read_u64(process, so + self.so_tbl)?;
        (read_u64(process, t)? == self.tbl_vt).then_some(t)
    }

    /// Table de l'objet stocke sous `key`.
    pub fn child(&self, process: &Process, tbl: u64, key: &str) -> Option<u64> {
        self.table_of(process, self.get(process, tbl, key)?)
    }

    pub fn child_cached(
        &self,
        process: &Process,
        tbl: u64,
        key: &str,
        hint: &mut u64,
    ) -> Option<u64> {
        self.table_of(process, self.get_cached(process, tbl, key, hint)?)
    }

    /// Retrouve `so_tbl` a partir d'un objet dont on connait une propriete.
    ///
    /// Sans la contrainte `expect_key`, plusieurs offsets menent a une table
    /// plausible et le mauvais serait mis en cache pour toutes les lectures
    /// suivantes.
    pub fn derive_so_tbl(&mut self, process: &Process, atom: u64, expect_key: &str) -> Option<u64> {
        let so = atom & !7;
        if !self.in_module(read_u64(process, so)?) {
            return None;
        }
        for off in SO_TBL_CANDIDATES {
            let Some(t) = read_u64(process, so + off) else {
                continue;
            };
            if read_u64(process, t) != Some(self.tbl_vt) {
                continue;
            }
            if self.capacity(process, t).is_none() {
                continue;
            }
            self.so_tbl = off;
            if self.get(process, t, expect_key).is_some() {
                return Some(t);
            }
        }
        None
    }

    /// Base de la table contenant `keyslot`, selon le profil courant.
    ///
    /// On remonte tant que le qword precedent decode comme une chaine, puis on
    /// soustrait l'offset des clefs. Un mauvais profil donne une base dont la
    /// vtable n'en est pas une, donc se rejette tout seul.
    ///
    /// `tbl_vt` inconnue (zero) : n'importe quel pointeur vers le module fait
    /// l'affaire et devient la vtable de reference. C'est ainsi qu'elle est
    /// derivee plutot que codee en dur.
    pub fn table_base(&mut self, process: &Process, keyslot: u64) -> Option<u64> {
        let mut first = keyslot;
        while first > self.profile.stride {
            let prev = first - self.profile.stride;
            if self.key_len(process, prev).is_none() {
                break;
            }
            first = prev;
        }
        let tbl = first.checked_sub(self.profile.keys)?;
        let vt = read_u64(process, tbl)?;
        if self.tbl_vt == 0 {
            if !self.in_module(vt) {
                return None;
            }
            self.tbl_vt = vt;
        } else if vt != self.tbl_vt {
            return None;
        }
        self.capacity(process, tbl)?;
        Some(tbl)
    }
}
