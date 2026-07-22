//! Multi-cloud VM inventory: a STRICTLY READ-ONLY connector over each cloud
//! provider's OWN OFFICIAL CLI (`aws`, `gcloud`, `az`) - the AWS/GCP/Azure
//! extension of decision D11's "Hetzner is read-only, v1" (docs/PHASE4.md §4)
//! to the rest of the multi-cloud inventory. Mirrors [`crate::HetznerClient`]'s
//! read-only guarantee: there is deliberately no create/start/stop/resize/
//! delete method on this module at all, by construction, so it cannot mutate
//! cloud infrastructure through it - only ever list what already exists.
//!
//! ## Why shell the official CLI, not each provider's REST API
//!
//! Like [`crate::QryxClient`] (whose machine surface IS its `--format` flag),
//! each provider's own CLI is already the authenticated, JSON-capable surface
//! an operator has on their box - reusing it means this connector never
//! handles a cloud credential directly. It only ever runs the provider's own
//! read/describe/list verb, with `--output`/`--format` JSON:
//!
//! - AWS: `aws ec2 describe-instances --output json`
//! - GCP: `gcloud compute instances list --format=json`
//! - Azure: `az vm list -d --output json`
//!
//! None of these three commands can create, start, stop, resize, or delete
//! anything; each is the provider's own read verb. This connector adds only
//! scoping flags (region/project/subscription/profile) on top.
//!
//! ## Auth is the CLI's problem, not ours
//!
//! This connector never reads or holds a credential itself - each CLI reads
//! its own already-configured auth (`~/.aws/credentials`/`$AWS_PROFILE`,
//! `gcloud auth`, `az login`). A CLI that is not authenticated exits nonzero
//! with a recognizable message; [`CloudCliError::NotAuthenticated`] surfaces
//! that with a short remediation hint instead of the generic
//! [`CloudCliError::Exec`], so a caller can show "run `aws configure`" rather
//! than a raw stderr dump.
//!
//! ## Fail-closed (06 §0.5)
//!
//! No panics, no `unwrap`/`expect`. A missing binary is
//! [`CloudCliError::CliNotFound`]; a nonzero exit whose stderr smells like an
//! auth problem is [`CloudCliError::NotAuthenticated`]; any other nonzero exit
//! is [`CloudCliError::Exec`]; unparseable stdout is [`CloudCliError::Json`].
//! Parsing itself is defensive throughout (raw `serde_json::Value` reads, not
//! a strict wire struct): a row missing an optional field - most notably a VM
//! with no public IP - maps to `None`, never a parse failure for the whole
//! listing.
//!
//! ## Async, off the calling thread, without a new tokio feature
//!
//! [`list_servers`] is `async`, but the actual CLI spawn runs inside
//! [`tokio::task::spawn_blocking`] - the exact bridge
//! `crates/api/src/{crypto,quality,memory,remote}/commands.rs` already use for
//! every other blocking connector call ([`crate::QryxClient`],
//! [`crate::VerdryxClient`], [`crate::SshClient`], ...). `genaryx-connectors`
//! only carries tokio's `rt-multi-thread`/`macros`/`sync`/`time`/`fs`/
//! `io-util` features (no `process`), so this reuses the exact
//! `std::process::Command` capture pattern [`crate::QryxClient`]/
//! [`crate::MockryxClient`] already use rather than adding a new tokio
//! feature for one connector.

use serde::{Deserialize, Serialize};

// ---- provider ---------------------------------------------------------------

/// Which cloud provider's CLI to shell out to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloudProvider {
    Aws,
    Gcp,
    Azure,
}

