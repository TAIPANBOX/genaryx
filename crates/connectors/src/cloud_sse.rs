//! `CloudSse`: an [`EventSource`] over TokenFuse Cloud's live stream
//! (`GET /v1/stream`, 07 §4.2: bearer-authenticated `text/event-stream`
//! carrying one CallRecord-level JSON object per SSE `data:` frame).
//!
//! Phase-0 spike #6 (06 §7): resilient to (a) SSE frames split across TCP
//! read boundaries -- delegated entirely to [`crate::sse_decoder::SseDecoder`],
//! which is fed raw bytes exactly as the transport hands them over -- and (b)
//! connection drops, via the bounded-exponential-backoff reconnect loop below.
//!
//! ## Transport choice: `reqwest`, not a dedicated SSE crate
//!
//! A maintained async SSE client (`eventsource-client`, built on hyper)
//! exists and already implements reconnect-with-backoff and
//! `Last-Event-ID`. It was not chosen here because this spike's actual
//! deliverable is the *frame decoder itself* under direct unit test (chunk-
//! split resilience, 07 §4.2, is the whole point of spike #6): reaching for
//! a crate that owns its own internal decoder would hide exactly the logic
//! this spike exists to prove, leaving us testing someone else's parser
//! instead of ours. `reqwest` (already a transitive dependency of this
//! workspace via `genaryx-ffi`'s bindgen tooling -- docs/PHASE0.md F-04, so
//! this adds no new dependency *family* to the tree, only a new direct edge
//! to one already resolved) supplies an async byte stream
//! (`Response::bytes_stream`, the `stream` feature) plus bearer auth
//! (`RequestBuilder::bearer_auth`) and nothing else opinionated: our own
//! [`SseDecoder`] owns all of the framing logic, so the unit tests in
//! `sse_decoder.rs` exercise the real thing, not a stand-in.
//!
//! ## Shape: a background thread, not `EventSource::poll()` gone async
//!
//! `EventSource::poll()` (`genaryx_core::ingest`) is synchronous and
//! non-blocking by contract: called repeatedly from a plain blocking loop
//! (`IngestService::run_blocking`) with no async runtime anywhere nearby. A
//! live SSE connection is the opposite shape -- a long-lived, inherently
//! async read loop. This module bridges the two exactly the way
//! `crates/ffi` already bridges its own async-to-sync boundary
//! (docs/PHASE0.md F-04): one dedicated OS thread owns a small
//! current-thread Tokio runtime and runs the connect/decode/reconnect loop
//! start to finish, forwarding decoded records to a plain
//! [`std::sync::mpsc`] channel that [`CloudSse::poll`] drains synchronously
//! with `try_recv`. No async ever crosses the `EventSource` boundary --
//! `CloudSse` is a complete, real `EventSource` impl, not a proposal.
//!
//! Each decoded `data:` JSON line becomes a [`RawRecord`] with `raw` set to
//! the JSON text and `file`/`offset` left `None` (there is no file or byte
//! offset for a network stream); this feeds the exact same
//! conform/quarantine/broadcast path in `IngestService::poll_once` that
//! `FileTail` already does, unchanged (07 §3/§4.2).

use crate::sse_decoder::SseDecoder;
use futures_util::StreamExt;
use genaryx_core::{Error, EventSource, RawRecord, Result};
use std::sync::mpsc;
use std::time::Duration;
use tokio::sync::watch;

/// Configuration for one [`CloudSse`] connection.
#[derive(Clone)]
pub struct CloudSseConfig {
    /// `GET` URL, e.g. `https://cloud.tokenfuse.example/v1/stream`.
    pub url: String,
    /// Bearer token sent as `Authorization: Bearer <token>` (07 §4.2).
    pub bearer_token: String,
    /// Delay before the first reconnect attempt after a failure; doubles on
    /// each consecutive failed attempt up to `max_backoff`.
    pub initial_backoff: Duration,
    /// Backoff cap: never wait longer than this between attempts.
    pub max_backoff: Duration,
    /// Give up (end the background loop) after this many *consecutive*
    /// attempts that fail to yield even one decoded event. `None` retries
    /// forever. Resets to 0 the moment any event is decoded, so a healthy
    /// connection that later drops gets a fresh budget rather than a
    /// stacked one; only a stream that never works at all eventually
    /// surfaces a clean error (see [`CloudSse::poll`]).
    pub max_attempts: Option<u32>,
}

