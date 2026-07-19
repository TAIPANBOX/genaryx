//! The C3 triage stage (docs/PHASE6-C3.md, itrat-console/13 D13.4): Felyx in
//! front of the relay's push path, with one inviolable rule -
//!
//! > **HARD events always push, unfiltered and immediately.**
//!
//! That floor is deterministic code here ([`Triage::on_intent`] dispatches a
//! HARD [`PushIntent`] BEFORE any copilot call), so an AI can never silence the
//! pager; and the copilot holds no signer (C0-C2), so it can never press its
//! buttons. The copilot may only ADD: a best-effort, budgeted annotation that
//! enriches what the phone's next poll shows, spawned so it never blocks the
//! push or delays the loop. SOFT events do not page immediately - they are held
//! in a soft-queue and flushed as one digest on a cadence (batch / hold).
//!
//! The copilot is OPTIONAL: with no provider configured (the default on a box
//! with no local model), `copilot` is `None` and the relay behaves exactly as
//! it did before C3 (plain pushes) - the deterministic pager always works
//! without any AI (itrat-console/13 Q7).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use genaryx_copilot::{Clients, CopilotConfig, CopilotService, ProviderKind};

use crate::exceptions::{ExceptionEngine, PushIntent, dispatch_push};
use crate::push::ApnsSender;
use crate::registry::Registry;

/// Triage tunables (all env-overridable, sane defaults).
#[derive(Debug, Clone, Copy)]
pub struct TriageConfig {
    /// How long a HARD push waits for its annotation before the enrichment is
    /// dropped (the push already went out plain). Default 3 s (D13.4).
    pub annotation_budget: Duration,
    /// How often the soft-event digest flushes. A long value is the "hold to a
    /// morning summary" mode; a short value batches near-real-time.
    pub soft_flush_secs: i64,
}

impl Default for TriageConfig {
    fn default() -> Self {
        Self {
            annotation_budget: Duration::from_millis(3000),
            soft_flush_secs: 300,
        }
    }
}

impl TriageConfig {
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            annotation_budget: std::env::var("GENARYX_RELAY_ANNOTATION_BUDGET_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_millis)
                .unwrap_or(d.annotation_budget),
            soft_flush_secs: std::env::var("GENARYX_RELAY_SOFT_FLUSH_SECS")
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(d.soft_flush_secs),
        }
    }
}

/// The triage stage. Holds the engine (to enrich items), the push path (to
/// dispatch the deterministic floor), the optional copilot, and the soft-queue.
pub struct Triage {
    engine: Arc<ExceptionEngine>,
    registry: Arc<Registry>,
    push: Arc<dyn ApnsSender>,
    /// `None` when no provider is configured -> no annotation, plain pushes
    /// (the pre-C3 behavior). The floor never depends on this being `Some`.
    copilot: Option<Arc<CopilotService>>,
    soft: Mutex<Vec<PushIntent>>,
    config: TriageConfig,
}

impl Triage {
    pub fn new(
        engine: Arc<ExceptionEngine>,
        registry: Arc<Registry>,
        push: Arc<dyn ApnsSender>,
        copilot: Option<Arc<CopilotService>>,
        config: TriageConfig,
    ) -> Self {
        Self {
            engine,
            registry,
            push,
            copilot,
            soft: Mutex::new(Vec::new()),
            config,
        }
    }

    pub fn engine(&self) -> &Arc<ExceptionEngine> {
        &self.engine
    }

    pub fn soft_flush_secs(&self) -> i64 {
        self.config.soft_flush_secs
    }

    /// Route one intent. HARD: dispatch NOW (the floor), then spawn a budgeted
    /// annotation. SOFT: hold for the next digest.
    pub fn on_intent(&self, intent: PushIntent) {
        if intent.hard {
            // The deterministic floor: unconditional, before any copilot call.
            dispatch_push(&self.registry, self.push.as_ref(), intent.clone());
            // Best-effort enrichment (spawned; cannot block, delay, or suppress).
            if let (Some(copilot), Some(key)) = (self.copilot.clone(), item_key(&intent)) {
                let engine = self.engine.clone();
                let budget = self.config.annotation_budget;
                tokio::spawn(async move {
                    annotate_hard(&copilot, &engine, &key, &intent, budget).await;
                });
            }
        } else {
            self.soft
                .lock()
                .expect("triage soft mutex poisoned")
                .push(intent);
        }
    }

    /// Emit ONE digest push for the batched SOFT events, then clear the queue.
    /// A no-op when the queue is empty.
    pub fn flush_soft(&self) {
        let batch: Vec<PushIntent> = {
            let mut q = self.soft.lock().expect("triage soft mutex poisoned");
            std::mem::take(&mut *q)
        };
        if batch.is_empty() {
            return;
        }
        dispatch_push(&self.registry, self.push.as_ref(), digest_intent(&batch));
    }

