//! This crate's integration tests, linked into ONE binary.
//!
//! Each file here used to be its own `tests/*.rs` target, so a change relinked
//! every one of them with the whole dependency tree inside; one binary is linked
//! once. Every test keeps its name and its file, and runs as
//! `cargo test -p <crate> --test it <file or test name>`.
//!
//! One binary is one process, so files that used to be isolated from each
//! other now share threads, the environment and statics. Audited when they
//! were merged (2026-10-06): scratch paths all carry a per-file prefix,
//! listeners bind port 0, and no file installs a global subscriber or hook.

mod command_test;
mod conform_test;
mod console_chain_test;
mod demo_test;
mod ingest_test;
mod store_test;