// Manual Debug: never print `bearer_token` verbatim. Fail-closed logging
// hygiene (06 §0.5) -- a stray `eprintln!("{cfg:?}")` must not leak a
// credential into logs.
impl std::fmt::Debug for CloudSseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudSseConfig")
            .field("url", &self.url)
            .field("bearer_token", &"<redacted>")
            .field("initial_backoff", &self.initial_backoff)
            .field("max_backoff", &self.max_backoff)
            .field("max_attempts", &self.max_attempts)
            .finish()
    }
}

impl CloudSseConfig {
    /// Sensible Phase-0 defaults: 250ms initial backoff doubling up to a
    /// 30s cap, giving up after 10 straight failed attempts (about a
    /// minute of retrying) if the endpoint never once produces an event.
    pub fn new(url: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            bearer_token: bearer_token.into(),
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(30),
            max_attempts: Some(10),
        }
    }
}

/// How long to wait before the (1-indexed) `attempt`-th reconnect. `attempt
/// == 0` means "about to make the very first connection ever" (or "just had
/// a successful connection"): no wait. Doubles each subsequent attempt, up
/// to `max_backoff`.
fn backoff_delay(config: &CloudSseConfig, attempt: u32) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let shift = attempt.saturating_sub(1).min(31); // 1u32 << 31 never overflows/panics
    let multiplier = 1u32 << shift;
    config
        .initial_backoff
        .saturating_mul(multiplier)
        .min(config.max_backoff)
}

/// A live connector to TokenFuse Cloud's `/v1/stream` (07 §4.2). Construct
/// with [`CloudSse::spawn`]; each call to [`CloudSse::poll`] (the
/// [`EventSource`] contract) drains whatever events have arrived since the
/// last call, never blocking.
pub struct CloudSse {
    id: String,
    events: mpsc::Receiver<RawRecord>,
    stop: watch::Sender<bool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl CloudSse {
    /// Spawn the background connection thread and return immediately; the
    /// first connect attempt happens on that thread, not here, so `spawn`
    /// itself cannot fail on network conditions -- its only `Err` case is
    /// local resource exhaustion (thread/runtime creation).
    pub fn spawn(id: impl Into<String>, config: CloudSseConfig) -> Result<Self> {
        let id = id.into();
        let (tx, rx) = mpsc::channel::<RawRecord>();
        let (stop_tx, stop_rx) = watch::channel(false);

        let thread_name = format!("cloud-sse:{id}");
        let worker = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("genaryx-connectors: cloud_sse: failed to start runtime: {e}");
                        return;
                    }
                };
                runtime.block_on(run_loop(config, tx, stop_rx));
            })
            .map_err(|e| {
                Error::Other(format!("cloud_sse: failed to spawn background thread: {e}"))
            })?;

        Ok(Self {
            id,
            events: rx,
            stop: stop_tx,
            worker: Some(worker),
        })
    }

    /// Signal the background loop to stop and join its thread. Safe to call
    /// more than once (a second call finds `worker` already taken and is a
    /// no-op beyond re-sending the stop signal). Prefer this over relying on
    /// `Drop` when a clean, bounded-time join matters (e.g. in tests): the
    /// worker checks `stop` at every await point (backoff sleeps included),
    /// so it wakes within a poll tick, not after a full backoff.
    pub fn shutdown(&mut self) {
        let _ = self.stop.send(true);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl EventSource for CloudSse {
    fn id(&self) -> &str {
        &self.id
    }

    fn poll(&mut self) -> Result<Vec<RawRecord>> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(record) => out.push(record),
                Err(mpsc::TryRecvError::Empty) => return Ok(out),
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Never drop already-decoded records just because the
                    // background loop has since ended: hand back what we
                    // have now, and only surface the error once a
                    // subsequent poll finds nothing pending at all (06
                    // §0.5: fail closed, but never silently lossy).
                    if out.is_empty() {
                        return Err(Error::Other(format!(
                            "cloud_sse[{}]: background stream ended (retry budget exhausted, or shut down)",
                            self.id
                        )));
                    }
                    return Ok(out);
                }
            }
        }
    }
}

