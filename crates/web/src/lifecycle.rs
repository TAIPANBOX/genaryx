//! Operator lifecycle blocking (Yurii, 2026-07-24): freeze an agent, stop a
//! business unit, stop a user. Two halves, both required:
//!
//! 1. ENFORCEMENT. A block writes a deny-all policy per affected agent into
//!    wardryx (`PUT /v1/policies/{id}`), so the PDP actually refuses the
//!    agent's work. Unblock deletes those policies again. wardryx is also the
//!    DURABLE record: the policies outlive this process, which is why
//!    [`rehydrate`] rebuilds the in-memory store from them at startup instead
//!    of trusting a console restart to have kept anything.
//! 2. REFLECTION. The store below is what every console read is projected
//!    through ([`project_runs`], [`project_graph`]), so a block reads
//!    `frozen`/`stopped` app-wide: Overview's spend-by-agent, the Money runs
//!    board, the watch dock, Agent 360's run list and the delegation graph all
//!    derive from those two commands on a real box. `lifecycle_blocks` serves
//!    the store itself, so the toggle buttons know which way to point without
//!    a per-entity record store the box does not have.
//!
//! Deliberately per-agent policies rather than one `agent://org/team/*` glob:
//! a unit or user block then needs no assumption about the PDP's glob
//! semantics, and unblock stays exact. The membership that a block expanded to
//! is not remembered anywhere; unblock instead lists wardryx and deletes every
//! policy under the entity's id prefix, so it also cleans up members that have
//! since left the unit.
//!
//! Kill is NOT modelled here. Killing a run is a Cloud-plane mutation
//! (`money_kill_run`), and a killed run already carries its own `killed` flag
//! from `GET /v1/runs`.

use genaryx_connectors::{Policy, WardryxClient, WardryxError};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Console-side block state, held behind an `RwLock` in [`crate::ctx::Ctx`]
/// and rebuilt from wardryx at startup by [`rehydrate`].
#[derive(Debug, Default)]
pub struct LifecycleStore {
    pub frozen_agents: HashSet<String>,
    pub stopped_units: HashSet<String>,
    pub stopped_users: HashSet<String>,
    /// `agent id -> owner handle`, cached whenever identities are resolved so
    /// a stopped USER's agents can be projected without an identity fetch on
    /// every read. Missing entries simply mean that agent is not projected as
    /// user-stopped, never a wrong badge.
    pub agent_owners: HashMap<String, String>,
}

impl LifecycleStore {
    /// The lifecycle badge for one agent id, or `None` when it is not
    /// operator-blocked. Frozen (the agent itself) outranks stopped (it was
    /// caught by its unit or its owner).
    pub fn state_for(&self, agent_id: &str) -> Option<&'static str> {
        if self.frozen_agents.contains(agent_id) {
            return Some("frozen");
        }
        if let Some(team) = team_of(agent_id)
            && self.stopped_units.contains(team)
        {
            return Some("stopped");
        }
        if let Some(owner) = self.agent_owners.get(agent_id)
            && self.stopped_users.contains(owner)
        {
            return Some("stopped");
        }
        None
    }

    /// The shape `lifecycle_blocks` serves to the frontend.
    pub fn to_json(&self) -> Value {
        let mut agents: Vec<&String> = self.frozen_agents.iter().collect();
        let mut units: Vec<&String> = self.stopped_units.iter().collect();
        let mut users: Vec<&String> = self.stopped_users.iter().collect();
        agents.sort();
        units.sort();
        users.sort();
        serde_json::json!({ "agents": agents, "units": units, "users": users })
    }
}

/// The `<team>` segment of an `agent://org/team/name` id.
pub fn team_of(agent_id: &str) -> Option<&str> {
    let rest = agent_id.strip_prefix("agent://")?;
    let mut parts = rest.splitn(3, '/');
    let _org = parts.next()?;
    let team = parts.next()?;
    parts.next()?; // a name segment must exist for this to be an agent id
    Some(team)
}

/// Marks every policy this console wrote for a block, so [`rehydrate`] and
/// unblock can find them again without keeping their own index.
const ID_PREFIX: &str = "console-block-";

/// The id prefix every policy for one blocked entity shares. An agent block
/// writes exactly this id; a unit or user block writes `<prefix>--<agent>`
/// per member, so unblock can delete the whole family by prefix.
pub fn block_prefix(kind: &str, key: &str) -> String {
    format!("{ID_PREFIX}{kind}-{}", sanitize(key))
}

/// Policy ids are URL path segments, so anything that is not alphanumeric
/// becomes a hyphen. This is lossy on purpose: the exact entity key is
/// carried in the policy NAME (see [`block_name`]), which round-trips
/// through wardryx untouched, so nothing depends on reversing this.
fn sanitize(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The policy name that carries the block back: `console-block:<kind>:<key>`.
/// [`rehydrate`] parses exactly this to rebuild the store after a restart.
pub fn block_name(kind: &str, key: &str) -> String {
    format!("console-block:{kind}:{key}")
}

fn parse_block_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("console-block:")?;
    let (kind, key) = rest.split_once(':')?;
    if kind.is_empty() || key.is_empty() {
        return None;
    }
    Some((kind, key))
}

