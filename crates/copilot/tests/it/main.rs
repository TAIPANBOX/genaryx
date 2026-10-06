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
//!
//! `residency_no_proxy_test.rs` is deliberately NOT here: it sets
//! `HTTP_PROXY`/`ALL_PROXY` for the whole process, and every reqwest client the
//! other files build would route through that proxy for as long as it is set.
//! It stays a binary of its own (allow-listed in
//! `scripts/one-test-binary-per-crate.sh`).

mod agent_id_header_test;
mod felyx_reads_budgets_test;
mod money_reaches_the_model_as_usd_test;
mod no_signer;
mod residency_hostname_test;
mod residency_no_redirect_test;