impl Drop for CloudSse {
    fn drop(&mut self) {
        // Best-effort only: `Drop` must never block. A caller that needs a
        // guaranteed, prompt join should call `shutdown()` explicitly.
        let _ = self.stop.send(true);
    }
}

/// Why one connection attempt ended, and what the reconnect loop should
/// carry forward into the next one.
enum ConnectOutcome {
    /// Told to stop mid-attempt; the whole loop should end.
    Stopped,
    /// The receiving end of the record channel is gone (the `CloudSse`
    /// handle was dropped); nothing left to feed, so the whole loop ends.
    ReceiverGone,
    /// The connection ended -- cleanly, by error, or never even
    /// established -- and reconnecting is appropriate.
    Disconnected {
        /// At least one event was decoded and forwarded during this
        /// attempt: resets the backoff/attempt budget in the caller.
        got_event: bool,
        /// The last SSE `id:` seen, if any, carried into the next
        /// attempt's `Last-Event-ID` header.
        newest_id: Option<String>,
    },
}

/// The whole connect / decode / reconnect lifecycle, run to completion on
/// its own current-thread runtime (see the module docs for why). Returns
/// (ending the thread) when told to stop, when the receiving end of `tx` is
/// gone, or when `config.max_attempts` consecutive failed-to-produce-an-event
/// attempts have been made.
async fn run_loop(
    config: CloudSseConfig,
    tx: mpsc::Sender<RawRecord>,
    mut stop: watch::Receiver<bool>,
) {
    let client = match reqwest::Client::builder().build() {
        Ok(client) => client,
        Err(e) => {
            eprintln!("genaryx-connectors: cloud_sse: failed to build http client: {e}");
            return;
        }
    };

    let mut attempt: u32 = 0;
    let mut last_event_id: Option<String> = None;

    loop {
        if *stop.borrow() {
            return;
        }

        if let Some(max) = config.max_attempts
            && attempt >= max
        {
            eprintln!(
                "genaryx-connectors: cloud_sse: giving up on {} after {attempt} consecutive \
                 failed attempts (no event ever decoded)",
                config.url
            );
            return;
        }

        if attempt > 0 && wait_or_stop(backoff_delay(&config, attempt), &mut stop).await {
            return; // Stop fired during backoff.
        }
        attempt += 1;

        match connect_and_stream(&client, &config, &last_event_id, &tx, &mut stop).await {
            ConnectOutcome::Stopped | ConnectOutcome::ReceiverGone => return,
            ConnectOutcome::Disconnected {
                got_event,
                newest_id,
            } => {
                if got_event {
                    attempt = 0;
                }
                if newest_id.is_some() {
                    last_event_id = newest_id;
                }
                // Loop again: reconnect, honoring the backoff/max_attempts
                // checks above.
            }
        }
    }
}