impl CloudProvider {
    /// The wire spelling, used both as [`CloudServer::provider`] and as the
    /// value [`Self::parse`] accepts back.
    pub fn as_str(self) -> &'static str {
        match self {
            CloudProvider::Aws => "aws",
            CloudProvider::Gcp => "gcp",
            CloudProvider::Azure => "azure",
        }
    }

    /// Parse a provider name, case-insensitively (`"AWS"`, `"Aws"`, `"aws"`
    /// all match). `None` for anything else - callers never default a bad
    /// string into a provider, mirroring [`crate::RelayDeviceKind::parse`]'s
    /// contract exactly.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "aws" => Some(CloudProvider::Aws),
            "gcp" => Some(CloudProvider::Gcp),
            "azure" => Some(CloudProvider::Azure),
            _ => None,
        }
    }

    /// The CLI binary this provider shells out to.
    fn cli_name(self) -> &'static str {
        match self {
            CloudProvider::Aws => "aws",
            CloudProvider::Gcp => "gcloud",
            CloudProvider::Azure => "az",
        }
    }
}

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`list_servers`] call can surface. Fail-closed: a
/// missing binary, an auth problem, any other nonzero exit, and a parse
/// failure are distinct variants, never a panic.
#[derive(Debug, thiserror::Error)]
pub enum CloudCliError {
    /// The provider CLI could not be spawned at all - the OS reported
    /// `NotFound` (not installed / not on PATH). Distinguishing this from
    /// [`Self::Exec`] lets a caller show "install the AWS CLI" rather than a
    /// raw OS error.
    #[error("{cli}: command not found (is it installed and on PATH?)")]
    CliNotFound { cli: String },

    /// The CLI ran but its stderr indicates it is not authenticated. `hint` is
    /// a short, provider-specific remediation (e.g. "run `aws configure`").
    #[error("{cli}: not authenticated - {hint}")]
    NotAuthenticated { cli: String, hint: String },

    /// The CLI exited nonzero for any other reason (or could not be spawned
    /// for a reason other than "missing binary"). Carries the exit code
    /// (`None` if killed by a signal, or if the process never started) and
    /// stderr verbatim.
    #[error("{cli} exited {code:?}: {stderr}")]
    Exec {
        cli: String,
        code: Option<i32>,
        stderr: String,
    },

    /// A 0-exit stdout that failed to parse into the shape this connector
    /// expects, or was not JSON at all.
    #[error("json: {message}")]
    Json { message: String },
}

fn json_err(e: serde_json::Error) -> CloudCliError {
    CloudCliError::Json {
        message: e.to_string(),
    }
}

// ---- public inventory row --------------------------------------------------

/// One VM in the inventory, flattened from whichever provider's shape to one
/// row the Remote panel shows - the multi-cloud sibling of
/// [`crate::HetznerServer`].
#[derive(Debug, Clone, Serialize)]
pub struct CloudServer {
    /// `"aws"` | `"gcp"` | `"azure"` ([`CloudProvider::as_str`]).
    pub provider: String,
    pub id: String,
    pub name: String,
    pub status: String,
    pub public_ip: Option<String>,
    pub private_ip: Option<String>,
    pub server_type: String,
    pub region: String,
}

/// Provider-scoping options for [`list_servers`]. Every field is optional and
/// consumed by exactly one provider (see each field's doc); the default
/// ([`CloudListOptions::default`]) asks each CLI to list whatever its own
/// already-configured default scope covers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CloudListOptions {
    /// AWS only: appended as `--region <r>`. `None` lets `aws` fall back to
    /// its own configured default region.
    pub region: Option<String>,
    /// GCP only: appended as `--project <p>`. `None` lets `gcloud` fall back
    /// to its own configured default project.
    pub project: Option<String>,
    /// Azure only: appended as `--subscription <s>`. `None` lets `az` fall
    /// back to its own configured default subscription.
    pub subscription: Option<String>,
    /// AWS only: appended as `--profile <p>`. `None` lets `aws` fall back to
    /// its default profile (or `$AWS_PROFILE`).
    pub profile: Option<String>,
}

// ---- public API --------------------------------------------------------------

/// List VMs from `provider`'s inventory, scoped by `opts`. Builds the
/// provider's own read-only argv, shells its OFFICIAL CLI with JSON output
/// (capturing stdout and stderr), and parses the result. NEVER creates,
/// modifies, or deletes a cloud resource - only ever runs a describe/list
/// verb.
///
/// Runs the actual spawn inside [`tokio::task::spawn_blocking`] (see the
/// module doc's "Async, off the calling thread"); if the blocking task itself
/// panics, the join failure is reported as [`CloudCliError::Exec`] with
/// `code: None` rather than propagating the panic across the await point.
pub async fn list_servers(
    provider: CloudProvider,
    opts: &CloudListOptions,
) -> Result<Vec<CloudServer>, CloudCliError> {
    let opts = opts.clone();
    tokio::task::spawn_blocking(move || list_servers_blocking(provider, &opts))
        .await
        .unwrap_or_else(|join_err| {
            Err(CloudCliError::Exec {
                cli: provider.cli_name().to_string(),
                code: None,
                stderr: format!("spawn_blocking join failed: {join_err}"),
            })
        })
}

