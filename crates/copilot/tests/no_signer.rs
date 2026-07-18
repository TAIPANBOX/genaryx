//! The D13.3/D13.4 structural invariant, enforced at build time: `crates/copilot`
//! must NEVER depend on `genaryx-signing`. Holding no signer is what makes "an
//! AI cannot press the buttons" a fact about the dependency graph rather than a
//! prompt. `include_str!` embeds this crate's own manifest at compile time; we
//! parse the actual dependency TABLES (not prose), so the invariant comment in
//! Cargo.toml does not trip the check.

#[test]
fn copilot_does_not_depend_on_the_signer() {
    let doc: toml::Value =
        toml::from_str(include_str!("../Cargo.toml")).expect("own Cargo.toml parses");
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(deps) = doc.get(table).and_then(toml::Value::as_table) {
            assert!(
                !deps.contains_key("genaryx-signing"),
                "crates/copilot must never depend on genaryx-signing (found in [{table}]): \
                 the copilot is read + propose, never act (D13.3/D13.4). Route accepted \
                 proposals through the existing human-signed ceremony instead."
            );
        }
    }
}