/// A deny-all-work policy for one agent.
///
/// wardryx has no single "deny everything" primitive: its PDP denies per
/// DIMENSION (cost, steps, attestation, domain, named tool), so a block has to
/// close every dimension at once. The combination below was probed against a
/// live wardryx (2026-07-24) and the results are why each field is here:
///
/// - `deny_above_usd` a tenth of a cent, below any real model call, denies
///   every priced request. It MUST be non-zero: serde's `skip_serializing_if`
///   mirrors Go's `omitempty`, so a literal `0.0` would never reach the wire.
///   On its own this was NOT enough - an unpriced request still passed.
/// - `max_steps: 1` denies anything declaring a step (the PDP denies when
///   `steps` reaches or exceeds it).
/// - `deny_if_unattested` denies every request that carries no attestation.
/// - `allow_domains` with a sentinel nothing can match denies any request that
///   names a domain, since a domain outside the allow-list is a deny.
///
/// Verified against the live PDP: a realistic agent call carrying cost, steps,
/// a tool and an attestation is DENIED, and an unrelated agent is unaffected.
/// The one request this cannot deny is a fully empty probe: attested, no cost,
/// no steps, no tool, no domain. `deny_tool` does not take a wildcard (exact
/// names only), so that gap is the PDP's vocabulary, not an oversight here.
/// It admits nothing an agent could do work with.
fn deny_all_policy(name: &str, target: &str) -> Policy {
    Policy {
        name: name.to_string(),
        target: target.to_string(),
        deny_above_usd: 0.001,
        max_steps: 1,
        deny_if_unattested: true,
        allow_domains: vec![BLOCK_DOMAIN_SENTINEL.to_string()],
        ..Policy::default()
    }
}

/// A domain no request can legitimately carry, so an `allow_domains` holding
/// only this denies every request that names any domain at all.
const BLOCK_DOMAIN_SENTINEL: &str = "console.blocked.invalid";

pub fn admin_client() -> Result<WardryxClient, String> {
    let env = genaryx_api::policy::env::discover()
        .ok_or_else(|| "no wardryx environment is resolved to enforce a block".to_string())?;
    WardryxClient::new(env.wardryx_url, env.admin_bearer).map_err(|e| e.to_string())
}

/// Write one deny-all policy per agent in `agents`, all under this entity's
/// id prefix and carrying its `kind`/`key` in the policy name.
pub async fn block(kind: &str, key: &str, agents: &[String]) -> Result<usize, String> {
    if agents.is_empty() {
        return Err(format!(
            "nothing to block: no agents resolved for this {kind}"
        ));
    }
    let client = admin_client()?;
    let prefix = block_prefix(kind, key);
    let name = block_name(kind, key);
    let mut written = 0usize;
    for agent in agents {
        // An agent block writes the bare prefix; a unit/user block writes one
        // id per member under it.
        let id = if kind == "agent" {
            prefix.clone()
        } else {
            format!("{prefix}--{}", sanitize(agent))
        };
        client
            .put_policy(&id, &deny_all_policy(&name, agent))
            .await
            .map_err(|e| format!("wardryx refused the block policy for {agent}: {e}"))?;
        written += 1;
    }
    Ok(written)
}

/// Delete every policy this console wrote for one entity. Listing rather than
/// remembering what a block expanded to keeps unblock exact even when the
/// unit's membership changed in between. An already-absent policy (404) is
/// success, so unblock is idempotent.
pub async fn unblock(kind: &str, key: &str) -> Result<usize, String> {
    let client = admin_client()?;
    let prefix = block_prefix(kind, key);
    let policies = client
        .list_policies()
        .await
        .map_err(|e| format!("wardryx would not list policies to unblock: {e}"))?;
    let mut removed = 0usize;
    for record in policies {
        if !record.id.starts_with(&prefix) {
            continue;
        }
        match client.delete_policy(&record.id).await {
            Ok(()) => removed += 1,
            Err(WardryxError::Api { status: 404, .. }) => {}
            Err(e) => return Err(format!("wardryx refused to remove {}: {e}", record.id)),
        }
    }
    Ok(removed)
}

/// Rebuild the store from wardryx at startup: the policies ARE the durable
/// record of what an operator blocked, so a console restart must not silently
/// present a blocked fleet as running. Best effort by design - a wardryx that
/// is not resolved yet leaves the store empty rather than failing startup.
pub async fn rehydrate() -> Result<LifecycleStore, String> {
    let client = admin_client()?;
    let policies = client
        .list_policies()
        .await
        .map_err(|e| format!("wardryx would not list policies: {e}"))?;
    let mut store = LifecycleStore::default();
    for record in policies {
        if !record.id.starts_with(ID_PREFIX) {
            continue;
        }
        let Some((kind, key)) = parse_block_name(&record.policy.name) else {
            continue;
        };
        match kind {
            "agent" => {
                store.frozen_agents.insert(key.to_string());
            }
            "unit" => {
                store.stopped_units.insert(key.to_string());
            }
            "user" => {
                store.stopped_users.insert(key.to_string());
            }
            _ => {}
        }
    }
    Ok(store)
}