/// The synchronous half of [`list_servers`]: build argv, run the CLI, parse
/// its stdout. Split out from [`list_servers`] only to keep the
/// `spawn_blocking` bridge itself trivial; the parse step is further split
/// into `parse_<provider>` so tests can exercise it directly, offline, never
/// spawning a real CLI.
fn list_servers_blocking(
    provider: CloudProvider,
    opts: &CloudListOptions,
) -> Result<Vec<CloudServer>, CloudCliError> {
    let cli = provider.cli_name();
    let args = build_args(provider, opts);
    let stdout = run_cli(cli, &args)?;
    let text = String::from_utf8_lossy(&stdout);
    match provider {
        CloudProvider::Aws => parse_aws(&text),
        CloudProvider::Gcp => parse_gcp(&text),
        CloudProvider::Azure => parse_azure(&text),
    }
}

// ---- argv construction --------------------------------------------------------

/// Build the argv for `provider`'s describe/list command, appending only the
/// scoping flags present in `opts`. Exact commands:
/// - AWS: `ec2 describe-instances --output json [--region R] [--profile P]`
/// - GCP: `compute instances list --format=json [--project P]`
/// - Azure: `vm list -d --output json [--subscription S]`
fn build_args(provider: CloudProvider, opts: &CloudListOptions) -> Vec<String> {
    match provider {
        CloudProvider::Aws => {
            let mut args: Vec<String> = ["ec2", "describe-instances", "--output", "json"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            if let Some(region) = &opts.region {
                args.push("--region".to_string());
                args.push(region.clone());
            }
            if let Some(profile) = &opts.profile {
                args.push("--profile".to_string());
                args.push(profile.clone());
            }
            args
        }
        CloudProvider::Gcp => {
            let mut args: Vec<String> = ["compute", "instances", "list", "--format=json"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            if let Some(project) = &opts.project {
                args.push("--project".to_string());
                args.push(project.clone());
            }
            args
        }
        CloudProvider::Azure => {
            let mut args: Vec<String> = ["vm", "list", "-d", "--output", "json"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            if let Some(sub) = &opts.subscription {
                args.push("--subscription".to_string());
                args.push(sub.clone());
            }
            args
        }
    }
}

// ---- CLI plumbing (mirrors QryxClient/MockryxClient's run_raw pattern) --------

/// Run `cli` with `args`, capturing stdout+stderr, and classify the result:
/// a spawn failure whose `io::ErrorKind` is `NotFound` becomes
/// [`CloudCliError::CliNotFound`]; any other spawn failure becomes
/// [`CloudCliError::Exec`] (there is no exit code to report, so `code: None`);
/// a nonzero exit whose stderr looks like an auth problem becomes
/// [`CloudCliError::NotAuthenticated`]; any other nonzero exit becomes
/// [`CloudCliError::Exec`]. On success, returns stdout bytes verbatim -
/// parsing is each provider's own `parse_<provider>` job.
fn run_cli(cli: &'static str, args: &[String]) -> Result<Vec<u8>, CloudCliError> {
    let spawned = std::process::Command::new(cli).args(args).output();
    let output = match spawned {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CloudCliError::CliNotFound {
                cli: cli.to_string(),
            });
        }
        Err(e) => {
            return Err(CloudCliError::Exec {
                cli: cli.to_string(),
                code: None,
                stderr: e.to_string(),
            });
        }
    };

    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if looks_unauthenticated(&stderr) {
        return Err(CloudCliError::NotAuthenticated {
            cli: cli.to_string(),
            hint: auth_hint(cli),
        });
    }
    Err(CloudCliError::Exec {
        cli: cli.to_string(),
        code: output.status.code(),
        stderr,
    })
}

/// Whether a nonzero-exit CLI's stderr indicates an auth problem rather than a
/// generic error - a lowercased substring match against the phrases each real
/// CLI is known to emit for "you are not logged in" (`aws`'s "Unable to
/// locate credentials", `gcloud`'s "reauthenticate"/"auth login" prompts,
/// `az`'s "Please run 'az login'"), plus the general words that cover any
/// close variant.
fn looks_unauthenticated(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    const NEEDLES: [&str; 5] = [
        "auth",
        "login",
        "credentials",
        "not logged in",
        "unable to locate credentials",
    ];
    NEEDLES.iter().any(|needle| lower.contains(needle))
}

/// A short, provider-specific remediation hint for
/// [`CloudCliError::NotAuthenticated`].
fn auth_hint(cli: &str) -> String {
    match cli {
        "aws" => "run `aws configure` (or set AWS_PROFILE)".to_string(),
        "gcloud" => "run `gcloud auth login`".to_string(),
        "az" => "run `az login`".to_string(),
        other => format!("authenticate the `{other}` CLI"),
    }
}

// ---- defensive JSON field access ----------------------------------------------

/// Read `key` off a JSON object as a string, accepting either a JSON string or
/// a JSON number (GCE's `id` is a numeric-looking string on the wire, but this
/// stays defensive against a tool that emits it as a bare number). Anything
/// else - absent key, `null`, object, array, bool - becomes `""`, never an
/// error: one missing or oddly-typed field must not fail the whole listing.
fn str_field(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// [`str_field`], but empty becomes `None`. Used for every optional IP field:
/// AWS omits the key entirely when a VM has no public IP; GCP/Azure can emit
/// the key with an empty value. Either shape ends up `None` here, never an
/// error.
fn opt_str_field(v: &serde_json::Value, key: &str) -> Option<String> {
    let s = str_field(v, key);
    if s.is_empty() { None } else { Some(s) }
}

/// The last `/`-separated segment of `full` (or `full` itself if there is no
/// `/`). GCP's `machineType`/`zone` are full resource URLs; the panel only
/// wants the trailing name (`e2-standard-4`, `us-central1-a`).
fn basename(full: &str) -> String {
    full.rsplit('/').next().unwrap_or(full).to_string()
}

// ---- AWS: `aws ec2 describe-instances --output json` --------------------------

/// Parse `aws ec2 describe-instances --output json`'s `Reservations[].
/// Instances[]`. Defensive throughout: a missing `Reservations`/`Instances`
/// array is treated as empty, not an error; a missing `Name` tag is `""`; a
/// missing `PublicIpAddress`/`PrivateIpAddress` is `None`.
fn parse_aws(json: &str) -> Result<Vec<CloudServer>, CloudCliError> {
    let root: serde_json::Value = serde_json::from_str(json).map_err(json_err)?;
    let empty = Vec::new();
    let reservations = root
        .get("Reservations")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);

    let mut out = Vec::new();
    for reservation in reservations {
        let instances = reservation
            .get("Instances")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty);
        for instance in instances {
            let tags = instance
                .get("Tags")
                .and_then(|v| v.as_array())
                .unwrap_or(&empty);
            let name = tags
                .iter()
                .find(|t| t.get("Key").and_then(|k| k.as_str()) == Some("Name"))
                .map(|t| str_field(t, "Value"))
                .unwrap_or_default();
            let status = instance
                .get("State")
                .map(|s| str_field(s, "Name"))
                .unwrap_or_default();
            let region = instance
                .get("Placement")
                .map(|p| str_field(p, "AvailabilityZone"))
                .unwrap_or_default();

            out.push(CloudServer {
                provider: CloudProvider::Aws.as_str().to_string(),
                id: str_field(instance, "InstanceId"),
                name,
                status,
                public_ip: opt_str_field(instance, "PublicIpAddress"),
                private_ip: opt_str_field(instance, "PrivateIpAddress"),
                server_type: str_field(instance, "InstanceType"),
                region,
            });
        }
    }
    Ok(out)
}

