//! A standing grant: "always allow this tool for this agent", asked on a hold.
//!
//! The hold it ends is wardryx's no-grant floor, and what ends that for good is
//! a `grant_tool` policy. stack-single's `./install.sh agent-allow` writes the
//! same policy, under the same id and in the same shape, so a grant made here
//! shows in `agent-list`, `agent-deny` takes it back, and the two add to one
//! policy per agent instead of leaving two (CLAUDE.md invariant 47).
//!
//! Everything here is a plain function of what a hold carried: the I/O and the
//! journal are `commands::policy_grant_tool`'s.

use genaryx_connectors::{Approval, Policy};
use sha2::{Digest, Sha256};

/// `context["hold_kind"]` on a tool call nothing grants, the one hold a grant
/// lifts. wardryx's `pdp.HoldKindNoGrant`.
const NO_GRANT: &str = "no_grant";
/// The sources `agent-allow` writes: an agent token, and `none`, because scopyx
/// asks wardryx about the same agent's fetch and sends no source at all.
const TOKEN_SOURCES: [&str; 2] = ["agent-token", "none"];
/// `agent-allow --static-key`: the same, and a static key.
const STATIC_SOURCES: [&str; 3] = ["agent-token", "client-key", "none"];

/// What one hold says about a standing grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub agent: String,
    pub tool: String,
    /// The held call came with a static key, so a grant that left `client-key`
    /// out would turn this hold into a refusal.
    pub static_key: bool,
}

/// The id of an agent's one grant policy: `grant-<slug>-<hash>`, byte for byte
/// `tool_hold_id` in stack-single's install.sh. The hash is what keeps
/// `agent://a/b-c` and `agent://a-b/c` apart.
pub fn policy_id(agent: &str) -> String {
    let rest = agent.strip_prefix("agent://").unwrap_or(agent);
    let slug: String = rest
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
                b as char
            } else {
                '-'
            }
        })
        .collect();
    let digest = Sha256::digest(agent.as_bytes());
    let hash: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("grant-{slug}-{hash}")
}

/// Whether `agent` names one agent in one trust domain, which is what a grant
/// may be written on.
///
/// This is `agent-allow`'s pattern with one difference: no `*`. A person there
/// types a glob on purpose. Here the id is whatever the caller wrote, so a `*`
/// in it would turn one click on one hold into a grant to a whole domain.
pub fn pins_trust_domain(agent: &str) -> bool {
    let Some(rest) = agent.strip_prefix("agent://") else {
        return false;
    };
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, path),
        None => (rest, ""),
    };
    let (domain, port) = match authority.rsplit_once(':') {
        Some((domain, port)) => (domain, Some(port)),
        None => (authority, None),
    };
    let domain_ok = !domain.is_empty()
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'-'));
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let path_ok = path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b':' | b'/' | b'-'));
    domain_ok && port_ok && path_ok
}

/// `agent-allow`'s tool pattern, for one tool.
fn is_tool_name(tool: &str) -> bool {
    !tool.is_empty()
        && tool
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-'))
}

/// The one tool of a hold. The call it is about when it names one, else the only
/// tool it lists: a hold listing several has no one tool to always allow.
fn held_tool(held: &Approval) -> Result<String, String> {
    if let Some(name) = held.tool_call().and_then(|call| call.name) {
        return Ok(name);
    }
    let tools = held.tool_names().unwrap_or_default();
    match tools.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err("this hold names no tool, so there is nothing to always allow".to_string()),
        _ => Err(format!(
            "this hold names {} tools and no one call, so there is no one tool to always allow",
            tools.len()
        )),
    }
}

/// What a hold allows to be granted, or the line that says why not. A pending
/// hold, or a hold a person already granted: the offer in the inbox names one of
/// those. A denied one is no basis for anything.
pub fn plan(held: &Approval) -> Result<Plan, String> {
    if !held.pending && held.decision.as_deref() != Some("grant") {
        return Err("a hold a person denied is no basis for a standing grant".to_string());
    }
    if held.hold_kind() != Some(NO_GRANT) {
        return Err(
            "this hold is not the no-grant floor, and a grant does not lift any other hold: decide it once"
                .to_string(),
        );
    }
    if !pins_trust_domain(&held.agent_id) {
        return Err(format!(
            "{} names no trust domain, and wardryx refuses a grant that does not: give it an id of the form agent://<domain>/<name>",
            held.agent_id
        ));
    }
    let tool = held_tool(held)?;
    if !is_tool_name(&tool) {
        return Err(format!("{tool:?} cannot be written as a tool in a grant"));
    }
    Ok(Plan {
        agent: held.agent_id.clone(),
        tool,
        static_key: held.identity_source() == Some("client-key"),
    })
}

