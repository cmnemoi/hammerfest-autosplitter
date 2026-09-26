//! Emits the version of the module, with the commit it was built from.
//!
//! The obfuscated keys are the reader's business: `src/reader/build.rs`.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    emit_version();
}

/// Emits `AUTOSPLITTER_VERSION`: the crate version, and the commit it was built
/// from.
///
/// The crate version alone names a release, not a build. A module built from
/// `main` after a release carries the same number, and so does one built from
/// a working tree with changes in it. The commit tells them apart, and `-dirty`
/// marks the changes. With no git at hand, the crate version stands alone.
fn emit_version() {
    // A commit or a checkout moves `HEAD` or a ref; staging moves the index; an
    // edit moves `src`. Each one must run this script again, or the version
    // would name the build before.
    for path in [".git/HEAD", ".git/index", ".git/refs", "src"] {
        println!("cargo:rerun-if-changed={path}");
    }
    let git = |args: &[&str]| Command::new("git").args(args).output().ok();
    let version = env!("CARGO_PKG_VERSION");
    let full = match git(&["rev-parse", "--short=7", "HEAD"]) {
        Some(out) if out.status.success() => {
            let commit = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // Untracked files are left out: a stray note beside the sources
            // changes nothing in the module.
            let dirty = git(&["diff", "--quiet", "HEAD"]).is_some_and(|out| !out.status.success());
            format!("{version} ({commit}{})", if dirty { "-dirty" } else { "" })
        }
        _ => version.to_string(),
    };
    println!("cargo:rustc-env=AUTOSPLITTER_VERSION={full}");
}
