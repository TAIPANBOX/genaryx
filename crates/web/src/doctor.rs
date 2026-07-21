//! Why a plane is not reading your stack, said out loud.
//!
//! Every plane resolves its own environment through a chain of `?` on
//! `Option`, which is right for the plane (any single missing piece means it
//! cannot safely pretend to be configured) but leaves the operator with one
//! undifferentiated answer: "no environment". That answer is identical whether
//! there is no stack at all, or a perfectly healthy Wardryx that this console
//! simply has no admin key for. `bus.rs` already makes exactly this argument
//! about demo-versus-unavailable: collapsing two different states is how a
//! broken console ends up looking like a working one. The same argument
//! applies here, and this module is the answer to it.
//!
//! The verdict for each plane comes from calling that plane's own
//! `env::discover()`, so this can never disagree with what the console
//! actually does. Only the EXPLANATION for a failure uses the table below, and
//! only after the plane itself has already said no.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What one plane needs out of a descriptor, for explaining a refusal.
///
/// Deliberately only used to explain, never to decide: the decision is always
/// the plane's own `discover()`. If a plane changes its requirements, the
/// worst this can do is give a slightly stale hint next to a correct verdict.
struct Needs {
    plane: &'static str,
    /// `services.<key>`, if the plane reads one.
    service: Option<&'static str>,
    /// `keys.<key>`, if the plane needs a bearer token.
    key_ref: Option<&'static str>,
    /// The service's url field names a file on disk rather than an endpoint.
    service_is_path: bool,
}

const NEEDS: &[Needs] = &[
    Needs {
        plane: "money",
        service: Some("cloud"),
        key_ref: Some("cloud_admin_ref"),
        service_is_path: false,
    },
    Needs {
        plane: "policy",
        service: Some("wardryx"),
        key_ref: Some("wardryx_admin_ref"),
        service_is_path: false,
    },
    Needs {
        plane: "identity",
        service: Some("idryx"),
        key_ref: None,
        service_is_path: false,
    },
    Needs {
        plane: "drills",
        service: Some("gateway"),
        key_ref: Some("cloud_admin_ref"),
        service_is_path: false,
    },
    Needs {
        plane: "quality",
        service: Some("verdryx"),
        key_ref: None,
        service_is_path: true,
    },
    Needs {
        plane: "memory",
        service: Some("engram"),
        key_ref: None,
        service_is_path: true,
    },
    Needs {
        plane: "crypto",
        service: None,
        key_ref: None,
        service_is_path: false,
    },
    Needs {
        plane: "evidence",
        service: None,
        key_ref: None,
        service_is_path: false,
    },
];

