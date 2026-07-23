//! Admission: I6 "admission gate" - the VERIFY step between the B2 onboard
//! wizard and flipping the gateway to strict identity mode (docs/ADMISSION.md).
//!
//! The wizard (`crate::onboard`) stays offline: it only generates artifacts.
//! This plane is the NEW one that actually talks to the stack, proving a
//! newcomer's key is known and bound, that first traffic flows, rehearsing
//! the guardrails with a mockryx drill AS the newcomer key (reusing
//! `crate::drills::commands::drills_run` unchanged - no new command for that
//! leg), and optionally establishing a Verdryx quality baseline for the
//! newcomer through the gateway - then handing back a copy-paste "enable
//! strict" proposal. Propose-never-mutate holds here too: nothing in this
//! module edits env vars, config, or the identity map; "enable strict" is a
//! text block the operator copies, exactly like `crate::onboard`'s bundle.
//!
//! Mirrors `crate::credentials`'s module shape (`env`/`state`/`commands`, the
//! same non-blocking bootstrap-in-`setup` wiring, the same
//! Bootstrapping/NoEnvironment/Unreachable/Ready gateway state machine over
//! `GatewayClient::get_keys` as the reachability probe) for the ONE leg that
//! benefits from a held connection - the gateway. The Verdryx binary and
//! `verdryx.db` legs are deliberately NOT folded into that same state machine
//! (see [`env`]'s module doc, "Honest per-piece resolution states"): they are
//! independent facts, re-checked fresh on every [`commands::admission_status`]
//! call, exactly like `crate::quality::env`'s own file-existence checks are
//! redone per call rather than cached.
//!
//! [`state`] bootstraps an [`state::AdmissionState`] the same non-blocking way
//! every other plane's `setup`/`Ctx::resolve` does. [`commands`] exposes three
//! commands: [`commands::admission_status`] (never fails - every leg's honest
//! resolution state), [`commands::admission_check`] (a viewer-safe read: is
//! this key known to the gateway, is it bound, has it seen traffic - straight
//! from `GatewayClient::get_keys`, plus an `in_map` check that ports the
//! docs/20 pattern grammar `crate::onboard::commands` already implements for
//! its own `in_map` field), and [`commands::admission_baseline`] (admin-only:
//! shells the `verdryx` binary to run an eval THROUGH the gateway under the
//! newcomer's own key, then a `verdryx baseline` snapshot, then reads the
//! result back via the EXISTING read-only `genaryx_connectors::VerdryxClient`,
//! no new connector needed). The fourth leg the UI shows, the guardrail
//! drill, adds NO command at all: the frontend calls the EXISTING
//! `crate::drills::commands::drills_run` with the newcomer's key.

pub mod commands;
pub mod env;
pub mod state;

pub use state::{AdmissionState, bootstrap};