/// The policy to write. A policy the agent already holds is added to, as
/// `tool_hold_allow` does: its tools stay, `*` stays `*`, a grant that took a
/// static key keeps taking one, and one written with no source requirement, on a
/// box with agent tokens off, stays that way, because adding one would deny.
pub fn grant_policy(plan: &Plan, kept: Option<&Policy>, date: &str, why: &str) -> Policy {
    let kept_tools = kept.map(|p| p.grant_tool.as_slice()).unwrap_or_default();
    let mut tools: Vec<String> = kept_tools.to_vec();
    tools.push(plan.tool.clone());
    tools.sort();
    tools.dedup();
    if kept_tools.iter().any(|t| t == "*") {
        tools = vec!["*".to_string()];
    }
    let sources: &[&str] = match kept.map(|p| p.require_identity_source.as_slice()) {
        Some([]) => &[],
        Some(sources) if sources.iter().any(|s| s == "client-key") => &STATIC_SOURCES,
        _ if plan.static_key => &STATIC_SOURCES,
        _ => &TOKEN_SOURCES,
    };
    Policy {
        name: format!("grant {} {date}: {why}", plan.agent),
        target: plan.agent.clone(),
        grant_tool: tools,
        require_identity_source: sources.iter().map(|s| s.to_string()).collect(),
        deny_if_chain_unproven: true,
        ..Policy::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn held(agent: &str, context: serde_json::Value) -> Approval {
        serde_json::from_value(json!({
            "approval_id": "ap_1",
            "agent_id": agent,
            "run_id": "run-1",
            "requested_at": "2026-11-07T10:00:00Z",
            "pending": true,
            "context": context,
        }))
        .expect("an approval")
    }

    fn floor_hold(agent: &str) -> Approval {
        held(
            agent,
            json!({
                "hold_kind": "no_grant",
                "identity_source": "agent-token",
                "tool_names": ["ask"],
                "tool_call": {"name": "ask", "target": "typryx"},
            }),
        )
    }

    /// Each right-hand side is what stack-single's `tool_hold_id` printed for the
    /// left, so a console grant and `agent-allow` land on the same policy.
    #[test]
    fn the_id_is_the_id_agent_allow_writes() {
        for (agent, id) in [
            (
                "agent://acme.example/agents/bot-1",
                "grant-acme.example-agents-bot-1-2ffe7f24",
            ),
            (
                "agent://local/agents/operator",
                "grant-local-agents-operator-72ea80ef",
            ),
            (
                "agent://mockryx.local:8443/a~b/c:d",
                "grant-mockryx.local-8443-a-b-c-d-9dc4b08f",
            ),
            (
                "agent://ci.example/tool-user",
                "grant-ci.example-tool-user-0beb4267",
            ),
        ] {
            assert_eq!(policy_id(agent), id);
        }
    }

    #[test]
    fn two_agents_whose_slugs_meet_keep_two_ids() {
        assert_ne!(policy_id("agent://a/b-c"), policy_id("agent://a-b/c"));
    }

    #[test]
    fn an_id_with_a_trust_domain_is_recognised() {
        for agent in [
            "agent://local/agents/operator",
            "agent://acme.example",
            "agent://acme.example/",
            "agent://acme.example:8443/a/b",
        ] {
            assert!(pins_trust_domain(agent), "{agent}");
        }
    }

    #[test]
    fn an_id_with_no_trust_domain_is_refused() {
        for agent in [
            "",
            "agent-42",
            "agent://",
            "agent:///x",
            "agent://*/x",
            "agent://ac*/x",
            "agent://acme:port/x",
            "agent://acme:/x",
            "spiffe://acme/x",
        ] {
            assert!(!pins_trust_domain(agent), "{agent}");
        }
    }

    #[test]
    fn a_glob_in_an_id_a_caller_wrote_is_not_a_grant_to_the_domain() {
        assert!(!pins_trust_domain("agent://acme.example/*"));
        assert!(!pins_trust_domain("agent://acme.example/agents/?"));
    }

    #[test]
    fn a_floor_hold_plans_the_tool_of_its_call() {
        let plan = plan(&floor_hold("agent://acme.example/a")).expect("a plan");
        assert_eq!(plan.agent, "agent://acme.example/a");
        assert_eq!(plan.tool, "ask");
        assert!(!plan.static_key);
    }

    #[test]
    fn a_hold_of_any_other_kind_is_not_planned() {
        let hold = held(
            "agent://acme.example/a",
            json!({"tool_names": ["ask"], "reason": "over the human threshold"}),
        );
        assert!(plan(&hold).unwrap_err().contains("not the no-grant floor"));
    }

    #[test]
    fn an_id_with_no_domain_is_not_planned_and_the_line_says_why() {
        let reason = plan(&floor_hold("agent-42")).unwrap_err();
        assert!(reason.contains("names no trust domain"), "{reason}");
    }

    #[test]
    fn a_hold_of_several_tools_is_not_planned() {
        let hold = held(
            "agent://acme.example/a",
            json!({"hold_kind": "no_grant", "tool_names": ["a", "b"]}),
        );
        assert!(plan(&hold).unwrap_err().contains("no one tool"));
    }

    #[test]
    fn a_denied_hold_is_no_basis_and_a_granted_one_is() {
        let mut hold = floor_hold("agent://acme.example/a");
        hold.pending = false;
        hold.decision = Some("deny".to_string());
        assert!(plan(&hold).unwrap_err().contains("denied"));
        hold.decision = Some("grant".to_string());
        assert!(plan(&hold).is_ok());
    }

    #[test]
    fn a_new_grant_has_the_shape_agent_allow_writes() {
        let plan = plan(&floor_hold("agent://acme.example/a")).unwrap();
        let body = grant_policy(&plan, None, "2026-11-07", "why");
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            json!({
                "name": "grant agent://acme.example/a 2026-11-07: why",
                "target": "agent://acme.example/a",
                "grant_tool": ["ask"],
                "require_identity_source": ["agent-token", "none"],
                "deny_if_chain_unproven": true,
            })
        );
    }

    #[test]
    fn a_hold_that_came_with_a_static_key_is_granted_one() {
        let hold = held(
            "agent://acme.example/a",
            json!({
                "hold_kind": "no_grant",
                "identity_source": "client-key",
                "tool_call": {"name": "ask"},
            }),
        );
        let plan = plan(&hold).unwrap();
        assert!(plan.static_key);
        let body = grant_policy(&plan, None, "2026-11-07", "why");
        assert_eq!(
            body.require_identity_source,
            ["agent-token", "client-key", "none"]
        );
    }

    #[test]
    fn a_policy_the_agent_holds_is_added_to() {
        let plan = plan(&floor_hold("agent://acme.example/a")).unwrap();
        let kept = Policy {
            grant_tool: vec!["fetch_url".to_string()],
            require_identity_source: vec![
                "agent-token".to_string(),
                "client-key".to_string(),
                "none".to_string(),
            ],
            ..Policy::default()
        };
        let body = grant_policy(&plan, Some(&kept), "2026-11-07", "why");
        assert_eq!(body.grant_tool, ["ask", "fetch_url"]);
        assert!(
            body.require_identity_source
                .contains(&"client-key".to_string()),
            "a grant that took a static key keeps taking one"
        );
    }

    #[test]
    fn every_tool_stays_every_tool() {
        let plan = plan(&floor_hold("agent://acme.example/a")).unwrap();
        let kept = Policy {
            grant_tool: vec!["*".to_string()],
            require_identity_source: vec!["agent-token".to_string()],
            ..Policy::default()
        };
        let body = grant_policy(&plan, Some(&kept), "2026-11-07", "why");
        assert_eq!(body.grant_tool, ["*"]);
    }

    #[test]
    fn a_grant_with_no_source_requirement_stays_without_one() {
        let plan = plan(&floor_hold("agent://acme.example/a")).unwrap();
        let kept = Policy {
            grant_tool: vec!["fetch_url".to_string()],
            ..Policy::default()
        };
        let body = grant_policy(&plan, Some(&kept), "2026-11-07", "why");
        assert!(body.require_identity_source.is_empty());
    }
}
