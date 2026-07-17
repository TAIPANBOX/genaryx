//! `SshClient`: the ops/remote-tail SSH connector (docs/PHASE4.md W4, decision
//! D11 §"SSH stays for ops"). WireGuard is the PRIMARY console<->Cloud channel
//! (see `crate` docs / D11); SSH is the SECONDARY, ops-focused transport: read
//! the remote taipan descriptor, tail remote logs, and (optionally) forward a
//! loopback port - all with **host-key pinning**, never a trust-on-first-use or
//! disabled check.
//!
//! ## Why shell OpenSSH (and why that is the secure choice here)
//!
//! Rather than hand-roll an SSH client, this shells the system `ssh` with a
//! locked-down, PINNED option set. Delegating the actual key exchange +
//! host-key enforcement to OpenSSH (the audited gold standard) is safer than a
//! bespoke handshake: the connector's only job is to construct the invocation
//! so the pin is ALWAYS enforced and there is NEVER a fallback to an unpinned or
//! interactive mode. Every command runs with, unconditionally
//! ([`base_ssh_args`], unit-tested to prove it):
//!
//! - `StrictHostKeyChecking=yes` - reject an unknown OR changed host key
//!   (fail-closed; NEVER `accept-new`/`no`).
//! - `UserKnownHostsFile=<pinned>` + `GlobalKnownHostsFile=/dev/null` - the
//!   ONLY trusted host key is the one the caller pinned; the user's real
//!   `~/.ssh/known_hosts` is not consulted, so a stale TOFU entry cannot
//!   silently authorize a different key.
//! - `BatchMode=yes` - never prompt; a missing or mismatched key fails instead
//!   of asking a human to click through.
//! - `PasswordAuthentication=no` + `KbdInteractiveAuthentication=no` +
//!   `IdentitiesOnly=yes` - key-only auth with ONLY the caller's `-i` key, no
//!   agent keys, no password path.
//!
//! ## Key hygiene (the standing hard rule)
//!
//! This connector NEVER generates, rotates, or deletes any key. It receives an
//! already-existing identity (private-key) file path to authenticate with and
//! an already-known pinned HOST key to trust, and writes only a private,
//! `0600`, temp known_hosts holding that pinned host key (removed on drop).
//! Handing Yurii a fresh PUBLIC key for a campaign, and all teardown, happen
//! entirely outside this code ([[never-delete-keys-on-own-initiative]],
//! [[hetzner-vps-provisioning]]).
//!
//! Fail-closed (06 §0.5): a spawn failure is [`SshError::Spawn`]; a nonzero
//! remote exit is [`SshError::Remote`] (carrying ssh's stderr, which includes
//! `Host key verification failed` on a pin mismatch); no panics.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

// ---- error -----------------------------------------------------------------

/// Every failure mode an [`SshClient`] call can surface. Fail-closed.
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    /// Writing the pinned temp known_hosts file failed.
    #[error("write pinned known_hosts {path}: {source}")]
    Pin {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// The `ssh` process could not be spawned (ssh not installed, bad identity
    /// path).
    #[error("ssh spawn: {0}")]
    Spawn(#[source] std::io::Error),

    /// `ssh` (or the remote command) exited nonzero. On a host-key PIN MISMATCH
    /// this carries OpenSSH's `Host key verification failed` - the fail-closed
    /// signal that the box is not the pinned one. Also covers auth failures and
    /// a nonzero remote command.
    #[error("ssh remote exited {code}: {stderr}")]
    Remote { code: i32, stderr: String },
}

// ---- target ----------------------------------------------------------------

/// Where + how to reach a remote box, with the pin. All fields are
/// caller-supplied; this connector resolves none of them and generates no keys.
#[derive(Debug, Clone)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Path to the private-key identity to authenticate with (never generated
    /// or deleted here; `-i`, with `IdentitiesOnly=yes`).
    pub identity_file: PathBuf,
    /// The PINNED host public key, as a `known_hosts` key field, i.e.
    /// `"<keytype> <base64>"` (e.g. `"ssh-ed25519 AAAAC3Nza..."`). This is the
    /// ONLY host key that will be trusted.
    pub pinned_host_key: String,
}

// ---- pure arg builder (the security core; unit-tested without a server) ----