    #[cfg(test)]
    fn soft_len(&self) -> usize {
        self.soft.lock().expect("triage soft mutex poisoned").len()
    }
}

/// The queue key an intent's exception is stored under - matches the key format
/// the engine's handlers build (`run:<id>` / `incident:<id>`), so the annotation
/// lands on the same item the snapshot serves.
fn item_key(intent: &PushIntent) -> Option<String> {
    if let Some(run) = &intent.run_id {
        Some(format!("run:{run}"))
    } else {
        intent
            .incident_id
            .as_ref()
            .map(|inc| format!("incident:{inc}"))
    }
}

/// Best-effort annotation of a HARD event, bounded by `budget`. The floor push
/// has ALREADY gone out by the time this runs; this only enriches the polled
/// snapshot. A timeout, a provider error, or a disabled copilot all leave the
/// item plain - never a failure that could affect the push.
async fn annotate_hard(
    copilot: &CopilotService,
    engine: &ExceptionEngine,
    key: &str,
    intent: &PushIntent,
    budget: Duration,
) {
    let event = format!("{}: {}", intent.title, intent.body);
    match tokio::time::timeout(budget, copilot.annotate(&event)).await {
        Ok(Ok(Some(annotation))) => {
            // Observability symmetry with the failure arms below: record the
            // enrichment we attached (the floor push has already gone out).
            eprintln!(
                "genaryx-relay: triage: attached copilot annotation to {key}: {}",
                annotation.summary
            );
            engine.annotate_item(key, annotation);
        }
        Ok(Ok(None)) => {} // copilot disabled -> plain push already delivered
        Ok(Err(e)) => eprintln!("genaryx-relay: triage: annotation failed (pushed plain): {e}"),
        Err(_) => eprintln!(
            "genaryx-relay: triage: annotation over {}ms budget (pushed plain)",
            budget.as_millis()
        ),
    }
}

/// One digest [`PushIntent`] summarizing a batch of SOFT events.
fn digest_intent(batch: &[PushIntent]) -> PushIntent {
    let n = batch.len();
    let names: Vec<&str> = batch
        .iter()
        .filter_map(|i| i.run_id.as_deref().or(i.incident_id.as_deref()))
        .take(5)
        .collect();
    let more = n.saturating_sub(names.len());
    let mut body = format!("{n} near-cap warning{}", if n == 1 { "" } else { "s" });
    if !names.is_empty() {
        body.push_str(": ");
        body.push_str(&names.join(", "));
        if more > 0 {
            body.push_str(&format!(" +{more} more"));
        }
    }
    PushIntent {
        title: "Warnings digest".to_string(),
        body,
        run_id: None,
        incident_id: None,
        kind: "digest".to_string(),
        // A digest is itself SOFT - it batches SOFT events; it never re-enters
        // the HARD floor.
        hard: false,
    }
}

/// Build the relay's copilot from env (`GENARYX_RELAY_COPILOT_*`), or `None`
/// when no provider is configured / the config is invalid. Annotation needs no
/// tools, so `Clients::default()` is correct here. Local-only by default (the
/// residency / trial posture); a remote BYO endpoint needs an explicit opt-in.
pub fn build_copilot_from_env() -> Option<Arc<CopilotService>> {
    let config = copilot_config_from_env();
    match CopilotService::from_config_and_clients(&config, Clients::default()) {
        Ok(svc) if svc.is_enabled() => {
            if let Some(d) = svc.descriptor() {
                eprintln!(
                    "genaryx-relay: copilot annotation enabled ({} / {}, local={})",
                    d.provider, d.model, d.local
                );
            }
            Some(Arc::new(svc))
        }
        // provider = none (the default): no annotation, plain pushes.
        Ok(_) => None,
        Err(e) => {
            eprintln!("genaryx-relay: copilot annotation disabled (config error): {e}");
            None
        }
    }
}

