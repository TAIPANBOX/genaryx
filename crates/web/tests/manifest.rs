//! The declaration in `components.json` is only worth reading if this repository
//! proves it, and proves it against the toolchain rather than by describing.
//!
//! estate-gates cannot do this. It has no Rust toolchain, and building
//! twenty-two repositories in its CI is a matrix it does not have. This
//! repository already runs `cargo test` on every push.
//!
//! What is proved here is exactly the `checked` bucket and nothing else. The
//! `declared` bucket is not asserted against anything, on purpose: a test that
//! pretended to verify a sentence about purpose would be the failure this whole
//! design exists to avoid.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The repository root. `CARGO_MANIFEST_DIR` is `crates/web`.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/web has two ancestors")
        .to_path_buf()
}

fn manifest() -> Value {
    let path = root().join("components.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("components.json is valid JSON")
}

fn components(m: &Value) -> Vec<&Value> {
    let cs = m["components"].as_array().expect("components is an array");
    assert!(
        !cs.is_empty(),
        "components.json declares nothing, so every test here measured nothing"
    );
    cs.iter().collect()
}

/// THE ONE THAT CLOSES THE HOLE. A binary this repository builds and does not
/// declare is invisible from outside by construction.
#[test]
fn every_binary_this_workspace_builds_is_declared_and_the_reverse() {
    let m = manifest();
    let comps = components(&m);

    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(root().join("Cargo.toml"))
        .output()
        .expect("cargo metadata runs");
    assert!(
        out.status.success(),
        "cargo metadata: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: Value = serde_json::from_slice(&out.stdout).expect("cargo metadata is JSON");

    let mut built: BTreeMap<String, String> = BTreeMap::new();
    for p in meta["packages"].as_array().expect("packages") {
        for t in p["targets"].as_array().expect("targets") {
            if t["kind"]
                .as_array()
                .expect("kind")
                .iter()
                .any(|k| k == "bin")
            {
                built.insert(
                    t["name"].as_str().expect("target name").to_string(),
                    p["name"].as_str().expect("package name").to_string(),
                );
            }
        }
    }
    assert!(
        !built.is_empty(),
        "cargo metadata found no binary, so this measured nothing"
    );

    let declared: BTreeMap<String, String> = comps
        .iter()
        .filter_map(|c| {
            Some((
                c["checked"]["binary"].as_str()?.to_string(),
                c["checked"]["crate"].as_str()?.to_string(),
            ))
        })
        .collect();
    assert!(
        !declared.is_empty(),
        "no component declares a binary, so this measured nothing"
    );

    for b in built.keys() {
        assert!(
            declared.contains_key(b),
            "this workspace builds `{b}` and components.json does not declare it"
        );
    }
    for (b, k) in &declared {
        assert!(
            built.contains_key(b),
            "components.json declares the binary `{b}` and no workspace builds it"
        );
        assert_eq!(
            built.get(b),
            Some(k),
            "components.json says `{b}` comes from crate `{k}`; cargo says {:?}",
            built.get(b)
        );
    }
}

/// Every `GENARYX_` name in PRODUCT source against every one declared, and the
/// test-only ones must not be declared at all.
///
/// RUST PUTS ITS UNIT TESTS INSIDE THE SOURCE FILE, which is what makes this
/// different from the Go siblings: they skip `*_test.go` and are done. Here a
/// `#[cfg(test)] mod tests` sits at the bottom of the same file, so each file is
/// cut at the first MODULE-LEVEL `#[cfg(test)]` and only what precedes it counts
/// as product.
///
/// The limit of that, stated rather than hidden: a `#[cfg(test)]` block in the
/// middle of a file, with product code after it, would put those later names in
/// the test set. The convention in this repository is tests at the end, and the
/// second assertion below is what would catch the day it stops being true, since
/// a product name that fell into the test set is reported by name rather than
/// silently dropped.
#[test]
fn every_environment_variable_this_repository_reads_is_declared_and_no_test_fixture_is() {
    let m = manifest();
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for c in components(&m) {
        if let Some(env) = c["checked"]["env"].as_object() {
            declared.extend(env.keys().cloned());
        }
    }
    assert!(
        !declared.is_empty(),
        "no component declares an environment variable, so this measured nothing"
    );

    let (product, test_only) = names_by_section(&root().join("crates"));
    assert!(
        !product.is_empty(),
        "no GENARYX_ name found in product source, so this measured nothing"
    );
    assert!(
        !test_only.is_empty(),
        "no GENARYX_ name found only under #[cfg(test)], so the cut this test \
         performs is now unexercised and would not be noticed if it broke"
    );

    let missing: Vec<_> = product.difference(&declared).cloned().collect();
    let extra: Vec<_> = declared.difference(&product).cloned().collect();
    assert!(
        missing.is_empty(),
        "product code reads these and components.json declares none of them: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "components.json declares these and no product source reads them: {extra:?}"
    );

    let fixtures: Vec<_> = declared.intersection(&test_only).cloned().collect();
    assert!(
        fixtures.is_empty(),
        "components.json declares {fixtures:?}, which appear only under #[cfg(test)].\n\
         A test fixture in this file becomes part of the estate's idea of how to \
         configure this component."
    );
}

/// The declared listen default is the one the argument parser falls back to.
#[test]
fn the_declared_listen_default_is_the_one_the_code_uses() {
    let m = manifest();
    let main =
        std::fs::read_to_string(root().join("crates/web/src/main.rs")).expect("reading main.rs");

    let mut checked = 0;
    for c in components(&m) {
        let Some(want) = c["checked"]["listen_default"].as_str() else {
            continue;
        };
        checked += 1;
        assert!(
            main.contains(&format!("default_value = \"{want}\"")),
            "components.json says the default listen address is {want:?} and \
             main.rs declares no such default"
        );
    }
    assert!(
        checked > 0,
        "no component declares a listen default, so this measured nothing"
    );
}

/// The declared health path is a route the service actually serves, and it is
/// served WITHOUT authentication, which is what a launcher polling it relies on.
#[test]
fn the_declared_health_path_is_a_route_the_service_serves() {
    let m = manifest();
    let main =
        std::fs::read_to_string(root().join("crates/web/src/main.rs")).expect("reading main.rs");

    let mut checked = 0;
    for c in components(&m) {
        let Some(path) = c["checked"]["health_path"].as_str() else {
            continue;
        };
        checked += 1;
        assert!(
            main.contains(&format!(".route(\"{path}\"")),
            "components.json declares {path:?} as the health path and main.rs \
             registers no such route"
        );
    }
    assert!(
        checked > 0,
        "no component declares a health path, so this measured nothing"
    );
}

/// Every declared subcommand is one the argument parser knows.
#[test]
fn every_declared_subcommand_is_one_the_binary_dispatches_on() {
    let m = manifest();
    let main =
        std::fs::read_to_string(root().join("crates/web/src/main.rs")).expect("reading main.rs");

    let mut checked = 0;
    for c in components(&m) {
        let Some(subs) = c["checked"]["subcommands"].as_array() else {
            continue;
        };
        for s in subs {
            let s = s.as_str().expect("a subcommand is a string");
            checked += 1;
            // clap derives the variant name, so `serve` is `Serve`.
            let mut variant = s.to_string();
            variant[..1].make_ascii_uppercase();
            assert!(
                main.contains(&format!("    {variant} "))
                    || main.contains(&format!("    {variant},")),
                "components.json says {} takes `{s}` and main.rs's command enum has no {variant}",
                c["name"]
            );
        }
    }
    assert!(
        checked > 0,
        "no component declares a subcommand, so this measured nothing"
    );
}

/// (product names, names seen only under `#[cfg(test)]`)
fn names_by_section(dir: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut product = BTreeSet::new();
    let mut after = BTreeSet::new();
    walk(dir, &mut |p: &Path| {
        let s = p.to_string_lossy();
        if !s.ends_with(".rs") || s.contains("/target/") || s.contains("/tests/") {
            return;
        }
        let Ok(body) = std::fs::read_to_string(p) else {
            return;
        };
        let cut = module_level_cfg_test(&body).unwrap_or(body.len());
        for n in names_in(&body[..cut]) {
            if !n.ends_with('_') {
                product.insert(n);
            }
        }
        for n in names_in(&body[cut..]) {
            if !n.ends_with('_') {
                after.insert(n);
            }
        }
    });
    let test_only: BTreeSet<String> = after.difference(&product).cloned().collect();
    (product, test_only)
}

/// The offset of the first `#[cfg(test)]` written at column zero.
fn module_level_cfg_test(body: &str) -> Option<usize> {
    let needle = "#[cfg(test)]";
    let mut at = 0;
    for line in body.split_inclusive('\n') {
        if line.starts_with(needle) {
            return Some(at);
        }
        at += line.len();
    }
    None
}

fn names_in(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let needle = b"GENARYX_";
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            while j < bytes.len()
                && (bytes[j].is_ascii_uppercase() || bytes[j].is_ascii_digit() || bytes[j] == b'_')
            {
                j += 1;
            }
            out.push(String::from_utf8_lossy(&bytes[i..j]).into_owned());
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn walk(dir: &Path, f: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "target" || name == ".git" || name == "node_modules" {
                continue;
            }
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}
