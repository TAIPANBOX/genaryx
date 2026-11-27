//! Test-only: write a stand-in executable without this process ever holding a
//! write descriptor on it.
//!
//! Why it exists: `std::fs::write` then exec from a multithreaded test binary
//! fails now and then with ETXTBSY ("Text file busy"). While one test still
//! has the script open for writing, another test thread forks to spawn its
//! own child; that child inherits the write descriptor until it execs, and
//! the kernel refuses to exec a file that any process holds open for writing.
//! Writing through a short-lived `/bin/sh` keeps the write descriptor in that
//! child only, so no sibling fork can inherit it.
//!
//! One source, included by `#[path]` from every test target that needs it.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write `body` to `path` and make it executable (mode 0755).
pub fn write_executable(path: &Path, body: &str) {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("cat > \"$1\" && chmod 755 \"$1\"")
        .arg("sh")
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn /bin/sh to write the stand-in");
    child
        .stdin
        .take()
        .expect("the writer's stdin")
        .write_all(body.as_bytes())
        .expect("hand the stand-in's body to the writer");
    let status = child.wait().expect("wait for the writer");
    assert!(
        status.success(),
        "writing the stand-in {} failed: {status}",
        path.display()
    );
}
