//! Drills-panel environment discovery: the `mockryx` binary, the TokenFuse
//! gateway URL (+ an optional bearer), and a default scenario directory.
//!
//! Like `memory::env` (see its own doc comment for the shared rationale),
//! BOTH the binary and the gateway URL gate readiness together here (see
//! [`discover`]): the Drills panel has exactly one function - run a drill -
//! which needs both, so there is no partial-functionality shape to split
//! them over the way Identity splits its optional Rescan binary from its
//! required Idryx URL. `scenario_dir` is looser: a best-effort starting
//! point for the operator's editable field (mirrors
//! `crypto::env::ResolvedEnv::default_target`'s identical "a starting point,
//! not an authority" role) - absent, it simply does not gate [`discover`] at
//! all, since the operator can always type a path.
//!
//! ## The binary: no standard install path exists yet (unlike qryx/idryx)
//! Tried in order, first hit wins:
//! 1. `~/.taipan/bin/mockryx` - the SAME well-known convention
//!    `crypto::env`/`identity::state` use for `qryx`/`idryx`, in case an
//!    operator symlinks it there.
//! 2. `~/Development/mockryx/bin/mockryx` - a local checkout's own build
//!    output, matching `go build -o bin/mockryx ./cmd/mockryx` exactly
//!    (mockryx's own Makefile convention, also how
//!    `crates/connectors/tests/exit_gate_test.rs::build_mockryx` builds it;
//!    docs/PHASE4.md grounds Mockryx from `~/Development/mockryx`).
//!
//! ## The gateway: the SAME taipan descriptor identity/quality/money read
//! `services.gateway.url` (NOT `services.cloud`, which is TokenFuse Cloud's
//! separate admin API Money reads off the same file - ground-truthed against
//! `~/Development/taipan/src/descriptor.rs`'s own doc comment:
//! `services:{gateway:{url,mode}, cloud:{url}, wardryx:{url}?, idryx:{url}?}`).
//! An optional bearer rides along: the descriptor's `KeysSection` has no
//! gateway-specific secret at all (only `cloud_admin_ref`/`cloud_viewer_ref`/
//! `wardryx_admin_ref`/`wardryx_viewer_ref` - the gateway is loopback-only
//! and mockryx's own `--api-key` is documented as inert against it,
//! `crates/connectors/src/mockryx.rs`'s module doc), so this best-effort
//! reuses `keys.cloud_admin_ref` (the SAME ref Money's admin bearer resolves
//! off the SAME descriptor) purely as forward-compatible plumbing - `None`
//! here is the honest, common case, never a blocker on the gateway itself
//! resolving.
//!
//! ## The scenario directory
//! `~/Development/mockryx/scenarios` - the checkout's shipped scenarios
//! (`crates/connectors/src/mockryx.rs`'s module doc: "the mockryx checkout's
//! shipped `scenarios/` is the usual one"), when that directory exists.
//!
//! Never panics: every filesystem/JSON step is a `?`-chained `Option`, so one
//! malformed descriptor or absent tier falls through rather than taking down
//! discovery.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a [`ResolvedEnv`]'s gateway came from, surfaced to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EnvSource {
    /// Discovered from `~/.taipan/environments/<name>.json`.
    Taipan { name: String },
}

/// A fully-resolved place to run drills from: the `mockryx` binary, the
/// gateway to rehearse against, an optional bearer, and a best-effort
/// starting scenario directory.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: EnvSource,
    pub mockryx_bin: PathBuf,
    pub gateway_url: String,
    pub api_key: Option<String>,
    /// A starting point for the operator's editable scenario-dir field, not
    /// a claim that this is THE scenario directory - see this module's doc
    /// comment.
    pub scenario_dir: Option<PathBuf>,
}

// ---- descriptor / keyfile wire shapes (read-only mirror, see money::env) --

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Default, Deserialize)]
struct DescriptorKeys {
    #[serde(default)]
    cloud_admin_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
    #[serde(default)]
    keys: DescriptorKeys,
}

#[derive(Debug, Deserialize)]
struct KeyFile {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

/// Resolve the Drills panel's environment: BOTH the `mockryx` binary and a
/// `services.gateway` descriptor entry must resolve, or this is `None` for a
/// clean "no drills plane" state - see this module's doc comment for why the
/// two are not independently gated.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    let mockryx_bin = discover_bin()?;
    let (source, gateway_url, api_key) = discover_gateway()?;
    Some(ResolvedEnv {
        source,
        mockryx_bin,
        gateway_url,
        api_key,
        scenario_dir: discover_scenario_dir(),
    })
}

