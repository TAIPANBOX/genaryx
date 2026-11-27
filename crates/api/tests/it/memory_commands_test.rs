//! The Memory view's commands before an engram-mcp process exists: every
//! state the panel can be in is named by its status and by each command, and
//! a scope that is not one printable word is refused before any process
//! could be started for it, whatever state the panel is in.
//!
//! No engram-mcp is spawned here.

use genaryx_api::memory::commands::{
    MemoryError, MemoryStatusDto, memory_forget, memory_recall, memory_stats, memory_status,
    memory_why,
};
use genaryx_api::memory::env::EnvSource;
use genaryx_api::memory::state::{MemoryInner, MemoryState};
use std::path::PathBuf;

async fn state_of(inner: MemoryInner) -> MemoryState {
    let state = MemoryState::pending();
    *state.inner.lock().await = inner;
    state
}

fn unreachable() -> MemoryInner {
    MemoryInner::Unreachable {
        source: EnvSource::WellKnown,
        engram_mcp_bin: PathBuf::from("/opt/engram/engram-mcp"),
        db_path: PathBuf::from("/var/lib/engram/.engram"),
        reason: "handshake failed".to_string(),
    }
}

/// The error of a command, or a panic naming the command that answered.
fn err_of<T>(what: &str, r: Result<T, MemoryError>) -> MemoryError {
    match r {
        Err(e) => e,
        Ok(_) => panic!("{what}: answered with no engram-mcp behind it"),
    }
}

#[tokio::test]
async fn the_status_names_each_state_before_ready() {
    assert!(matches!(
        memory_status(&MemoryState::pending()).await,
        Ok(MemoryStatusDto::Bootstrapping)
    ));
    assert!(matches!(
        memory_status(&state_of(MemoryInner::NoEnvironment).await).await,
        Ok(MemoryStatusDto::NoEnvironment)
    ));
    match memory_status(&state_of(unreachable()).await).await {
        Ok(MemoryStatusDto::Unreachable {
            engram_mcp_bin,
            db_path,
            reason,
            ..
        }) => {
            assert_eq!(engram_mcp_bin, "/opt/engram/engram-mcp");
            assert_eq!(db_path, "/var/lib/engram/.engram");
            assert_eq!(reason, "handshake failed");
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }
}

#[tokio::test]
async fn every_command_answers_the_panels_state_while_it_is_not_ready() {
    for (inner, label) in [
        (MemoryInner::Bootstrapping, "bootstrapping"),
        (MemoryInner::NoEnvironment, "no_environment"),
        (unreachable(), "unreachable"),
    ] {
        let state = state_of(inner).await;
        let errors = [
            err_of("stats", memory_stats(None, &state).await),
            err_of(
                "recall",
                memory_recall("refund policy".to_string(), 5, "hybrid".to_string(), None, &state)
                    .await,
            ),
            err_of("why", memory_why("mem-1".to_string(), &state).await),
            err_of("forget", memory_forget("mem-1".to_string(), &state).await),
        ];
        for e in errors {
            match (label, e) {
                ("bootstrapping", MemoryError::Bootstrapping)
                | ("no_environment", MemoryError::NoEnvironment) => {}
                ("unreachable", MemoryError::Unreachable { reason }) => {
                    assert_eq!(reason, "handshake failed");
                }
                (label, other) => panic!("{label}: got {other:?}"),
            }
        }
    }
}

/// An empty agent name is the whole-file view, so it reaches the panel's
/// state like no name at all.
#[tokio::test]
async fn an_empty_agent_name_is_the_whole_file_view() {
    let pending = MemoryState::pending();
    assert!(matches!(
        memory_stats(Some(String::new()), &pending).await,
        Err(MemoryError::Bootstrapping)
    ));
}

/// A scope that could be read as a flag or as two arguments is refused before
/// the panel is consulted, so it can never reach a process's argument list.
#[tokio::test]
async fn a_scope_that_is_not_one_printable_word_is_refused_first() {
    let pending = MemoryState::pending();
    for bad in ["two words", "--db", "tab\there", "line\nbreak"] {
        match err_of(
            "stats",
            memory_stats(Some(bad.to_string()), &pending).await,
        ) {
            MemoryError::Mcp { message } => {
                assert!(message.contains("one printable word"), "{bad:?}: {message}");
            }
            other => panic!("{bad:?}: expected the scope refusal, got {other:?}"),
        }
        match err_of(
            "recall",
            memory_recall(
                "q".to_string(),
                1,
                "cosine".to_string(),
                Some(bad.to_string()),
                &pending,
            )
            .await,
        ) {
            MemoryError::Mcp { message } => {
                assert!(message.contains("no engram-mcp was started"), "{message}");
            }
            other => panic!("{bad:?}: expected the scope refusal, got {other:?}"),
        }
    }
}