fn copilot_config_from_env() -> CopilotConfig {
    let provider = match std::env::var("GENARYX_RELAY_COPILOT_PROVIDER")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "ollama" => ProviderKind::Ollama,
        "lmstudio" => ProviderKind::LmStudio,
        "openai_compat" => ProviderKind::OpenAiCompat,
        "anthropic" => ProviderKind::Anthropic,
        "openrouter" => ProviderKind::OpenRouter,
        _ => ProviderKind::None,
    };
    CopilotConfig {
        provider,
        base_url: std::env::var("GENARYX_RELAY_COPILOT_BASE_URL").ok(),
        model: std::env::var("GENARYX_RELAY_COPILOT_MODEL").ok(),
        api_key_ref: std::env::var("GENARYX_RELAY_COPILOT_API_KEY_REF").ok(),
        // Local-only unless explicitly opted in - the relay's residency / trial
        // default (a trial license hard-locks this to false; the sim keeps it
        // false, the strongest "nothing leaves the box" posture).
        allow_non_local_endpoints: std::env::var("GENARYX_RELAY_COPILOT_ALLOW_REMOTE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        run_id: "genaryx-relay-copilot".to_string(),
        ..CopilotConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exceptions::ExceptionClass;
    use crate::push::Notification;

    struct DropSender;
    impl ApnsSender for DropSender {
        fn send(&self, _n: Notification) {}
    }

    fn triage(copilot: Option<Arc<CopilotService>>) -> Triage {
        Triage::new(
            Arc::new(ExceptionEngine::new("acme", 0.8, 600)),
            Arc::new(Registry::open_in_memory().unwrap()),
            Arc::new(DropSender),
            copilot,
            TriageConfig::default(),
        )
    }

    fn intent(hard: bool, run: &str) -> PushIntent {
        PushIntent {
            title: "t".into(),
            body: "b".into(),
            run_id: Some(run.into()),
            incident_id: None,
            kind: "budget".into(),
            hard,
        }
    }

    #[tokio::test]
    async fn hard_intent_is_dispatched_now_not_held_even_without_a_copilot() {
        // The deterministic floor: a HARD intent is routed to immediate dispatch
        // (never parked in the soft-queue), and works with copilot = None.
        let t = triage(None);
        t.on_intent(intent(true, "reconciliation-batch"));
        assert_eq!(
            t.soft_len(),
            0,
            "a HARD event is never held in the soft-queue"
        );
    }

    #[tokio::test]
    async fn soft_intent_is_held_until_the_digest_flush() {
        let t = triage(None);
        t.on_intent(intent(false, "support-bot-1"));
        t.on_intent(intent(false, "support-bot-2"));
        assert_eq!(t.soft_len(), 2, "SOFT events wait for the digest");
        t.flush_soft();
        assert_eq!(t.soft_len(), 0, "flush drains the soft-queue");
        t.flush_soft(); // empty flush is a no-op
        assert_eq!(t.soft_len(), 0);
    }

    #[test]
    fn item_key_matches_the_engine_key_format() {
        assert_eq!(item_key(&intent(true, "r-1")).as_deref(), Some("run:r-1"));
        let inc = PushIntent {
            title: "t".into(),
            body: "b".into(),
            run_id: None,
            incident_id: Some("i-9".into()),
            kind: "incident".into(),
            hard: true,
        };
        assert_eq!(item_key(&inc).as_deref(), Some("incident:i-9"));
    }

    #[test]
    fn digest_summarizes_the_batch() {
        let batch = vec![intent(false, "a"), intent(false, "b"), intent(false, "c")];
        let d = digest_intent(&batch);
        assert!(!d.hard);
        assert_eq!(d.kind, "digest");
        assert!(d.body.contains("3 near-cap warnings"));
        assert!(d.body.contains("a, b, c"));
    }

    // A HARD event enriches its snapshot item via annotate_item (the engine
    // side of the annotation path); the copilot's own annotate() summary is
    // covered in the copilot crate. Together with the floor test above, this
    // covers "push first (floor), enrich after".
    #[test]
    fn annotate_item_enriches_a_hard_items_snapshot() {
        use crate::exceptions::ExceptionItem;
        use genaryx_copilot::CopilotAnnotation;
        let engine = ExceptionEngine::new("acme", 0.8, 600);
        // Seed a HARD (over-cap) item the way a kill would.
        engine.seed_item_for_test(ExceptionItem {
            key: "run:r-1".into(),
            run_id: Some("r-1".into()),
            incident_id: None,
            kind: "kill".into(),
            class: ExceptionClass::OverCap,
            severity: None,
            headline: "Agent run r-1 was killed".into(),
            spent_microusd: 0,
            budget_micros: None,
            fraction: None,
            first_seen_unix: 0,
            last_seen_unix: 0,
            acknowledged: false,
            killed: true,
            copilot: None,
        });
        let ann = CopilotAnnotation {
            summary: "r-1 was killed after tripling its burn".into(),
            recommended_action: None,
            confidence: 0.6,
            chain: vec![],
        };
        assert!(engine.annotate_item("run:r-1", ann.clone()));
        let snap = engine.snapshot();
        let found = snap.queue.iter().find(|i| i.key == "run:r-1").unwrap();
        assert_eq!(found.copilot.as_ref().unwrap().summary, ann.summary);
        // Annotating a missing key is a no-op, not a panic.
        assert!(!engine.annotate_item("run:does-not-exist", ann));
    }
}