// ---- GCP: `gcloud compute instances list --format=json` -----------------------

/// The first `natIP` under `networkInterfaces[0].accessConfigs[]`, if any - a
/// VM with no external access config (private-only) has none, `None`, not an
/// error.
fn gcp_public_ip(instance: &serde_json::Value) -> Option<String> {
    instance
        .get("networkInterfaces")?
        .as_array()?
        .first()?
        .get("accessConfigs")?
        .as_array()?
        .first()?
        .get("natIP")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// `networkInterfaces[0].networkIP`, if any.
fn gcp_private_ip(instance: &serde_json::Value) -> Option<String> {
    instance
        .get("networkInterfaces")?
        .as_array()?
        .first()?
        .get("networkIP")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Parse `gcloud compute instances list --format=json`'s top-level array.
/// `machineType`/`zone` are full resource URLs on the wire; only their
/// [`basename`] is kept.
fn parse_gcp(json: &str) -> Result<Vec<CloudServer>, CloudCliError> {
    let root: serde_json::Value = serde_json::from_str(json).map_err(json_err)?;
    let empty = Vec::new();
    let instances = root.as_array().unwrap_or(&empty);

    let mut out = Vec::new();
    for instance in instances {
        out.push(CloudServer {
            provider: CloudProvider::Gcp.as_str().to_string(),
            id: str_field(instance, "id"),
            name: str_field(instance, "name"),
            status: str_field(instance, "status"),
            public_ip: gcp_public_ip(instance),
            private_ip: gcp_private_ip(instance),
            server_type: basename(&str_field(instance, "machineType")),
            region: basename(&str_field(instance, "zone")),
        });
    }
    Ok(out)
}

// ---- Azure: `az vm list -d --output json` --------------------------------------

/// Parse `az vm list -d --output json`'s top-level array. `id` prefers
/// `vmId`, falling back to `name` when `vmId` is absent/empty (both are
/// unique-enough identifiers on the wire; `vmId` is the GUID, `name` is
/// always present).
fn parse_azure(json: &str) -> Result<Vec<CloudServer>, CloudCliError> {
    let root: serde_json::Value = serde_json::from_str(json).map_err(json_err)?;
    let empty = Vec::new();
    let instances = root.as_array().unwrap_or(&empty);

    let mut out = Vec::new();
    for instance in instances {
        let name = str_field(instance, "name");
        let vm_id = str_field(instance, "vmId");
        let id = if vm_id.is_empty() {
            name.clone()
        } else {
            vm_id
        };
        let server_type = instance
            .get("hardwareProfile")
            .map(|h| str_field(h, "vmSize"))
            .unwrap_or_default();

        out.push(CloudServer {
            provider: CloudProvider::Azure.as_str().to_string(),
            id,
            name,
            status: str_field(instance, "powerState"),
            public_ip: opt_str_field(instance, "publicIps"),
            private_ip: opt_str_field(instance, "privateIps"),
            server_type,
            region: str_field(instance, "location"),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exact shapes each provider's own CLI emits (trimmed to the fields this
    // connector reads), parsed offline. A live shell of a real aws/gcloud/az
    // binary is deliberately out of scope here (external, paid, auth-gated
    // CLIs) - only the parse/argv-build/error-classification layers are
    // tested, mirroring hetzner.rs's/qryx.rs's own offline-fixture style.

    const AWS_FIXTURE: &str = r#"{
      "Reservations": [
        {
          "Instances": [
            {
              "InstanceId": "i-0abcdef1234567890",
              "InstanceType": "t3.medium",
              "State": { "Code": 16, "Name": "running" },
              "Placement": { "AvailabilityZone": "us-east-1a" },
              "PublicIpAddress": "203.0.113.10",
              "PrivateIpAddress": "10.0.1.5",
              "Tags": [
                { "Key": "Name", "Value": "taipan-web-1" },
                { "Key": "managed-by", "Value": "taipan" }
              ]
            },
            {
              "InstanceId": "i-0fedcba9876543210",
              "InstanceType": "t3.small",
              "State": { "Code": 16, "Name": "running" },
              "Placement": { "AvailabilityZone": "us-east-1b" },
              "PrivateIpAddress": "10.0.1.9"
            }
          ]
        }
      ]
    }"#;

    const GCP_FIXTURE: &str = r#"[
      {
        "id": "1234567890123456789",
        "name": "taipan-live-1",
        "status": "RUNNING",
        "machineType": "https://www.googleapis.com/compute/v1/projects/my-proj/zones/us-central1-a/machineTypes/e2-standard-4",
        "zone": "https://www.googleapis.com/compute/v1/projects/my-proj/zones/us-central1-a",
        "networkInterfaces": [
          {
            "networkIP": "10.128.0.5",
            "accessConfigs": [
              { "type": "ONE_TO_ONE_NAT", "natIP": "34.120.10.20" }
            ]
          }
        ]
      },
      {
        "id": "9876543210987654321",
        "name": "taipan-private-1",
        "status": "TERMINATED",
        "machineType": "https://www.googleapis.com/compute/v1/projects/my-proj/zones/us-central1-a/machineTypes/e2-medium",
        "zone": "https://www.googleapis.com/compute/v1/projects/my-proj/zones/us-central1-a",
        "networkInterfaces": [
          { "networkIP": "10.128.0.9" }
        ]
      }
    ]"#;

    const AZURE_FIXTURE: &str = r#"[
      {
        "vmId": "11111111-2222-3333-4444-555555555555",
        "name": "taipan-live-1",
        "powerState": "VM running",
        "publicIps": "20.10.20.30",
        "privateIps": "10.1.0.4",
        "hardwareProfile": { "vmSize": "Standard_D2s_v5" },
        "location": "eastus"
      },
      {
        "vmId": "",
        "name": "taipan-private-1",
        "powerState": "VM deallocated",
        "publicIps": "",
        "privateIps": "10.1.0.9",
        "hardwareProfile": { "vmSize": "Standard_B2s" },
        "location": "westeurope"
      }
    ]"#;

    #[test]
    fn provider_as_str_and_parse_round_trip_case_insensitively() {
        assert_eq!(CloudProvider::Aws.as_str(), "aws");
        assert_eq!(CloudProvider::Gcp.as_str(), "gcp");
        assert_eq!(CloudProvider::Azure.as_str(), "azure");

        assert_eq!(CloudProvider::parse("aws"), Some(CloudProvider::Aws));
        assert_eq!(CloudProvider::parse("AWS"), Some(CloudProvider::Aws));
        assert_eq!(CloudProvider::parse("Gcp"), Some(CloudProvider::Gcp));
        assert_eq!(CloudProvider::parse("AZURE"), Some(CloudProvider::Azure));
        assert_eq!(CloudProvider::parse("digitalocean"), None);
    }

    #[test]
    fn aws_parses_name_tag_placement_and_optional_public_ip() {
        let out = parse_aws(AWS_FIXTURE).expect("parse");
        assert_eq!(out.len(), 2);

        let a = &out[0];
        assert_eq!(a.provider, "aws");
        assert_eq!(a.id, "i-0abcdef1234567890");
        assert_eq!(a.name, "taipan-web-1");
        assert_eq!(a.status, "running");
        assert_eq!(a.public_ip.as_deref(), Some("203.0.113.10"));
        assert_eq!(a.private_ip.as_deref(), Some("10.0.1.5"));
        assert_eq!(a.server_type, "t3.medium");
        assert_eq!(a.region, "us-east-1a");

        // No Name tag, no PublicIpAddress key at all -> "" name, None IP,
        // never a parse failure.
        let b = &out[1];
        assert_eq!(b.name, "");
        assert!(b.public_ip.is_none());
        assert_eq!(b.private_ip.as_deref(), Some("10.0.1.9"));
    }

    #[test]
    fn gcp_parses_basenamed_type_and_zone_and_optional_nat_ip() {
        let out = parse_gcp(GCP_FIXTURE).expect("parse");
        assert_eq!(out.len(), 2);

        let a = &out[0];
        assert_eq!(a.provider, "gcp");
        assert_eq!(a.id, "1234567890123456789");
        assert_eq!(a.name, "taipan-live-1");
        assert_eq!(a.status, "RUNNING");
        assert_eq!(a.public_ip.as_deref(), Some("34.120.10.20"));
        assert_eq!(a.private_ip.as_deref(), Some("10.128.0.5"));
        assert_eq!(a.server_type, "e2-standard-4");
        assert_eq!(a.region, "us-central1-a");

        // No accessConfigs (private-only instance) -> None, not an error.
        let b = &out[1];
        assert!(b.public_ip.is_none());
        assert_eq!(b.private_ip.as_deref(), Some("10.128.0.9"));
        assert_eq!(b.server_type, "e2-medium");
    }

    #[test]
    fn azure_parses_power_state_and_falls_back_id_to_name() {
        let out = parse_azure(AZURE_FIXTURE).expect("parse");
        assert_eq!(out.len(), 2);

        let a = &out[0];
        assert_eq!(a.provider, "azure");
        assert_eq!(a.id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(a.status, "VM running");
        assert_eq!(a.public_ip.as_deref(), Some("20.10.20.30"));
        assert_eq!(a.server_type, "Standard_D2s_v5");
        assert_eq!(a.region, "eastus");

        // Empty vmId -> id falls back to name; empty publicIps -> None.
        let b = &out[1];
        assert_eq!(b.id, "taipan-private-1");
        assert!(b.public_ip.is_none());
        assert_eq!(b.private_ip.as_deref(), Some("10.1.0.9"));
    }

    #[test]
    fn empty_top_level_shapes_parse_to_empty_vecs() {
        assert!(
            parse_aws(r#"{"Reservations":[]}"#)
                .expect("parse")
                .is_empty()
        );
        assert!(parse_gcp("[]").expect("parse").is_empty());
        assert!(parse_azure("[]").expect("parse").is_empty());
    }

    #[test]
    fn malformed_json_is_a_json_error_not_a_panic() {
        assert!(matches!(
            parse_aws("not json"),
            Err(CloudCliError::Json { .. })
        ));
        assert!(matches!(
            parse_gcp("not json"),
            Err(CloudCliError::Json { .. })
        ));
        assert!(matches!(
            parse_azure("not json"),
            Err(CloudCliError::Json { .. })
        ));
    }

    #[test]
    fn build_args_appends_only_the_scoping_flags_present() {
        let none = CloudListOptions::default();
        assert_eq!(
            build_args(CloudProvider::Aws, &none),
            vec!["ec2", "describe-instances", "--output", "json"]
        );
        assert_eq!(
            build_args(CloudProvider::Gcp, &none),
            vec!["compute", "instances", "list", "--format=json"]
        );
        assert_eq!(
            build_args(CloudProvider::Azure, &none),
            vec!["vm", "list", "-d", "--output", "json"]
        );

        let scoped = CloudListOptions {
            region: Some("eu-west-1".to_string()),
            project: Some("my-proj".to_string()),
            subscription: Some("sub-id".to_string()),
            profile: Some("prod".to_string()),
        };
        assert_eq!(
            build_args(CloudProvider::Aws, &scoped),
            vec![
                "ec2",
                "describe-instances",
                "--output",
                "json",
                "--region",
                "eu-west-1",
                "--profile",
                "prod"
            ]
        );
        assert_eq!(
            build_args(CloudProvider::Gcp, &scoped),
            vec![
                "compute",
                "instances",
                "list",
                "--format=json",
                "--project",
                "my-proj"
            ]
        );
        assert_eq!(
            build_args(CloudProvider::Azure, &scoped),
            vec![
                "vm",
                "list",
                "-d",
                "--output",
                "json",
                "--subscription",
                "sub-id"
            ]
        );
    }

    #[test]
    fn looks_unauthenticated_matches_known_auth_failure_phrases() {
        assert!(looks_unauthenticated("Unable to locate credentials"));
        assert!(looks_unauthenticated(
            "Please run 'az login' to setup account."
        ));
        assert!(looks_unauthenticated(
            "You do not currently have an active account selected. Please run: gcloud auth login"
        ));
        assert!(!looks_unauthenticated("InvalidParameterValue: bad region"));
    }

    #[test]
    fn run_cli_against_a_missing_binary_is_cli_not_found() {
        // A binary name that cannot possibly exist -> CliNotFound (this is
        // exactly what a caller reads as "install the CLI"). Mirrors
        // QryxClient/MockryxClient's own "/nonexistent/…-binary-xyz" tests.
        match run_cli("definitely-not-a-real-cloud-cli-xyz", &[]) {
            Err(CloudCliError::CliNotFound { cli }) => {
                assert_eq!(cli, "definitely-not-a-real-cloud-cli-xyz");
            }
            other => panic!("expected CliNotFound, got {other:?}"),
        }
    }
}
