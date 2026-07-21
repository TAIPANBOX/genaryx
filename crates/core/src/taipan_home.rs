//! Where a `taipan up` install keeps its environment descriptors.
//!
//! One definition, deliberately: this used to be copied into every plane's
//! `env.rs` AND into `bus.rs`, and the copies drifted. Only the `bus.rs` one
//! honoured `TAIPAN_HOME`, so pointing that variable at a scratch install put
//! the Bus Explorer on the scratch environment while every plane kept reading
//! the real `~/.taipan`. The console then showed two different environments
//! at once and called both healthy, which is worse than either being broken.
//!
//! `bus.rs` already spelled out why the variable has to be honoured: stack-up
//! honours the same variable, so a clean-machine test where the tools write to
//! a scratch home and the console reads the real one proves nothing.

use std::path::PathBuf;

/// `~/.taipan/environments`, honouring `TAIPAN_HOME` so an entire install can
/// be pointed at a scratch directory (stack-up honours the same variable; a
/// clean-machine test where the tools write to a scratch home and the console
/// reads the real one proves nothing).
///
/// `None` when neither `TAIPAN_HOME` nor `HOME` is set, rather than a panic
/// over a missing environment variable.
pub fn environments_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("TAIPAN_HOME") {
        return Some(PathBuf::from(home).join("environments"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("environments"))
}