#[derive(Debug, Default, Deserialize)]
struct DescriptorEvents {
    #[serde(default)]
    dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    #[serde(default)]
    events: DescriptorEvents,
    #[serde(default)]
    services: BTreeMap<String, DescriptorService>,
    #[serde(default)]
    keys: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
struct KeyFile {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

/// One line of the report.
pub struct Finding {
    pub plane: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// The whole report.
pub struct Report {
    pub environments_dir: Option<PathBuf>,
    pub descriptor: Option<PathBuf>,
    pub findings: Vec<Finding>,
}

impl Report {
    /// True when every plane resolved.
    pub fn all_ok(&self) -> bool {
        self.findings.iter().all(|f| f.ok)
    }
}

/// `$TAIPAN_HOME/environments`, else `~/.taipan/environments`. Same rule the
/// planes use, so this looks exactly where they look.
fn environments_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("TAIPAN_HOME") {
        return Some(PathBuf::from(home).join("environments"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".taipan").join("environments"))
}

/// The newest descriptor, matching every plane's own newest-first tie-break.
fn newest_descriptor(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.ends_with(".json") && !name.ends_with(".keys.json") && !name.ends_with(".pid.json")
        })
        .collect();
    candidates.sort_by_key(|p| {
        std::cmp::Reverse(
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        )
    });
    candidates.into_iter().next()
}

/// How a plane resolved, which matters as much as whether it did.
///
/// A plane that only worked because a fallback tier caught it is NOT the same
/// as one reading the descriptor you wrote, and reporting both as a bare "ok"
/// is its own kind of lie: the operator concludes the descriptor is right when
/// something else is quietly carrying it, and finds out when the fallback goes
/// away. That is precisely the trap this module exists to close, so it must
/// not reintroduce it one level up.
enum How {
    No,
    Descriptor(String),
    Fallback(&'static str),
}

/// Asked of the plane itself, never guessed.
fn how(plane: &str) -> How {
    use genaryx_api as api;
    match plane {
        "money" => match api::money::env::discover() {
            None => How::No,
            Some(r) => match r.source {
                api::money::env::EnvSource::Taipan { name } => How::Descriptor(name),
                api::money::env::EnvSource::EnvFallback => {
                    How::Fallback("TOKENFUSE_CLOUD_ADMIN_KEY against 127.0.0.1:8080")
                }
            },
        },
        "policy" => match api::policy::env::discover() {
            None => How::No,
            Some(r) => match r.source {
                api::policy::env::EnvSource::Taipan { name } => How::Descriptor(name),
                api::policy::env::EnvSource::EnvFallback => {
                    How::Fallback("WARDRYX_URL / WARDRYX_ADMIN_KEY")
                }
            },
        },
        "quality" => match api::quality::env::discover() {
            None => How::No,
            Some(r) => match r.source {
                api::quality::env::EnvSource::Taipan { name } => How::Descriptor(name),
                api::quality::env::EnvSource::WellKnown => {
                    How::Fallback("the well-known ~/.taipan/verdryx.db")
                }
            },
        },
        "memory" => match api::memory::env::discover() {
            None => How::No,
            Some(r) => match r.source {
                api::memory::env::EnvSource::Taipan { name } => How::Descriptor(name),
                api::memory::env::EnvSource::WellKnown => {
                    How::Fallback("the well-known ~/.taipan store")
                }
            },
        },
        // These two resolve only from a descriptor, so resolving at all means
        // the descriptor was read.
        "identity" => match api::identity::env::discover() {
            None => How::No,
            Some(r) => match r.source {
                api::identity::env::EnvSource::Taipan { name } => How::Descriptor(name),
            },
        },
        "drills" => match api::drills::env::discover() {
            None => How::No,
            Some(r) => match r.source {
                api::drills::env::EnvSource::Taipan { name } => How::Descriptor(name),
            },
        },
        // Local tools rather than a descriptor: a resolved binary, no source
        // to distinguish.
        "crypto" => match api::crypto::env::discover() {
            Some(_) => How::Descriptor("local qryx binary".into()),
            None => How::No,
        },
        "evidence" => match api::evidence::env::discover_qryx() {
            Some(_) => How::Descriptor("local binaries".into()),
            None => How::No,
        },
        _ => How::No,
    }
}

/// Inspect the descriptor and name the first thing this plane is missing.
fn explain(needs: &Needs, desc: Option<&Descriptor>, keys: &KeyFile) -> String {
    let Some(desc) = desc else {
        return "no descriptor found, and no environment variables set either. \
                Write one (see docs/WEB-SHELL.md) or run `taipan up`."
            .into();
    };
    if let Some(svc) = needs.service {
        let Some(entry) = desc.services.get(svc) else {
            return format!(
                "descriptor '{}' has no services.{svc}. Add it, or this plane cannot know where to look.",
                desc.name
            );
        };
        if needs.service_is_path && !Path::new(&entry.url).is_file() {
            return format!(
                "services.{svc}.url is '{}', which is not an existing file. \
                 This field is a FILESYSTEM PATH here, not an endpoint.",
                entry.url
            );
        }
    }
    if let Some(kref) = needs.key_ref {
        let Some(reference) = desc.keys.get(kref) else {
            return format!(
                "descriptor '{}' has the service but no keys.{kref}, so there is no admin token to use. \
                 The service being healthy is not enough.",
                desc.name
            );
        };
        let label = reference.rsplit('/').next().unwrap_or(reference);
        if !keys.secrets.contains_key(label) {
            return format!(
                "keys.{kref} points at '{label}', which is absent from the sibling {}.keys.json secrets.",
                desc.name
            );
        }
    }
    "the descriptor looks complete for this plane, so the refusal is elsewhere: \
     check the service is actually reachable from this host."
        .into()
}

/// Run every check.
pub fn run() -> Report {
    let dir = environments_dir();
    let descriptor_path = dir.as_deref().and_then(newest_descriptor);
    let descriptor: Option<Descriptor> = descriptor_path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok());
    let keys: KeyFile = descriptor_path
        .as_ref()
        .zip(descriptor.as_ref())
        .map(|(p, d)| p.with_file_name(format!("{}.keys.json", d.name)))
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    let mut findings = Vec::new();

