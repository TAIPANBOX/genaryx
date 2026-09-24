//! Delegation: the console's cut-off switch for an agent's or a user's
//! delegated authority, against vouchryx (`POST /v1/revoke`).
//!
//! - [`env`] resolves `GENARYX_VOUCHRYX_URL` +
//!   `GENARYX_VOUCHRYX_REVOKE_KEY_FILE` once at startup. Unlike every other
//!   plane, an inconsistent pair refuses to START the process rather than
//!   degrading to a clean "no environment": see its module doc.
//! - [`commands`] is `delegation_revoke` itself: validate, call vouchryx,
//!   journal the attempt (success or refusal) into the same command journal
//!   `money_kill_run` and `remote_operator_wg_revoke` write to.
//!
//! No `state.rs`: there is no long-lived client to manage or resolve in the
//! background (`crates/web/src/ctx.rs`'s `Ctx::resolve` pattern every other
//! plane uses). The revoke key is read once by [`env::resolve_from_env`] at
//! startup and handed to [`commands::delegation_revoke`] on every call.

pub mod commands;
pub mod env;
