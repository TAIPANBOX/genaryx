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
//! `delegation_revoke_test` is the only file that writes the environment, and
//! only the two `GENARYX_VOUCHRYX_*` variables, under its own `ENV_LOCK`; no
//! other file here reads them.

mod admission_check_test;
mod agent_stop_test;
mod approval_token_test;
mod budget_proof_test;
mod bus_state_test;
mod case_file_test;
mod case_seal_test;
mod cloud_stub;
mod copilot_commands_test;
mod credentials_keys_test;
mod crypto_commands_test;
mod delegation_revoke_test;
mod egress_panel_test;
mod events_ui_test;
mod evidence_build_test;
mod felyx_bench_test;
mod gateway_posture_test;
mod identity_commands_test;
mod keys_panel_test;
mod keys_witness_test;
mod memory_commands_test;
mod money_commands_test;
mod money_gateways_test;
mod money_runs_budgets_test;
mod money_runs_status_test;
mod money_units_test;
mod policy_decide_test;
mod policy_preview_test;
mod policy_proof_test;
mod quality_commands_test;
mod quarantine_is_visible_test;
mod record_verify_test;
mod remedy_check_test;
mod retained_close_test;
mod standing_grant_test;
mod stats_scale;