// ---- binary discovery ---------------------------------------------------

fn discover_bin() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;

    let well_known = well_known_bin_path(&home);
    if well_known.is_file() {
        return Some(well_known);
    }

    let checkout = checkout_bin_path(&home);
    if checkout.is_file() {
        return Some(checkout);
    }

    None
}

fn well_known_bin_path(home: &Path) -> PathBuf {
    home.join(".taipan").join("bin").join("mockryx")
}

fn checkout_bin_path(home: &Path) -> PathBuf {
    home.join("Development")
        .join("mockryx")
        .join("bin")
        .join("mockryx")
}

// ---- gateway discovery ---------------------------------------------------

fn discover_gateway() -> Option<(EnvSource, String, Option<String>)> {
    let dir = genaryx_core::taipan_home::environments_dir()?;
    discover_taipan_gateway_in(&dir)
}

/// Testable core of the descriptor path: scan `environments_dir` for
/// descriptor files (newest last-modified first), and return the first one
/// that yields a usable gateway URL. A descriptor's `keys.cloud_admin_ref`
/// resolving to a real secret is bundled along, best-effort, as `api_key`.
fn discover_taipan_gateway_in(
    environments_dir: &Path,
) -> Option<(EnvSource, String, Option<String>)> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - identical filter to
/// `money::env::list_descriptor_paths`.
fn list_descriptor_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            name.ends_with(".json") && !name.ends_with(".keys.json") && !name.ends_with(".pid.json")
        })
        .collect()
}

