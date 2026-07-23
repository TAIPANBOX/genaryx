//! The "new agent" onboarding wizard (docs/ONBOARD.md, D15/B2).
//!
//! Registering an agent takes four hand-written artifacts that must agree
//! with each other: a Passport JSON (agent-passport v0.1), a
//! `TOKENFUSE_CLIENT_KEYS` entry, an identity-map fragment (open TokenFuse's
//! docs/20 map), and a Wardryx policy stub. This plane generates all four
//! consistently from one form, plus a Terraform alternative, and lists what
//! is already provisioned.
//!
//! Propose, never mutate: the commands return text blocks the operator
//! copies and commits themselves. The ONE convenience write is
//! [`commands::onboard_write_passport`], which stages the passport file into
//! the local passports dir (`~/.taipan/passports/` by convention) for the
//! operator to commit to their own git. Nothing here touches the network,
//! the identity map file, env vars, or the Cloud, and the minted client-key
//! secret is shown once and never persisted by the console.
//!
//! Unlike every other plane there is no `env`/`state` pair: onboard has no
//! service to discover and no client to hold. Both commands re-read the
//! local filesystem fresh on every call.

pub mod commands;
