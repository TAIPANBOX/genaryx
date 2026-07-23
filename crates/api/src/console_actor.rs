//! Request-scoped console actor: WHO drove this command (docs/CONSOLE-IDP.md,
//! B3/1).
//!
//! Every privileged mutation journals a `CommandRecord.operator`. Built once
//! at bootstrap from the OS user (`money::state::operator_principal`), that is
//! honest for the DESKTOP shell, which runs on the operator's own machine with
//! no login of its own. The WEB shell is different: several named people sign
//! in over the tunnel, and the audit trail must name the human who logged in,
//! not the service account running `genaryx-web`.
//!
//! Threading an explicit actor argument through every mutation command (and
//! both shells' call sites) would churn a wide, shared surface for something
//! only the web shell sets. Instead this is a tokio task-local: the web shell
//! wraps each `dispatch` in [`with_actor`], and the two `CommandRecord`
//! builders read [`current`] as an OVERRIDE of the client's default operator.
//! The desktop shell never calls [`with_actor`], so [`current`] is `None`
//! there and behavior is byte-for-byte unchanged. Request-scoped context is
//! the textbook task-local use; the override is small, one place per plane,
//! and tested on both paths.

use std::future::Future;

tokio::task_local! {
    static ACTOR: Option<String>;
}

/// Run `fut` with `username` as the console actor for the duration. The web
/// shell passes the signed-in operator's username (`alice`), NOT a full
/// principal: the org domain is not the shell's to invent, so
/// [`operator_or`] grafts the username onto the client's own default
/// principal (which already carries the resolved `user://<org>/...`). A
/// `None` username behaves like no override, so the web shell can wrap
/// unconditionally.
pub async fn with_actor<F, T>(username: Option<String>, fut: F) -> T
where
    F: Future<Output = T>,
{
    ACTOR.scope(username, fut).await
}

/// The console-actor username set for the current task, if any. `None`
/// outside a [`with_actor`] scope (every desktop-shell call).
pub fn current() -> Option<String> {
    ACTOR.try_with(|a| a.clone()).ok().flatten()
}

/// The effective operator principal for a journaled command: the client's own
/// default (`fallback`, e.g. `user://acme.example/os-user`) with its LAST path
/// segment replaced by the request-scoped console username when one is set.
/// This keeps the scheme and org domain exactly what the audit trail already
/// uses (see `money::state::operator_principal`) and only swaps WHO. With no
/// override set (the desktop shell) it returns `fallback` unchanged.
pub fn operator_or(fallback: &str) -> String {
    match current() {
        None => fallback.to_string(),
        Some(user) => match fallback.rfind('/') {
            Some(pos) => format!("{}/{}", &fallback[..pos], user),
            // No `/` to graft onto (not a real principal); use the username as
            // given rather than fabricating a scheme.
            None => user,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_is_none_outside_a_scope() {
        assert!(current().is_none());
        assert_eq!(
            operator_or("user://acme.example/os-user"),
            "user://acme.example/os-user"
        );
    }

    #[tokio::test]
    async fn a_scope_grafts_the_username_onto_the_fallbacks_domain() {
        let got = with_actor(Some("alice".to_string()), async {
            operator_or("user://acme.example/os-user")
        })
        .await;
        // Scheme + org domain preserved, only WHO changed.
        assert_eq!(got, "user://acme.example/alice");
    }

    #[tokio::test]
    async fn a_none_scope_keeps_the_fallback() {
        let got = with_actor(None, async { operator_or("user://acme.example/os-user") }).await;
        assert_eq!(got, "user://acme.example/os-user");
    }

    #[tokio::test]
    async fn the_scope_does_not_leak_past_its_future() {
        with_actor(Some("alice".to_string()), async {}).await;
        // Back outside the scope: the override is gone.
        assert!(current().is_none());
    }
}