    // The bus first: it decides whether anything you see is real.
    let bus_dir = descriptor
        .as_ref()
        .and_then(|d| d.events.dir.as_deref())
        .filter(|s| !s.trim().is_empty());
    findings.push(Finding {
        plane: "bus",
        ok: bus_dir.is_some(),
        detail: match bus_dir {
            Some(d) => format!("live, tailing {d}"),
            None => "no events.dir in the descriptor, so the Bus Explorer runs a \
                     synthetic demo feeder. Everything it shows is invented."
                .into(),
        },
    });

    // Remote is reported but never failed, and that is not an oversight.
    // It has no discoverable environment by design (see
    // `genaryx_api::remote::env`): the WireGuard peer, the SSH target and even
    // the binary path are operator-defined per campaign, and this lookup only
    // pre-fills the form. Marking a missing default as a PROBLEM would fail
    // `doctor` on a box where the operator is about to type the path in, which
    // would teach people to ignore the exit code.
    findings.push(Finding {
        plane: "remote",
        ok: true,
        detail: match genaryx_api::remote::env::discover() {
            Some(p) => format!("wireguard-go default {}", p.display()),
            None => "no wireguard-go found to pre-fill with. Not a fault: set the path \
                     in the Remote panel, along with the peer and SSH target."
                .into(),
        },
    });

    for needs in NEEDS {
        let (ok, detail) = match how(needs.plane) {
            How::Descriptor(name) => (true, format!("resolved from {name}")),
            How::Fallback(what) => (
                false,
                format!(
                    "NOT reading your descriptor: it fell back to {what}. It works, but \
                     the descriptor is wrong or incomplete, and the day the fallback \
                     goes away this plane goes dark. {}",
                    explain(needs, descriptor.as_ref(), &keys)
                ),
            ),
            How::No if needs.service.is_none() => (
                false,
                "could not resolve its local tool. Check the binary is installed and on PATH."
                    .into(),
            ),
            How::No => (false, explain(needs, descriptor.as_ref(), &keys)),
        };
        findings.push(Finding {
            plane: needs.plane,
            ok,
            detail,
        });
    }

    Report {
        environments_dir: dir,
        descriptor: descriptor_path,
        findings,
    }
}

/// Human-readable report, for the `doctor` subcommand.
pub fn print(report: &Report) {
    match &report.environments_dir {
        Some(d) => println!("environments  {}", d.display()),
        None => println!("environments  (neither TAIPAN_HOME nor HOME is set)"),
    }
    match &report.descriptor {
        Some(p) => println!("descriptor    {}", p.display()),
        None => println!("descriptor    none found"),
    }
    println!();
    for f in &report.findings {
        println!(
            "{:<9} {}  {}",
            f.plane,
            if f.ok { "ok     " } else { "PROBLEM" },
            f.detail
        );
    }
    println!();
    if report.all_ok() {
        println!("Everything resolved.");
    } else {
        println!("See docs/WEB-SHELL.md for the descriptor shape.");
    }
}

/// The same findings as startup warnings, so an operator who never runs
/// `doctor` still learns which panels will be empty and why.
pub fn log(report: &Report) {
    for f in report.findings.iter().filter(|f| !f.ok) {
        tracing::warn!(plane = f.plane, "{}", f.detail);
    }
}
