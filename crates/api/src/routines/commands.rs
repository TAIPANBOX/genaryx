//! The Routines tab's two commands (I7b): [`routines_status`] (per-routine
//! summary: installed-as-timer + latest recorded run) and
//! [`routines_history`] (parsed run history, newest first, optionally
//! filtered to one routine and capped at a limit).
//!
//! Both are viewer-safe reads: nothing here writes anything, installs a
//! timer, or runs a routine (see `super`'s module doc for the non-goal,
//! stated plainly). Every call re-reads the local filesystem fresh - no
//! managed state, mirroring `crate::onboard::commands`'s identical
//! "re-reads fresh on every call" contract.
//!
//! [`RoutineRunDto`] mirrors stack-up's `stackup.routine-run/v1` schema
//! (`~/Development/stack-up/README.md`, "The record") field-for-field. Its
//! `status` field is carried as a plain `String`, not a closed Rust enum:
//! the contract names four values (`ok | findings | skipped | error`), but
//! an unrecognized fifth value must still render rather than being rejected
//! outright - the same "honesty over rejection" tolerance
//! `genaryx_connectors::GatewayKeysReport::strict_mode`/`IdryxAlert::severity`
//! already keep for their own open-ended wire strings.

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::env;

/// The five routines `routines.sh` knows about, exact spelling and order as
/// its own `ROUTINE_NAMES` array. A fixed list here, not discovered from the
/// filesystem, so a routine with no record yet (a fresh install, or one
/// that has simply never fired) still gets its own row in
/// [`routines_status`] instead of silently vanishing.
pub const ROUTINE_NAMES: [&str; 5] = [
    "focus-export",
    "qryx-trend",
    "verdryx-drift",
    "idryx-detect",
    "mockryx-drill",
];

/// [`routines_history`]'s default cap when the caller does not pass `limit`.
const DEFAULT_HISTORY_LIMIT: u32 = 200;
/// [`routines_history`]'s hard ceiling, regardless of what the caller asks
/// for - `history.ndjson` is unbounded (append-only, never rotated by
/// `routines.sh`), so an unbounded ask must not become an unbounded read.
const MAX_HISTORY_LIMIT: u32 = 1000;

// ============================================================================
// The stable v1 record
// ============================================================================

/// Mirrors stack-up's `stackup.routine-run/v1` record exactly (see this
/// module's doc comment). `#[serde(default)]` on the three optional fields
/// is a defensive extra: every record `routines.sh` actually writes carries
/// all eight keys (nulling the unused ones), but a missing key parsing as
/// `None` rather than a hard failure costs nothing and is one more honest
/// tolerance in the same spirit as the `status` field's open string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutineRunDto {
    pub schema: String,
    pub routine: String,
    pub started_at: String,
    pub finished_at: String,
    pub exit_code: i64,
    /// `ok | findings | skipped | error` per the contract - see this
    /// module's doc comment for why this stays a plain `String`.
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub artifact: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

// ============================================================================
// routines_status
// ============================================================================

/// One routine's row in [`RoutinesStatusDto`].
#[derive(Debug, Clone, Serialize)]
pub struct RoutineSummaryDto {
    pub name: String,
    /// Whether `installed.txt` names a timer/unit file for this routine -
    /// see [`is_installed`] for the exact match rule.
    pub installed: bool,
    /// `None` when the routine has never run at all (no
    /// `status/<name>.json` on disk yet) - a normal, honest "never run"
    /// state, not an error.
    pub latest: Option<RoutineRunDto>,
    /// Set INSTEAD of `latest` when `status/<name>.json` exists but could
    /// not be read or parsed - a per-routine note, never a whole-command
    /// failure (see [`routines_status`]'s doc comment).
    pub latest_error: Option<String>,
}

/// `routines_status`'s result: the resolved routines directory (+ whether it
/// exists), and one [`RoutineSummaryDto`] per entry in [`ROUTINE_NAMES`].
#[derive(Debug, Clone, Serialize)]
pub struct RoutinesStatusDto {
    pub routines_dir: String,
    pub routines_dir_exists: bool,
    pub routines: Vec<RoutineSummaryDto>,
}