/// The base `ssh` argument vector for `target`, using `known_hosts_path` as the
/// pinned host-key store. This is the security-critical core: the pin +
/// fail-closed options below are ALWAYS present and there is NO code path that
/// omits or weakens them.
fn base_ssh_args(target: &SshTarget, known_hosts_path: &Path) -> Vec<String> {
    vec![
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", known_hosts_path.display()),
        "-o".into(),
        "GlobalKnownHostsFile=/dev/null".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "PasswordAuthentication=no".into(),
        "-o".into(),
        "KbdInteractiveAuthentication=no".into(),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-i".into(),
        target.identity_file.display().to_string(),
        "-p".into(),
        target.port.to_string(),
        format!("{}@{}", target.user, target.host),
    ]
}

/// The `known_hosts` line pinning `target`'s host key. A non-22 port uses the
/// `[host]:port` form OpenSSH expects.
fn known_hosts_line(target: &SshTarget) -> String {
    let host_field = if target.port == 22 {
        target.host.clone()
    } else {
        format!("[{}]:{}", target.host, target.port)
    };
    format!("{host_field} {}\n", target.pinned_host_key.trim())
}

// ---- client ----------------------------------------------------------------

/// A host-key-pinned SSH ops client. Holds the target + a private temp
/// known_hosts (written once at construction, removed on drop). Every method
/// shells `ssh` with [`base_ssh_args`], so the pin is always enforced.
#[derive(Debug)]
pub struct SshClient {
    target: SshTarget,
    known_hosts: PathBuf,
}

