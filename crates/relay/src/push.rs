//! `ApnsSender` seam (docs/PHASE5.md "push" module). Sim uses [`NullSender`]
//! (the phone polls `GET /relay/v1/exceptions` instead, docs/PHASE5.md
//! "Sim-first deltas"); R1 drops in `tokenfuse-cloud::apns`'s real HTTP/2
//! provider-JWT sender (itrat-console/13 D12.2b step 4) behind this same
//! trait, no call-site change required.

/// One push to deliver to the paired device.
#[derive(Debug, Clone)]
pub struct Notification {
    pub apns_token: String,
    pub title: String,
    pub body: String,
    pub run_id: Option<String>,
    pub incident_id: Option<String>,
    pub kind: String,
}

/// Where pushes go. Mirrors `tokenfuse-cloud::push::PushSender`'s shape
/// (fire-and-forget, `Send + Sync` for sharing across the relay's async
/// tasks) without depending on that crate, since only the ONE method this
/// relay needs (an alert-style push) applies here -- Live-Activity updates
/// are R2 scope (itrat-console/13 D12.2b step 6).
pub trait ApnsSender: Send + Sync {
    fn send(&self, notification: Notification);
}

/// Sim-phase sender: logs what would have been pushed and does nothing else.
/// The phone's polling loop is the real delivery path until R1 wires real
/// APNs (docs/PHASE5.md: "The `ApnsSender` seam is wired but points at a
/// NullSender; swapping in the real ... `tokenfuse-cloud::apns` is an R1
/// config change").
pub struct NullSender;

impl ApnsSender for NullSender {
    fn send(&self, notification: Notification) {
        eprintln!(
            "genaryx-relay: would push to device token {}: {} - {} (run={:?} incident={:?} kind={})",
            redact_token(&notification.apns_token),
            notification.title,
            notification.body,
            notification.run_id,
            notification.incident_id,
            notification.kind
        );
    }
}

/// First 6 chars + length, never the full token: even in the sim NullSender,
/// a push token is device-identifying and has no business in full in logs
/// (06 §0.5 logging hygiene, the same instinct behind every `<redacted>`
/// `Debug` impl elsewhere in this workspace).
fn redact_token(token: &str) -> String {
    let prefix: String = token.chars().take(6).collect();
    format!("{prefix}…(len={})", token.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSender {
        sent: Mutex<Vec<Notification>>,
    }

    impl ApnsSender for RecordingSender {
        fn send(&self, notification: Notification) {
            self.sent.lock().unwrap().push(notification);
        }
    }

    #[test]
    fn null_sender_does_not_panic_and_accepts_any_notification() {
        let sender = NullSender;
        sender.send(Notification {
            apns_token: "tok".to_string(),
            title: "t".to_string(),
            body: "b".to_string(),
            run_id: Some("r1".to_string()),
            incident_id: None,
            kind: "kill".to_string(),
        });
    }

    #[test]
    fn a_sender_impl_receives_exactly_what_was_sent() {
        let sender = RecordingSender::default();
        sender.send(Notification {
            apns_token: "tok".to_string(),
            title: "Run killed".to_string(),
            body: "Agent run r1 was killed".to_string(),
            run_id: Some("r1".to_string()),
            incident_id: None,
            kind: "kill".to_string(),
        });
        let sent = sender.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].title, "Run killed");
    }
}
