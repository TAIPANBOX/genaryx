//! `taipan demo` data generator: a realistic event stream mirroring the forms of
//! real validation campaigns, so the product never shows an empty screen (08 §2).
//!
//! Phase-0 stub. Delegated to Sonnet (task #5). Target shapes (09 §5, 08 §2):
//! ~176 calls / 65 runs / 12 blocks; a 34-agent burst; breaker + cache + router
//! effects preserved. Emits one NDJSON file per service into `events_dir`, matching
//! the `taipan up` layout (`~/.taipan/events/<service>.ndjson`, 07 §3/§7).

use crate::error::Result;
use std::path::Path;

/// Generate a demo event stream into `events_dir` (one file per service).
/// Returns the number of events written.
pub fn generate(events_dir: &Path) -> Result<usize> {
    // TODO(sonnet, task#5): synthesize tokenfuse/wardryx/idryx/engram/verdryx/
    // mockryx/qryx lines with real `data` shapes; keep chronological `ts` order.
    let _ = events_dir;
    Ok(0)
}