/// Whole-tab summary. Never fails: a missing routines directory renders as
/// `routines_dir_exists: false` with every routine reporting "never run,
/// not installed" (a clean, expected state right after a fresh `stack-up`
/// clone, before `routines.sh` has ever run) rather than an error, and an
/// unparseable `status/<name>.json` lands on that ONE routine's
/// `latest_error` rather than failing this whole command.
pub async fn routines_status() -> Result<RoutinesStatusDto, ()> {
    let resolved = env::discover();
    let installed_lines = read_installed_manifest(&resolved.path);

    let routines = ROUTINE_NAMES
        .iter()
        .map(|&name| {
            let (latest, latest_error) = read_latest_status(&resolved.path, name);
            RoutineSummaryDto {
                name: name.to_string(),
                installed: is_installed(name, &installed_lines),
                latest,
                latest_error,
            }
        })
        .collect();

    Ok(RoutinesStatusDto {
        routines_dir: resolved.path.display().to_string(),
        routines_dir_exists: resolved.exists,
        routines,
    })
}

/// Every non-blank line of `<routines_dir>/installed.txt`, or an empty list
/// when the manifest does not exist (no timers ever installed) or cannot be
/// read - never a failure, mirroring [`routines_status`]'s own tolerance.
fn read_installed_manifest(routines_dir: &Path) -> Vec<String> {
    let path = routines_dir.join("installed.txt");
    std::fs::read_to_string(path)
        .map(|body| {
            body.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether ANY manifest line names this routine's own unit file.
///
/// `routines.sh`'s `install_systemd_unit`/`install_launchd_unit` write
/// `stack-up-routine-<name>.{service,timer}` (systemd) or
/// `dev.taipanbox.stack-up.routine-<name>.plist` (launchd) - every shape
/// contains the literal substring `routine-<name>`, so a plain substring
/// check is exact rather than approximate. Safe against false positives
/// across the fixed [`ROUTINE_NAMES`] list specifically because none of the
/// five names is a prefix of another (`focus-export`, `qryx-trend`,
/// `verdryx-drift`, `idryx-detect`, `mockryx-drill`), so `routine-idryx-detect`
/// can never be mistaken for a different routine's line.
fn is_installed(name: &str, manifest_lines: &[String]) -> bool {
    let needle = format!("routine-{name}");
    manifest_lines.iter().any(|line| line.contains(&needle))
}

/// Read and parse `<routines_dir>/status/<name>.json` into `(latest,
/// latest_error)` - exactly one of the two is ever `Some`:
///
/// - file absent -> `(None, None)`, "never run".
/// - file present but unreadable/unparseable -> `(None, Some(message))`.
/// - file present and parses -> `(Some(record), None)`.
fn read_latest_status(routines_dir: &Path, name: &str) -> (Option<RoutineRunDto>, Option<String>) {
    let path = routines_dir.join("status").join(format!("{name}.json"));
    match std::fs::read_to_string(&path) {
        Ok(body) => match serde_json::from_str::<RoutineRunDto>(body.trim()) {
            Ok(record) => (Some(record), None),
            Err(e) => (
                None,
                Some(format!("could not parse {}: {e}", path.display())),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
        Err(e) => (
            None,
            Some(format!("could not read {}: {e}", path.display())),
        ),
    }
}

// ============================================================================
// routines_history
// ============================================================================

/// `routines_history`'s result.
#[derive(Debug, Clone, Serialize)]
pub struct RoutinesHistoryDto {
    /// Newest first (see [`assemble_history`]).
    pub records: Vec<RoutineRunDto>,
    /// Count of lines in `history.ndjson` that were not valid JSON / did not
    /// match [`RoutineRunDto`]'s shape - truncation/corruption is reported
    /// here, never silent.
    pub skipped_lines: u32,
    pub routines_dir: String,
    pub history_file_exists: bool,
}

/// Parsed run history, newest first, optionally filtered to one routine and
/// capped at `limit` (default [`DEFAULT_HISTORY_LIMIT`], hard-capped at
/// [`MAX_HISTORY_LIMIT`] regardless of what is asked for). Never fails: a
/// missing routines dir or history file renders as an empty `records` list
/// with `history_file_exists: false`, and a malformed line is counted in
/// `skipped_lines` rather than failing the whole read.
pub async fn routines_history(
    routine: Option<String>,
    limit: Option<u32>,
) -> Result<RoutinesHistoryDto, ()> {
    let resolved = env::discover();
    let history_path = resolved.path.join("history.ndjson");
    let history_file_exists = history_path.is_file();
    let capped_limit = limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .min(MAX_HISTORY_LIMIT);

    let (records, skipped_lines) = if history_file_exists {
        match std::fs::read_to_string(&history_path) {
            Ok(body) => assemble_history(&body, routine.as_deref(), capped_limit),
            // Exists but could not be read (e.g. a permissions oddity) - as
            // honest as this command gets without a dedicated error field
            // for it: reported via `history_file_exists`/an empty list
            // rather than failing the command outright.
            Err(_) => (Vec::new(), 0),
        }
    } else {
        (Vec::new(), 0)
    };

    Ok(RoutinesHistoryDto {
        records,
        skipped_lines,
        routines_dir: resolved.path.display().to_string(),
        history_file_exists,
    })
}

/// The whole parse -> reverse (newest first) -> filter -> cap pipeline over
/// a raw `history.ndjson` body. Pure and filesystem-free so it is directly
/// unit-testable against literal fixtures, no tempdir needed - mirrors how
/// `credentials::commands::status_dto`/`admission`'s own pure helpers are
/// each unit-tested in isolation from any real gateway.
///
/// `history.ndjson` is append-only, newest LAST, so this reverses before
/// returning - every other newest/worst-first list this console renders
/// (`quality_list_run_summaries`, the Money panel's runs table,
/// `CredentialsKeysTable`'s worst-first sort) puts the most relevant row
/// first, and this is no exception. A blank line is skipped silently (not
/// counted as malformed - it is not corruption, just incidental
/// whitespace); a non-blank line that fails to parse as a [`RoutineRunDto`]
/// increments `skipped_lines` instead of aborting the whole read.
fn assemble_history(body: &str, routine: Option<&str>, limit: u32) -> (Vec<RoutineRunDto>, u32) {
    let mut skipped = 0u32;
    let mut records = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<RoutineRunDto>(trimmed) {
            Ok(record) => records.push(record),
            Err(_) => skipped += 1,
        }
    }
    records.reverse();
    if let Some(name) = routine {
        records.retain(|r| r.routine == name);
    }
    records.truncate(limit as usize);
    (records, skipped)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch(label: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "genaryx-routines-test-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn record(routine: &str, started_at: &str, status: &str) -> RoutineRunDto {
        RoutineRunDto {
            schema: "stackup.routine-run/v1".to_string(),
            routine: routine.to_string(),
            started_at: started_at.to_string(),
            finished_at: started_at.to_string(),
            exit_code: if status == "error" { 1 } else { 0 },
            status: status.to_string(),
            reason: None,
            artifact: None,
            summary: Some(format!("{routine} {status}")),
        }
    }

    fn line(r: &RoutineRunDto) -> String {
        serde_json::to_string(r).expect("serialize fixture record")
    }

    // -- RoutineRunDto wire shape ---------------------------------------

    #[test]
    fn routine_run_dto_round_trips_the_exact_v1_schema_shape() {
        let raw = r#"{"schema":"stackup.routine-run/v1","routine":"idryx-detect","started_at":"2026-07-23T06:37:00Z","finished_at":"2026-07-23T06:37:04Z","exit_code":0,"status":"ok","reason":null,"artifact":"out/idryx-detect-latest.json","summary":"3 alert(s)"}"#;
        let parsed: RoutineRunDto = serde_json::from_str(raw).expect("must parse a real record");
        assert_eq!(parsed.routine, "idryx-detect");
        assert_eq!(parsed.exit_code, 0);
        assert_eq!(parsed.status, "ok");
        assert_eq!(parsed.reason, None);
        assert_eq!(
            parsed.artifact.as_deref(),
            Some("out/idryx-detect-latest.json")
        );
        assert_eq!(parsed.summary.as_deref(), Some("3 alert(s)"));
    }

    #[test]
    fn routine_run_dto_tolerates_an_unrecognized_status_value() {
        // Honesty over rejection (this module's doc comment): a value
        // outside ok|findings|skipped|error must still parse, since `status`
        // is a plain String, not a closed enum.
        let raw = r#"{"schema":"stackup.routine-run/v1","routine":"qryx-trend","started_at":"x","finished_at":"y","exit_code":0,"status":"a-future-status","reason":null,"artifact":null,"summary":null}"#;
        let parsed: RoutineRunDto =
            serde_json::from_str(raw).expect("an unknown status must still parse");
        assert_eq!(parsed.status, "a-future-status");
    }

    #[test]
    fn routine_run_dto_tolerates_missing_optional_keys() {
        let raw = r#"{"schema":"stackup.routine-run/v1","routine":"focus-export","started_at":"x","finished_at":"y","exit_code":0,"status":"skipped"}"#;
        let parsed: RoutineRunDto =
            serde_json::from_str(raw).expect("missing optional keys must still parse");
        assert_eq!(parsed.reason, None);
        assert_eq!(parsed.artifact, None);
        assert_eq!(parsed.summary, None);
    }

    // -- assemble_history: parse / reverse / filter / cap / malformed-skip --

    #[test]
    fn assemble_history_reverses_to_newest_first() {
        let oldest = record("focus-export", "2026-07-20T06:07:00Z", "ok");
        let middle = record("focus-export", "2026-07-21T06:07:00Z", "ok");
        let newest = record("focus-export", "2026-07-22T06:07:00Z", "ok");
        let body = format!("{}\n{}\n{}\n", line(&oldest), line(&middle), line(&newest));

        let (records, skipped) = assemble_history(&body, None, 200);
        assert_eq!(skipped, 0);
        assert_eq!(
            records
                .iter()
                .map(|r| r.started_at.as_str())
                .collect::<Vec<_>>(),
            vec![
                "2026-07-22T06:07:00Z",
                "2026-07-21T06:07:00Z",
                "2026-07-20T06:07:00Z"
            ]
        );
    }

    #[test]
    fn assemble_history_counts_and_skips_malformed_lines_without_failing() {
        let good_a = record("qryx-trend", "2026-07-21T06:17:00Z", "ok");
        let good_b = record("qryx-trend", "2026-07-22T06:17:00Z", "ok");
        let body = format!(
            "{}\n{{ this is not json\n{}\n\n",
            line(&good_a),
            line(&good_b)
        );

        let (records, skipped) = assemble_history(&body, None, 200);
        assert_eq!(
            skipped, 1,
            "exactly the one malformed line, not the blank line too"
        );
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn assemble_history_filters_to_one_routine_after_reversing() {
        let a1 = record("focus-export", "2026-07-20T06:07:00Z", "ok");
        let b1 = record("qryx-trend", "2026-07-20T06:17:00Z", "ok");
        let a2 = record("focus-export", "2026-07-21T06:07:00Z", "error");
        let body = format!("{}\n{}\n{}\n", line(&a1), line(&b1), line(&a2));

        let (records, skipped) = assemble_history(&body, Some("focus-export"), 200);
        assert_eq!(skipped, 0);
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.routine == "focus-export"));
        // Still newest first within the filtered set.
        assert_eq!(records[0].status, "error");
    }

    #[test]
    fn assemble_history_caps_at_the_given_limit_after_filtering() {
        let mut body = String::new();
        for day in 1..=10 {
            let r = record(
                "verdryx-drift",
                &format!("2026-07-{day:02}T06:27:00Z"),
                "ok",
            );
            body.push_str(&line(&r));
            body.push('\n');
        }
        let (records, _) = assemble_history(&body, None, 3);
        assert_eq!(records.len(), 3);
        // The three newest, in newest-first order.
        assert_eq!(records[0].started_at, "2026-07-10T06:27:00Z");
        assert_eq!(records[1].started_at, "2026-07-09T06:27:00Z");
        assert_eq!(records[2].started_at, "2026-07-08T06:27:00Z");
    }

    #[test]
    fn assemble_history_is_empty_for_an_empty_body() {
        let (records, skipped) = assemble_history("", None, 200);
        assert!(records.is_empty());
        assert_eq!(skipped, 0);
    }

    // -- is_installed --------------------------------------------------

    #[test]
    fn is_installed_matches_launchd_and_systemd_filename_shapes() {
        let manifest = vec![
            "/Users/x/Library/LaunchAgents/dev.taipanbox.stack-up.routine-focus-export.plist"
                .to_string(),
            "/home/x/.config/systemd/user/stack-up-routine-qryx-trend.service".to_string(),
            "/home/x/.config/systemd/user/stack-up-routine-qryx-trend.timer".to_string(),
        ];
        assert!(is_installed("focus-export", &manifest));
        assert!(is_installed("qryx-trend", &manifest));
        assert!(!is_installed("verdryx-drift", &manifest));
        assert!(!is_installed("idryx-detect", &manifest));
        assert!(!is_installed("mockryx-drill", &manifest));
    }

    #[test]
    fn is_installed_is_false_for_an_empty_manifest() {
        assert!(!is_installed("focus-export", &[]));
    }

    #[test]
    fn no_routine_name_is_a_false_positive_substring_of_another() {
        // Guards the exact safety claim `is_installed`'s doc comment makes:
        // a manifest line for any one routine must never register as
        // "installed" for a DIFFERENT routine in the fixed list.
        for &installed_name in ROUTINE_NAMES.iter() {
            let manifest = vec![format!("/some/dir/stack-up-routine-{installed_name}.timer")];
            for &other_name in ROUTINE_NAMES.iter() {
                let expected = other_name == installed_name;
                assert_eq!(
                    is_installed(other_name, &manifest),
                    expected,
                    "installed={installed_name} checked against={other_name}"
                );
            }
        }
    }

    // -- read_installed_manifest / read_latest_status (real tempdir fixtures) --

    #[test]
    fn read_installed_manifest_is_empty_when_the_file_does_not_exist() {
        let dir = scratch("manifest-missing");
        assert!(read_installed_manifest(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_installed_manifest_trims_and_drops_blank_lines() {
        let dir = scratch("manifest-present");
        std::fs::write(
            dir.join("installed.txt"),
            "  /a/stack-up-routine-focus-export.timer  \n\n/b/stack-up-routine-qryx-trend.timer\n",
        )
        .expect("write manifest fixture");
        let lines = read_installed_manifest(&dir);
        assert_eq!(
            lines,
            vec![
                "/a/stack-up-routine-focus-export.timer".to_string(),
                "/b/stack-up-routine-qryx-trend.timer".to_string(),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_latest_status_reports_never_run_for_a_missing_file() {
        let dir = scratch("status-missing");
        let (latest, latest_error) = read_latest_status(&dir, "focus-export");
        assert_eq!(latest, None);
        assert_eq!(latest_error, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_latest_status_parses_a_real_status_file() {
        let dir = scratch("status-real");
        std::fs::create_dir_all(dir.join("status")).expect("create status dir");
        let fixture = record("idryx-detect", "2026-07-23T06:37:00Z", "ok");
        std::fs::write(dir.join("status").join("idryx-detect.json"), line(&fixture))
            .expect("write status fixture");

        let (latest, latest_error) = read_latest_status(&dir, "idryx-detect");
        assert_eq!(latest_error, None);
        assert_eq!(latest, Some(fixture));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_latest_status_reports_an_error_note_for_an_unparseable_file_never_panicking() {
        let dir = scratch("status-broken");
        std::fs::create_dir_all(dir.join("status")).expect("create status dir");
        std::fs::write(
            dir.join("status").join("mockryx-drill.json"),
            "{ not json at all",
        )
        .expect("write broken status fixture");

        let (latest, latest_error) = read_latest_status(&dir, "mockryx-drill");
        assert_eq!(latest, None);
        assert!(latest_error.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- top-level commands (still exercise the real env::discover() path,
    // proving the whole pipeline resolves to something renderable regardless
    // of what this box's real STACK_UP_HOME/HOME happen to be) --

    #[tokio::test]
    async fn routines_status_never_fails_and_always_lists_all_five_routines() {
        let status = routines_status()
            .await
            .expect("routines_status is infallible");
        assert_eq!(status.routines.len(), ROUTINE_NAMES.len());
        let names: Vec<&str> = status.routines.iter().map(|r| r.name.as_str()).collect();
        for expected in ROUTINE_NAMES {
            assert!(
                names.contains(&expected),
                "missing {expected} in routines_status"
            );
        }
    }

    #[tokio::test]
    async fn routines_history_never_fails_and_respects_the_hard_cap() {
        let history = routines_history(None, Some(50_000))
            .await
            .expect("routines_history is infallible");
        assert!(history.records.len() <= MAX_HISTORY_LIMIT as usize);
    }
}
