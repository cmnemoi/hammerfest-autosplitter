//! Extrait de `vendor/hf.map.json` les seules clefs dont l'autosplitter a
//! besoin, et les emet en constantes.
//!
//! La table complete fait 2952 entrees et 75 Kio ; l'embarquer dans le `.wasm`
//! n'aurait pas de sens. La generer ici plutot que de recopier les chaines a la
//! main garde une seule source de verite : si la table change, la compilation
//! le repercute, et une clef disparue casse le build au lieu de produire un
//! autosplitter qui ne trouve plus rien a l'execution.
//!
//! Aucune dependance : chaque crate de build ajoute une macro procedurale ou un
//! script a compiler pour la machine hote, et le lecteur ci-dessous tient en
//! trente lignes.

use std::{collections::BTreeMap, env, fs, path::Path};

/// (nom de la constante, identifiant du source Hammerfest)
const WANTED: &[(&str, &str)] = &[
    ("WORLD", "world"),
    ("MANAGER", "manager"),
    ("CURRENT", "current"),
    ("F_VERSION", "fVersion"),
    ("CURRENT_ID", "currentId"),
    ("PREVIOUS_ID", "_previousId"),
    ("SET_NAME", "setName"),
    ("GAME_CHRONO", "gameChrono"),
    ("CURRENT_DIM", "currentDim"),
    ("FL_GAME_OVER", "fl_gameOver"),
    ("FRAME_TIMER", "frameTimer"),
    ("GAME_TIMER", "gameTimer"),
    ("HALTED_TIMER", "haltedTimer"),
    ("FL_STOP", "fl_stop"),
    ("FL_LOCK", "fl_lock"),
];

/// Identifiants que l'obfuscateur laisse tels quels.
///
/// Ce sont des noms de l'API AS2 standard : les renommer casserait le lecteur
/// Flash lui-meme. Leur absence de la table est donc la bonne reponse et non
/// une erreur -- mais leur *presence* en serait une, puisqu'elle voudrait dire
/// que la table a change de convention. D'ou la verification.
const WANTED_PLAIN: &[(&str, &str)] = &[("DURATION", "duration")];

/// Les noms de monde : obfusques eux aussi, puisqu'ils ressemblent a des
/// identifiants. Servent a verifier qu'un `GameMechanics` est bien un monde.
const WORLDS: &[&str] = &[
    "xml_adventure",
    "xml_deepnight",
    "xml_hiko",
    "xml_ayame",
    "xml_hk",
];

fn main() {
    let map_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/hf.map.json");
    println!("cargo:rerun-if-changed={}", map_path.display());
    println!("cargo:rerun-if-changed=build.rs");

    let raw = fs::read_to_string(&map_path).expect("vendor/hf.map.json illisible");
    let map = parse_flat_object(&raw);

    let lookup = |clear: &str| -> &str {
        map.get(clear)
            .unwrap_or_else(|| panic!("{clear:?} absent de vendor/hf.map.json"))
            .as_str()
    };

    // Commentaires ordinaires et non `//!` : le fichier est inclus dans un
    // module deja ouvert, ou un doc-comment interne n'a plus sa place.
    let mut out = String::from(
        "// Genere par build.rs depuis vendor/hf.map.json. Ne pas editer.\n\
         //\n\
         // Noms de proprietes tels qu'ils apparaissent dans le SWF obfusque.\n\n",
    );
    for (konst, clear) in WANTED {
        out.push_str(&format!(
            "/// `{clear}`\npub const {konst}: &str = {:?};\n",
            lookup(clear)
        ));
    }
    for (konst, clear) in WANTED_PLAIN {
        assert!(
            !map.contains_key(*clear),
            "{clear:?} est desormais renomme dans hf.map.json : le deplacer \
             dans WANTED"
        );
        out.push_str(&format!(
            "/// `{clear}`, que l'obfuscateur ne renomme pas.\n\
             pub const {konst}: &str = {clear:?};\n"
        ));
    }
    out.push_str(
        "\n/// Les mondes, nom obfusque puis nom clair. Le premier est le monde\n\
         /// principal, celui de l'aventure.\n\
         pub const WORLDS: &[(&str, &str)] = &[\n",
    );
    for clear in WORLDS {
        out.push_str(&format!("    ({:?}, {clear:?}),\n", lookup(clear)));
    }
    out.push_str("];\n");

    let dst = Path::new(&env::var("OUT_DIR").unwrap()).join("keys.rs");
    fs::write(dst, out).expect("ecriture de keys.rs impossible");
}

/// Lit un objet JSON plat `{"clef": "valeur", ...}`.
///
/// La table n'est faite que de chaines ASCII sans echappement -- c'est
/// verifie, pas suppose : toute sequence d'echappement fait echouer la
/// compilation plutot que de produire une clef silencieusement fausse.
fn parse_flat_object(src: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut chars = src.char_indices().peekable();
    let mut pending: Option<String> = None;

    while let Some((i, c)) = chars.next() {
        match c {
            '"' => {
                let start = i + 1;
                let mut end = None;
                for (j, c) in chars.by_ref() {
                    if c == '\\' {
                        panic!(
                            "echappement JSON a l'offset {j} : ce lecteur minimal ne \
                             les gere pas, il faut le completer"
                        );
                    }
                    if c == '"' {
                        end = Some(j);
                        break;
                    }
                }
                let s = src[start..end.expect("chaine JSON non terminee")].to_string();
                match pending.take() {
                    None => pending = Some(s),
                    Some(key) => {
                        map.insert(key, s);
                    }
                }
            }
            '{' | '}' | ':' | ',' => {}
            c if c.is_whitespace() => {}
            c => panic!("caractere inattendu {c:?} a l'offset {i}"),
        }
    }
    assert!(pending.is_none(), "clef sans valeur en fin de fichier");
    assert!(!map.is_empty(), "vendor/hf.map.json est vide");
    map
}