/// Sleep for `delay`, waking early (and returning `true`) if `stop` fires
/// first. Returns `false` if the full delay elapsed without a stop signal.
async fn wait_or_stop(delay: Duration, stop: &mut watch::Receiver<bool>) -> bool {
    if delay.is_zero() {
        return *stop.borrow();
    }
    tokio::select! {
        _ = stop.changed() => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

/// One connection attempt: send the request (bearer auth + `Last-Event-ID`
/// if we have one), then read and decode the response body until it ends,
/// forwarding every decoded event as a [`RawRecord`].
async fn connect_and_stream(
    client: &reqwest::Client,
    config: &CloudSseConfig,
    last_event_id: &Option<String>,
    tx: &mpsc::Sender<RawRecord>,
    stop: &mut watch::Receiver<bool>,
) -> ConnectOutcome {
    let mut request = client
        .get(config.url.as_str())
        .bearer_auth(&config.bearer_token)
        .header(reqwest::header::ACCEPT, "text/event-stream");
    if let Some(id) = last_event_id {
        request = request.header("Last-Event-ID", id.clone());
    }

    let response = tokio::select! {
        _ = stop.changed() => return ConnectOutcome::Stopped,
        result = request.send() => result,
    };

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            eprintln!("genaryx-connectors: cloud_sse: connect failed: {e}");
            return ConnectOutcome::Disconnected {
                got_event: false,
                newest_id: None,
            };
        }
    };

    if !response.status().is_success() {
        eprintln!(
            "genaryx-connectors: cloud_sse: unexpected status {} from {}",
            response.status(),
            config.url
        );
        return ConnectOutcome::Disconnected {
            got_event: false,
            newest_id: None,
        };
    }

    let mut decoder = SseDecoder::new();
    let mut stream = response.bytes_stream();
    let mut got_event = false;
    let mut newest_id = last_event_id.clone();

    loop {
        let next = tokio::select! {
            _ = stop.changed() => return ConnectOutcome::Stopped,
            item = stream.next() => item,
        };

        match next {
            Some(Ok(bytes)) => {
                for event in decoder.feed(&bytes) {
                    got_event = true;
                    if event.id.is_some() {
                        newest_id = event.id.clone();
                    }
                    let record = RawRecord {
                        raw: event.data,
                        file: None,
                        offset: None,
                    };
                    if tx.send(record).is_err() {
                        return ConnectOutcome::ReceiverGone;
                    }
                }
            }
            Some(Err(e)) => {
                eprintln!("genaryx-connectors: cloud_sse: stream read error: {e}");
                return ConnectOutcome::Disconnected {
                    got_event,
                    newest_id,
                };
            }
            None => {
                // Clean end of stream: the server closed the connection.
                return ConnectOutcome::Disconnected {
                    got_event,
                    newest_id,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(initial_ms: u64, max_ms: u64) -> CloudSseConfig {
        CloudSseConfig {
            url: "http://example.invalid/v1/stream".to_string(),
            bearer_token: "secret-token".to_string(),
            initial_backoff: Duration::from_millis(initial_ms),
            max_backoff: Duration::from_millis(max_ms),
            max_attempts: Some(10),
        }
    }

    #[test]
    fn first_attempt_never_waits() {
        assert_eq!(backoff_delay(&cfg(100, 10_000), 0), Duration::ZERO);
    }

    #[test]
    fn backoff_doubles_each_consecutive_attempt() {
        let c = cfg(100, 10_000);
        assert_eq!(backoff_delay(&c, 1), Duration::from_millis(100));
        assert_eq!(backoff_delay(&c, 2), Duration::from_millis(200));
        assert_eq!(backoff_delay(&c, 3), Duration::from_millis(400));
        assert_eq!(backoff_delay(&c, 4), Duration::from_millis(800));
    }

    #[test]
    fn backoff_is_capped_at_max_backoff() {
        let c = cfg(100, 500);
        assert_eq!(backoff_delay(&c, 1), Duration::from_millis(100));
        assert_eq!(backoff_delay(&c, 2), Duration::from_millis(200));
        assert_eq!(backoff_delay(&c, 3), Duration::from_millis(400));
        assert_eq!(backoff_delay(&c, 4), Duration::from_millis(500), "capped");
        assert_eq!(
            backoff_delay(&c, 20),
            Duration::from_millis(500),
            "stays capped"
        );
    }

    #[test]
    fn debug_never_prints_the_bearer_token() {
        let c = cfg(1, 1);
        let printed = format!("{c:?}");
        assert!(!printed.contains("secret-token"));
        assert!(printed.contains("<redacted>"));
    }
}