impl SshClient {
    /// Pin the target's host key into a private temp known_hosts and return a
    /// client. Does NOT connect yet (no network at construction); the first
    /// method call is the first connection. Fails only if the pin file cannot
    /// be written.
    pub fn connect(target: SshTarget) -> Result<Self, SshError> {
        static N: AtomicU32 = AtomicU32::new(0);
        let known_hosts = std::env::temp_dir().join(format!(
            "genaryx-ssh-known-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        write_pinned_known_hosts(&known_hosts, &known_hosts_line(&target))?;
        Ok(Self {
            target,
            known_hosts,
        })
    }

    /// Run `remote_cmd` on the box and return its stdout bytes. The host-key pin
    /// is enforced (a mismatch fails with `Host key verification failed` in the
    /// [`SshError::Remote`] stderr). Used for one-shot ops like reading the
    /// remote taipan descriptor.
    pub fn run(&self, remote_cmd: &str) -> Result<Vec<u8>, SshError> {
        let mut args = base_ssh_args(&self.target, &self.known_hosts);
        args.push(remote_cmd.to_string());
        let out = Command::new("ssh")
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .map_err(SshError::Spawn)?;
        if !out.status.success() {
            return Err(SshError::Remote {
                code: out.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(out.stdout)
    }

    /// A reachability + pin + auth probe: run `true` on the box. `Ok(())` means
    /// the pinned box authenticated us; an `Err` distinguishes a pin/auth
    /// failure (its stderr) from an unreachable host.
    pub fn check_reachable(&self) -> Result<(), SshError> {
        self.run("true").map(|_| ())
    }

    /// Read a remote file's bytes (e.g. the taipan descriptor
    /// `~/.taipan/environments/<name>.json`) via `cat --`.
    pub fn read_remote_file(&self, remote_path: &str) -> Result<Vec<u8>, SshError> {
        // `cat --` so a path starting with `-` is not read as a flag.
        self.run(&format!("cat -- {}", shell_single_quote(remote_path)))
    }

    /// Spawn a streaming `tail -F` of a remote file starting at byte offset
    /// `from_offset`, returning the live `ssh` child. The caller reads the
    /// child's stdout (line-delimited log bytes) and kills it when done. Mirrors
    /// the local `FileTail` (`genaryx_core::ingest`) but over the pinned SSH
    /// channel. `-c +N` is 1-based (OpenSSH/coreutils `tail`), so this passes
    /// `from_offset + 1`.
    pub fn spawn_tail(&self, remote_path: &str, from_offset: u64) -> Result<Child, SshError> {
        let mut args = base_ssh_args(&self.target, &self.known_hosts);
        args.push(format!(
            "tail -c +{} -F {}",
            from_offset.saturating_add(1),
            shell_single_quote(remote_path)
        ));
        Command::new("ssh")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(SshError::Spawn)
    }

    /// Spawn a background local-port-forward: `-L
    /// 127.0.0.1:<local_port>:127.0.0.1:<remote_port>` plus `-N` (no remote
    /// command), returning the live `ssh` child. Binds the LOCAL end to
    /// loopback only (never `0.0.0.0`), so the forwarded port is not itself
    /// exposed. The caller kills the child to tear the forward down. (Secondary
    /// to WireGuard, which is the primary persistent channel per D11.)
    pub fn spawn_forward(&self, local_port: u16, remote_port: u16) -> Result<Child, SshError> {
        let mut args = base_ssh_args(&self.target, &self.known_hosts);
        args.push("-N".into());
        args.push("-L".into());
        args.push(format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}"));
        Command::new("ssh")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(SshError::Spawn)
    }
}

impl Drop for SshClient {
    fn drop(&mut self) {
        // Remove the temp known_hosts (best effort). It only ever held the
        // caller's already-known pinned HOST key, never a private/user key.
        let _ = std::fs::remove_file(&self.known_hosts);
    }
}

// ---- helpers ---------------------------------------------------------------

fn write_pinned_known_hosts(path: &Path, line: &str) -> Result<(), SshError> {
    let mkerr = |source| SshError::Pin {
        path: path.display().to_string(),
        source,
    };
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(mkerr)?;
    f.write_all(line.as_bytes()).map_err(mkerr)?;
    Ok(())
}

/// Single-quote a string for a POSIX remote shell (`'` -> `'\''`), so a remote
/// path cannot inject shell syntax.
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> SshTarget {
        SshTarget {
            host: "203.0.113.7".into(),
            port: 2222,
            user: "root".into(),
            identity_file: PathBuf::from("/home/op/.ssh/hetzner-2026"),
            pinned_host_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc".into(),
        }
    }

    #[test]
    fn base_args_always_enforce_the_pin_and_fail_closed_options() {
        let kh = PathBuf::from("/tmp/pinned-known-hosts");
        let args = base_ssh_args(&target(), &kh);
        let joined = args.join(" ");
        // The pin + fail-closed set is ALWAYS present.
        assert!(joined.contains("StrictHostKeyChecking=yes"));
        assert!(joined.contains("UserKnownHostsFile=/tmp/pinned-known-hosts"));
        assert!(joined.contains("GlobalKnownHostsFile=/dev/null"));
        assert!(joined.contains("BatchMode=yes"));
        assert!(joined.contains("PasswordAuthentication=no"));
        assert!(joined.contains("IdentitiesOnly=yes"));
        // And NEVER a weakening option.
        assert!(!joined.contains("StrictHostKeyChecking=no"));
        assert!(!joined.contains("StrictHostKeyChecking=accept-new"));
        // Identity + port + user@host are wired.
        assert!(joined.contains("-i /home/op/.ssh/hetzner-2026"));
        assert!(joined.contains("-p 2222"));
        assert!(joined.contains("root@203.0.113.7"));
    }

    #[test]
    fn known_hosts_line_uses_bracket_port_form_for_non_22() {
        let line = known_hosts_line(&target());
        assert_eq!(
            line,
            "[203.0.113.7]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc\n"
        );
        // Port 22 uses the bare host form.
        let mut t = target();
        t.port = 22;
        assert!(known_hosts_line(&t).starts_with("203.0.113.7 ssh-ed25519 "));
    }

    #[test]
    fn connect_writes_a_private_pinned_known_hosts_then_drop_removes_it() {
        let c = SshClient::connect(target()).expect("pin write");
        let path = c.known_hosts.clone();
        assert!(path.is_file(), "pinned known_hosts written");
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "pin file is private");
        }
        drop(c);
        assert!(!path.exists(), "drop removes the temp pin file");
    }

    #[test]
    fn shell_single_quote_neutralizes_injection() {
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_single_quote("/x/y.json"), "'/x/y.json'");
    }

    #[test]
    fn run_against_an_unresolvable_host_is_fail_closed() {
        // No network dependency: BatchMode + an unresolvable host means ssh
        // exits nonzero fast, surfacing as Remote/Spawn, never a hang or a
        // panic. (If `ssh` is absent entirely, that is Spawn - also fine.)
        let mut t = target();
        t.host = "genaryx.invalid.nonexistent.example".into();
        let c = SshClient::connect(t).expect("pin");
        match c.check_reachable() {
            Err(SshError::Remote { .. }) | Err(SshError::Spawn(_)) => {}
            other => panic!("expected a fail-closed error, got {other:?}"),
        }
    }
}
