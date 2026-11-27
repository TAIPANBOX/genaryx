//! Stand-in command-line tools for the clients that shell one: a small
//! `/bin/sh` script written into a scratch directory, never a real plane
//! binary, so a test fixes exactly what the "tool" prints and how it exits,
//! and can read back the arguments and environment it was given.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A fresh, empty directory unique to this process and call.
pub fn scratch_dir(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "gx-conn-standin-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write an executable `/bin/sh` script named `name` into `dir`.
pub fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write stand-in");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

/// The lines a stand-in recorded with `printf '%s\n' "$@" > <file>`.
pub fn recorded_args(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .expect("the stand-in recorded its arguments")
        .lines()
        .map(str::to_string)
        .collect()
}

/// Whether an error is the kernel refusing to run a file another thread still
/// has open for writing ("text file busy"), which a script written a moment
/// before it is run can meet once while other tests fork in parallel.
pub fn is_busy(text: &str) -> bool {
    text.to_ascii_lowercase().contains("text file busy")
}

/// Run `f` again while it fails only because the stand-in was busy.
pub fn retry_busy<T, E: std::fmt::Display>(mut f: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    for _ in 0..50 {
        match f() {
            Err(e) if is_busy(&e.to_string()) => std::thread::sleep(Duration::from_millis(20)),
            other => return other,
        }
    }
    f()
}
