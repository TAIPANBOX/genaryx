//! Tiny demo-data generator: writes the deterministic `taipan demo` NDJSON bus
//! files (tokenfuse / wardryx / engram / verdryx / mockryx / qryx) into a
//! directory, so the bus-driven Genaryx tabs render full for a screenshot pass.
//!
//!   cargo run -p genaryx-core --example gen_demo -- /tmp/genaryx-events

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/genaryx-events".to_string());
    let path = std::path::Path::new(&dir);
    match genaryx_core::demo::generate(path) {
        Ok(n) => println!("gen_demo: wrote {n} events into {dir}"),
        Err(e) => {
            eprintln!("gen_demo: FAILED: {e}");
            std::process::exit(1);
        }
    }
}
