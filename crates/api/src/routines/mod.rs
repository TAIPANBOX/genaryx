//! Routines (I7b "Routines tab"): a READ-ONLY surface over what stack-up's
//! `routines.sh` already recorded under `$STACK_UP_HOME/routines/` - the
//! stable contract documented in `~/Development/stack-up/README.md`
//! ("Scheduled governance runs" / "The record"), schema
//! `stackup.routine-run/v1`.
//!
//! **Non-goal, stated plainly**: this console does NOT install, uninstall,
//! or run a routine. That remains the operator's own `routines.sh` on the
//! box (`./routines.sh install`, `run <name>`, `uninstall`). These two
//! commands only SURFACE what is already there: the schedule state
//! (installed as a timer, per `installed.txt`) and the recorded history
//! (`history.ndjson` / `status/<name>.json`). A future "install/run from the
//! console" is an explicit, named follow-up (docs/ROUTINES.md), not
//! something this plane does today.
//!
//! [`commands::RoutineRunDto`] mirrors the v1 record schema field-for-field
//! and never redefines it - stack-up owns that contract, this console only
//! reads it.
//!
//! Unlike every OTHER plane in this crate, [`env`] does NOT consult
//! `genaryx_core::taipan_home` or any `taipan up` descriptor at all: routines
//! is a stack-up concept (`$STACK_UP_HOME`), not a taipan-up plane
//! (`$TAIPAN_HOME`) - see [`env`]'s own module doc for why conflating the two
//! would be a real bug, not a simplification.
//!
//! Like `crate::onboard`, there is no `state` module here: every call
//! re-reads a handful of small local files fresh (cheap - a few JSON files
//! and one ndjson file, no service, no descriptor, no client to hold), so
//! there is nothing worth caching and nothing to bootstrap - see
//! `onboard/mod.rs`'s identical "no env/state pair" rationale, which this
//! plane follows for `state` specifically (it still has its own `env`,
//! unlike onboard, because the routines-dir resolution rule is non-trivial
//! enough to deserve its own tested module).

pub mod commands;
pub mod env;