fn modified_time(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// Load and resolve one descriptor: read `services.gateway.url` (required -
/// `None` at this step falls through to the next candidate), then
/// best-effort follow `keys.cloud_admin_ref` to a bearer in the sibling
/// `<name>.keys.json` - any failure along THAT sub-path (no ref, no keyfile,
/// no matching secret) simply yields `api_key: None`, never blocking the
/// gateway resolution it rides along with (see this module's doc comment for
/// why there is no dedicated gateway secret to look for instead).
fn try_load_descriptor(path: &Path) -> Option<(EnvSource, String, Option<String>)> {
    let bytes = std::fs::read(path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
    let gateway_url = descriptor.services.get("gateway")?.url.clone();
    let api_key = resolve_admin_bearer(path, &descriptor);
    Some((
        EnvSource::Taipan {
            name: descriptor.name,
        },
        gateway_url,
        api_key,
    ))
}

/// Best-effort `keys.cloud_admin_ref` -> sibling keyfile secret, mirroring
/// `money::env::try_load_descriptor`'s identical resolution step - `None` at
/// any point (no ref on this descriptor, no keyfile, no matching secret
/// label) rather than an error, since this whole value is optional plumbing.
fn resolve_admin_bearer(descriptor_path: &Path, descriptor: &Descriptor) -> Option<String> {
    let admin_ref = descriptor.keys.cloud_admin_ref.as_ref()?;
    let label = admin_ref.rsplit('/').next()?;
    let keys_path = descriptor_path.with_file_name(format!("{}.keys.json", descriptor.name));
    let key_bytes = std::fs::read(&keys_path).ok()?;
    let keyfile: KeyFile = serde_json::from_slice(&key_bytes).ok()?;
    keyfile.secrets.get(label).cloned()
}

// ---- scenario directory discovery ---------------------------------------

fn discover_scenario_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let candidate = PathBuf::from(home)
        .join("Development")
        .join("mockryx")
        .join("scenarios");
    candidate.is_dir().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-drills-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- binary discovery ----

    #[test]
    fn well_known_bin_path_ends_with_the_expected_relative_shape() {
        let home = PathBuf::from("/home/op");
        assert_eq!(
            well_known_bin_path(&home),
            PathBuf::from("/home/op/.taipan/bin/mockryx")
        );
    }

    #[test]
    fn checkout_bin_path_ends_with_the_expected_relative_shape() {
        let home = PathBuf::from("/home/op");
        assert_eq!(
            checkout_bin_path(&home),
            PathBuf::from("/home/op/Development/mockryx/bin/mockryx")
        );
    }

    #[test]
    fn discover_bin_never_panics() {
        let _ = discover_bin();
    }

    // ---- gateway discovery ----

    #[test]
    fn empty_directory_yields_no_candidate() {
        let dir = unique_dir("empty");
        std::fs::create_dir_all(&dir).expect("create dir");
        assert!(discover_taipan_gateway_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_yields_no_candidate_not_a_panic() {
        let dir = unique_dir("missing").join("nested").join("deeper");
        assert!(discover_taipan_gateway_in(&dir).is_none());
    }

    #[test]
    fn ignores_keys_json_and_pid_json_as_descriptor_candidates() {
        let dir = unique_dir("siblings");
        write(
            &dir.join("p1full.keys.json"),
            r#"{"name":"p1full","secrets":{}}"#,
        );
        write(
            &dir.join("p1full.pid.json"),
            r#"{"name":"p1full","processes":[]}"#,
        );
        assert!(discover_taipan_gateway_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_descriptor_with_no_gateway_service_falls_through() {
        let dir = unique_dir("no-gateway");
        write(
            &dir.join("plain.json"),
            r#"{"name":"plain","services":{"cloud":{"url":"http://x"}}}"#,
        );
        assert!(discover_taipan_gateway_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_a_gateway_url_with_no_keys_section_and_no_api_key() {
        // The key case this whole module diverges from money::env over: a
        // descriptor with a gateway but no cloud_admin_ref (or no matching
        // keyfile) still resolves the gateway - the admin bearer is
        // optional plumbing, never a blocker.
        let dir = unique_dir("no-keys");
        write(
            &dir.join("plain.json"),
            r#"{"name":"plain","services":{"gateway":{"url":"http://127.0.0.1:41000","mode":"enforce"}}}"#,
        );
        let (source, gateway_url, api_key) =
            discover_taipan_gateway_in(&dir).expect("must resolve on the gateway url alone");
        assert_eq!(
            source,
            EnvSource::Taipan {
                name: "plain".to_string()
            }
        );
        assert_eq!(gateway_url, "http://127.0.0.1:41000");
        assert_eq!(api_key, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_a_gateway_url_and_a_best_effort_api_key_when_the_keyfile_matches() {
        let dir = unique_dir("with-keys");
        write(
            &dir.join("p1full.json"),
            r#"{
                "name": "p1full",
                "services": {
                    "gateway": {"url": "http://127.0.0.1:41000", "mode": "enforce"},
                    "cloud": {"url": "http://127.0.0.1:41001"}
                },
                "keys": {"cloud_admin_ref": "taipan/p1full/cloud_admin"}
            }"#,
        );
        write(
            &dir.join("p1full.keys.json"),
            r#"{"name":"p1full","secrets":{"cloud_admin":"tp_deadbeef:taipan-p1full:admin"}}"#,
        );

        let (source, gateway_url, api_key) =
            discover_taipan_gateway_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(
            source,
            EnvSource::Taipan {
                name: "p1full".to_string()
            }
        );
        assert_eq!(gateway_url, "http://127.0.0.1:41000");
        assert_eq!(api_key.as_deref(), Some("tp_deadbeef:taipan-p1full:admin"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cloud_admin_ref_with_no_matching_keyfile_still_resolves_the_gateway() {
        let dir = unique_dir("orphan-ref");
        write(
            &dir.join("orphan.json"),
            r#"{"name":"orphan","services":{"gateway":{"url":"http://127.0.0.1:1"}},
                "keys":{"cloud_admin_ref":"taipan/orphan/cloud_admin"}}"#,
        );
        // No sibling keyfile at all.
        let (_, gateway_url, api_key) =
            discover_taipan_gateway_in(&dir).expect("gateway must still resolve");
        assert_eq!(gateway_url, "http://127.0.0.1:1");
        assert_eq!(api_key, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_descriptor_wins_when_multiple_environments_exist() {
        let dir = unique_dir("multi");
        write(
            &dir.join("older.json"),
            r#"{"name":"older","services":{"gateway":{"url":"http://127.0.0.1:1"}}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            r#"{"name":"newer","services":{"gateway":{"url":"http://127.0.0.1:2"}}}"#,
        );

        let (source, gateway_url, _) =
            discover_taipan_gateway_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            source,
            EnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(gateway_url, "http://127.0.0.1:2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- scenario directory discovery ----

    #[test]
    fn discover_scenario_dir_never_panics() {
        let _ = discover_scenario_dir();
    }

    #[test]
    fn discover_never_panics() {
        let _ = discover();
    }
}
