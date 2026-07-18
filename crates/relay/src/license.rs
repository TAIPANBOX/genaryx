//! License gate SEAM (docs/PHASE5.md "license" module; itrat-console/13
//! D12.5: "The relay ... refuses to start without an ML-DSA license").
//!
//! SIM: [`LicenseGate::permissive`] always grants and says so loudly, so no
//! sim run can be mistaken for a licensed one. R1 replaces the body of
//! [`LicenseGate::check`] with a real verification of an installed offline
//! license bundle via `genaryx_signing::mldsa::verify` (the exact machinery
//! D8 already built and proved for evidence-pack signatures) -- the seam
//! here is deliberately the only thing that needs to change; every caller
//! keeps calling `LicenseGate::check` the same way.

use thiserror::Error;

/// Why the gate refused to grant. The sim gate can never actually produce
/// this (see [`LicenseGate::permissive`]); it exists now so `main`'s
/// fail-closed handling of a denied gate is real code today, not a TODO.
#[derive(Debug, Error)]
pub enum LicenseError {
    #[error(
        "no valid license installed (TODO R1: verify an ML-DSA-signed offline license bundle \
         via genaryx_signing::mldsa::verify, D8 licensing)"
    )]
    NotLicensed,
}

/// Gate that must pass before the relay serves any traffic.
pub struct LicenseGate {
    permissive: bool,
}

impl LicenseGate {
    /// The sim-phase gate (docs/PHASE5.md "Sim-first deltas" -- "Do not
    /// block the sim build on real licensing"): always grants, but logs
    /// plainly so a sim deployment is never confused for a licensed one.
    pub fn permissive() -> Self {
        eprintln!(
            "genaryx-relay: sim: license gate bypassed (TODO R1: wire genaryx-signing::mldsa \
             against an installed offline license bundle)"
        );
        Self { permissive: true }
    }

    /// `Ok(())` iff the relay may serve traffic. Kept fail-closed in shape
    /// even though the sim impl can never deny, so R1 only needs a body
    /// change here, never a call-site change.
    pub fn check(&self) -> Result<(), LicenseError> {
        if self.permissive {
            Ok(())
        } else {
            Err(LicenseError::NotLicensed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissive_gate_always_grants() {
        let gate = LicenseGate::permissive();
        assert!(gate.check().is_ok());
    }
}