/// Stamp `lifecycle` on every run whose agent is operator-blocked, and mark it
/// not-live (`killed`) so it drops out of active counts and every
/// spend-by-agent surface, exactly like the demo mock does client-side.
/// Mutates the serialized reply in place; a shape it does not recognise is
/// left untouched.
pub fn project_runs(value: &mut Value, store: &LifecycleStore) {
    let Some(runs) = value.as_array_mut() else {
        return;
    };
    for run in runs {
        let Some(agent_id) = run.get("agent_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(state) = store.state_for(agent_id) else {
            continue;
        };
        if let Some(obj) = run.as_object_mut() {
            obj.insert("lifecycle".to_string(), Value::String(state.to_string()));
            obj.insert("killed".to_string(), Value::Bool(true));
        }
    }
}

/// The same projection for the delegation graph: a blocked agent's node
/// carries `lifecycle`, so the Graph tab tints it like every other surface.
pub fn project_graph(value: &mut Value, store: &LifecycleStore) {
    let Some(nodes) = value.get_mut("nodes").and_then(Value::as_array_mut) else {
        return;
    };
    for node in nodes {
        let Some(id) = node.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(state) = store.state_for(id) else {
            continue;
        };
        if let Some(obj) = node.as_object_mut() {
            obj.insert("lifecycle".to_string(), Value::String(state.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> String {
        id.to_string()
    }

    #[test]
    fn team_is_the_second_segment_of_an_agent_id() {
        assert_eq!(
            team_of("agent://acme/treasury/reconciler"),
            Some("treasury")
        );
        // Not an agent id (no name segment), and not an agent URI at all.
        assert_eq!(team_of("agent://acme/treasury"), None);
        assert_eq!(team_of("user://acme/d.hayes"), None);
    }

    #[test]
    fn a_policy_name_round_trips_the_exact_entity_key() {
        let name = block_name("agent", "agent://acme/treasury/reconciler");
        assert_eq!(
            parse_block_name(&name),
            Some(("agent", "agent://acme/treasury/reconciler"))
        );
        // An id, by contrast, is deliberately lossy and is never parsed back.
        assert!(block_prefix("agent", "agent://acme/treasury/reconciler").starts_with(ID_PREFIX));
        assert_eq!(parse_block_name("some-unrelated-policy"), None);
    }

    #[test]
    fn frozen_outranks_stopped_and_both_beat_live() {
        let mut store = LifecycleStore::default();
        store.frozen_agents.insert(agent("agent://acme/sre/rca"));
        store.stopped_units.insert("treasury".to_string());
        store.stopped_users.insert("d.hayes".to_string());
        store
            .agent_owners
            .insert(agent("agent://acme/lending/scorer"), "d.hayes".to_string());

        assert_eq!(store.state_for("agent://acme/sre/rca"), Some("frozen"));
        assert_eq!(
            store.state_for("agent://acme/treasury/reconciler"),
            Some("stopped")
        );
        assert_eq!(
            store.state_for("agent://acme/lending/scorer"),
            Some("stopped")
        );
        assert_eq!(store.state_for("agent://acme/data/checker"), None);
    }

    #[test]
    fn a_block_policy_closes_every_deny_dimension() {
        // The live PDP denies per dimension, so dropping any one of these
        // silently reopens a way for a blocked agent to keep working (probed
        // against a live wardryx: cost alone let unpriced calls through).
        let p = deny_all_policy("console-block:agent:a", "agent://acme/sre/rca");
        assert_eq!(p.target, "agent://acme/sre/rca");
        assert!(p.deny_above_usd > 0.0, "a zero is dropped by omitempty");
        assert!(p.deny_above_usd < 0.01, "must sit below any real call");
        assert_eq!(p.max_steps, 1);
        assert!(p.deny_if_unattested);
        assert_eq!(p.allow_domains, vec![BLOCK_DOMAIN_SENTINEL.to_string()]);
    }

    #[test]
    fn runs_and_graph_nodes_are_projected_in_place() {
        let mut store = LifecycleStore::default();
        store.frozen_agents.insert(agent("agent://acme/sre/rca"));

        let mut runs = serde_json::json!([
            { "run_id": "r1", "agent_id": "agent://acme/sre/rca", "killed": false },
            { "run_id": "r2", "agent_id": "agent://acme/data/ok", "killed": false },
        ]);
        project_runs(&mut runs, &store);
        assert_eq!(runs[0]["lifecycle"], "frozen");
        assert_eq!(runs[0]["killed"], true);
        assert!(runs[1].get("lifecycle").is_none());
        assert_eq!(runs[1]["killed"], false);

        let mut graph = serde_json::json!({
            "nodes": [{ "id": "agent://acme/sre/rca" }, { "id": "user://acme/d.hayes" }],
        });
        project_graph(&mut graph, &store);
        assert_eq!(graph["nodes"][0]["lifecycle"], "frozen");
        assert!(graph["nodes"][1].get("lifecycle").is_none());
    }
}
