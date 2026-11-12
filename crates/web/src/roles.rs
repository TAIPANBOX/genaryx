//! Console roles and the per-command role gate (docs/CONSOLE-IDP.md, B3/1).
//!
//! Three roles, least-privilege ordered: `viewer` < `approver` < `admin`.
//! Every command the console can dispatch is classified into the minimum role
//! that may run it, and the web command chokepoint refuses (403) a caller
//! below that minimum BEFORE the command executes, so gating lives in one
//! place rather than sprinkled across the planes.
//!
//! The classification is data, not a heuristic: every dispatch arm is named
//! explicitly, and a test asserts the classified set equals the live dispatch
//! set, so a new command cannot be added without being placed. An unknown
//! name (a probe, or a not-yet-classified command) requires `admin` - fail
//! closed, never open.
//!
//! "The live dispatch set" is meant literally. That test used to compare the
//! three lists below against a FOURTH hand-maintained list in this same file,
//! which is not the dispatcher and cannot disagree with it until somebody
//! updates one and not the other: adding a command to `dispatch.rs` and
//! nowhere else passed. It now reads `dispatch.rs` itself.

use serde::Serialize;

/// A console role. Ordered so `>=` expresses "at least this privilege".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Approver,
    Admin,
}

impl Role {
    /// The wire string for the session payload and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Approver => "approver",
            Role::Admin => "admin",
        }
    }
}

/// One row of the command table: what a command needs before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    /// The command's name, as `dispatch.rs` routes it.
    pub name: &'static str,
    /// The minimum role that may run it.
    pub role: Role,
    /// Whether a run needs a fresh passkey ceremony: a per-action assertion
    /// from a person the authenticator verified (`main.rs`'s `webauthn_gate`),
    /// whenever the caller has a passkey enrolled. Set on the sensitive
    /// writes: money, delegations, tunnel peers, retained reservations, unit
    /// caps and approval grants.
    pub ceremony: bool,
}

const fn gate(name: &'static str, role: Role, ceremony: bool) -> Gate {
    Gate {
        name,
        role,
        ceremony,
    }
}

/// Every command the console can dispatch, placed once: the role that may run
/// it, and whether it needs the passkey ceremony. A test asserts this table and
/// the live dispatch set are the same, so a command cannot be added without a
/// row. The ceremony is read from this column and from nowhere else.
const COMMANDS: &[Gate] = &[
    // Reads and non-mutating inspections: `viewer` and up.
    gate("admission_check", Role::Viewer, false),
    gate("admission_status", Role::Viewer, false),
    gate("agent_events", Role::Viewer, false),
    gate("agent_profile", Role::Viewer, false),
    gate("agent_stops", Role::Viewer, false),
    gate("agent_graph", Role::Viewer, false),
    gate("agent_slice", Role::Viewer, false),
    gate("bus_quarantine", Role::Viewer, false),
    gate("bus_status", Role::Viewer, false),
    // The case file for an incident: the reads the console already makes,
    // joined, with no model in the path. A read, and a viewer may make it.
    gate("case_file", Role::Viewer, false),
    gate("copilot_ask", Role::Viewer, false),
    gate("copilot_explain", Role::Viewer, false),
    // Felyx reading the case file of one incident: no tool loop, no proposal.
    gate("copilot_explain_case", Role::Viewer, false),
    gate("copilot_status", Role::Viewer, false),
    gate("credentials_gateway_retained_runs", Role::Viewer, false),
    gate("credentials_keys", Role::Viewer, false),
    // The gateway's settings, keys summary and price book, read-only.
    gate("credentials_posture", Role::Viewer, false),
    gate("credentials_status", Role::Viewer, false),
    gate("crypto_scan_cbom", Role::Viewer, false),
    gate("crypto_scan_evidence", Role::Viewer, false),
    gate("crypto_scan_ncsc", Role::Viewer, false),
    gate("crypto_status", Role::Viewer, false),
    gate("crypto_verify_evidence", Role::Viewer, false),
    gate("drills_status", Role::Viewer, false),
    gate("evidence_status", Role::Viewer, false),
    gate("identity_list_alerts", Role::Viewer, false),
    gate("identity_list_identities", Role::Viewer, false),
    gate("identity_list_remediations", Role::Viewer, false),
    gate("identity_status", Role::Viewer, false),
    gate("lifecycle_blocks", Role::Viewer, false),
    gate("memory_recall", Role::Viewer, false),
    gate("memory_stats", Role::Viewer, false),
    gate("memory_status", Role::Viewer, false),
    gate("memory_why", Role::Viewer, false),
    gate("money_incidents", Role::Viewer, false),
    // D8: the gateway's retained reservations, read-only.
    gate("money_list_retained", Role::Viewer, false),
    gate("money_overview", Role::Viewer, false),
    gate("money_owners", Role::Viewer, false),
    gate("money_runs", Role::Viewer, false),
    gate("money_spend_window", Role::Viewer, false),
    gate("money_savings", Role::Viewer, false),
    // Unit month caps and spend, read-only.
    gate("money_units", Role::Viewer, false),
    // Which gateways the Cloud still hears from, read-only.
    gate("money_gateways", Role::Viewer, false),
    gate("egress_recent", Role::Viewer, false),
    gate("money_status", Role::Viewer, false),
    gate("onboard_status", Role::Viewer, false),
    gate("policy_list_approvals", Role::Viewer, false),
    // Reading what a held call's arguments say is part of deciding it, so it
    // needs the role that decides; it changes nothing, so no ceremony
    // (CLAUDE.md invariant 48).
    gate("policy_approval_preview", Role::Approver, false),
    gate("policy_list_policies", Role::Viewer, false),
    gate("policy_status", Role::Viewer, false),
    // The proof of a candidate policy: `wardryx replay` run over the recorded
    // decisions, read only. It changes no policy and applies nothing.
    gate("policy_proof", Role::Viewer, false),
    // The proof of a candidate budget: `tokenfuse backtest` run over the calls
    // the gateway recorded, read only. It changes no budget and applies nothing.
    gate("budget_proof", Role::Viewer, false),
    // Sealing a case puts an operator's name to a decision in the console's own
    // chain. It changes nothing in any plane, so it needs no ceremony, and it is
    // an approver's act; reading a sealed pack back is a read.
    gate("case_seal", Role::Approver, false),
    gate("case_pack", Role::Viewer, false),
    // A typed remedy checked against an incident, read only: it proves where a
    // proof exists, names a drill and a route, and applies nothing.
    gate("remedy_check", Role::Viewer, false),
    gate("quality_list_baselines", Role::Viewer, false),
    gate("quality_list_run_summaries", Role::Viewer, false),
    gate("quality_run_scores", Role::Viewer, false),
    gate("quality_status", Role::Viewer, false),
    gate("policy_enforcement_status", Role::Viewer, false),
    gate("recent_events", Role::Viewer, false),
    gate("remote_cloud_list", Role::Viewer, false),
    // Which devices hold tunnel access is a read. Seeing it without being able
    // to change it is exactly what a reviewer needs.
    gate("remote_operator_wg_peers", Role::Viewer, false),
    gate("remote_ssh_check_reachable", Role::Viewer, false),
    gate("remote_status", Role::Viewer, false),
    gate("routines_history", Role::Viewer, false),
    gate("routines_status", Role::Viewer, false),
    gate("run_events", Role::Viewer, false),
    gate("stats_counts", Role::Viewer, false),
    // Commands that grant or deny a Wardryx approval: `approver` and up.
    gate("policy_decide_approval", Role::Approver, true),
    // Commands that change enforcement state, sign an action, delete data, drive
    // a remote box, spawn real work, or pair a device: `admin` only.
    // A standing grant: the same ceremony as a grant, and a higher floor, because
    // this one outlives the call and takes a person out of the next ones.
    gate("policy_grant_tool", Role::Admin, true),
    // Money plane: the signed Cloud mutations.
    gate("money_kill_run", Role::Admin, true),
    gate("money_set_budget", Role::Admin, true),
    gate("money_ack_incident", Role::Admin, false),
    // D8: closing a retained reservation settles money on the gateway, the
    // same floor as a budget change. Listing them is a read, below.
    gate("money_close_retained", Role::Admin, true),
    // A unit's central month cap: the gateways enforce it on every call of
    // the unit, the same floor as a run budget.
    gate("money_set_unit_budget", Role::Admin, true),
    // Lifecycle: freezing an agent, or stopping a whole unit or user, writes
    // deny-all wardryx policies and records the block. A real enforcement
    // change on the fleet - admin only. Reading the blocks back
    // (`lifecycle_blocks`) is an ordinary viewer read, listed below.
    gate("agent_block", Role::Admin, false),
    gate("unit_block", Role::Admin, false),
    gate("user_block", Role::Admin, false),
    // Identity: a rescan spawns a detector run.
    gate("identity_rescan", Role::Admin, false),
    // Drills fire crafted traffic at the operator's own gateway.
    gate("drills_run", Role::Admin, false),
    // Evidence assembles AND signs a pack.
    gate("evidence_build", Role::Admin, false),
    // Memory forget deletes recorded memory.
    gate("memory_forget", Role::Admin, false),
    // Onboard generates a minted secret and stages a passport write.
    gate("onboard_generate", Role::Admin, false),
    gate("onboard_write_passport", Role::Admin, false),
    // Admission's baseline leg fires real gateway calls under the newcomer's
    // own key and spends real provider money (docs/ADMISSION.md) - the same
    // "admin only" posture as Drills' `drills_run` just above.
    gate("admission_baseline", Role::Admin, false),
    // Copilot: logging a proposal as approved leads to a signed action.
    gate("copilot_log_proposal_approved", Role::Admin, false),
    // Remote: everything that touches or reaches the client-hosted box.
    gate("remote_set_environment", Role::Admin, false),
    gate("remote_wg_connect", Role::Admin, false),
    gate("remote_wg_disconnect", Role::Admin, false),
    gate("remote_ssh_read_file", Role::Admin, false),
    gate("remote_ssh_tail_start", Role::Admin, false),
    gate("remote_ssh_tail_stop", Role::Admin, false),
    // Mutates THIS box's own local WireGuard server (adds a peer) rather
    // than reaching a client-hosted one, but it is still a real mutation
    // that hands out tunnel access to this box, the same posture as every
    // other admin-only Remote command above.
    gate("remote_operator_wg_config", Role::Admin, true),
    // Revoking cuts a device's access to the control plane. Listing the peers
    // is deliberately NOT here: seeing who holds access is a read, and a
    // reviewer should be able to look without being able to change anything.
    gate("remote_operator_wg_revoke", Role::Admin, true),
    // Hetzner inventory takes a live cloud API token as a plain command
    // argument, so this is the one "read" on the console that carries a
    // secret in its request body. A viewer could hand the console any token
    // and have it call out to Hetzner holding it. That is a different act
    // from the sibling `remote_cloud_list`, which spawns the operator's own
    // already-authenticated CLI on their own machine and never sees a
    // credential at all, and which stays a viewer read for exactly that
    // reason. Classified by what crosses the boundary, not by whether the
    // upstream call happens to be a GET.
    gate("remote_hetzner_list", Role::Admin, false),
    // Cuts an agent's or a user's delegated authority at vouchryx, the same
    // reason `remote_operator_wg_revoke` is admin-only just above: it takes
    // access away mid-incident, a deliberate, attributed act, not a read.
    gate("delegation_revoke", Role::Admin, true),
    // Stopping an agent bans it at vouchryx and kills its runs: the two acts
    // above in one, so the same floor and the ceremony. Lifting the ban lets
    // it back, which is the same class of decision.
    gate("agent_stop", Role::Admin, true),
    gate("agent_lift", Role::Admin, true),
];

fn gate_of(command: &str) -> Option<&'static Gate> {
    COMMANDS.iter().find(|g| g.name == command)
}

/// The minimum role that may run `command`. An entirely unknown name requires
/// `admin` (fail closed), never a read.
pub fn required_role(command: &str) -> Role {
    gate_of(command).map_or(Role::Admin, |g| g.role)
}

/// Whether `command` needs a fresh passkey ceremony. An unknown name does too,
/// the same fail-closed rule as its role.
pub fn requires_ceremony(command: &str) -> bool {
    gate_of(command).is_none_or(|g| g.ceremony)
}

/// The commands whose row carries the ceremony.
pub fn ceremony_commands() -> impl Iterator<Item = &'static str> {
    COMMANDS.iter().filter(|g| g.ceremony).map(|g| g.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command name `crates/web/src/dispatch.rs` actually routes,
    /// read out of the dispatcher's own source at compile time.
    ///
    /// `include_str!` rather than a copied list, because a copied list is
    /// not the dispatcher: the previous version of this test compared the
    /// classified sets against another array in THIS file, so a command
    /// added to `dispatch.rs` and to neither list satisfied it, which is the
    /// exact case the test exists to catch. Reading the source cannot drift.
    fn dispatch_commands() -> std::collections::BTreeSet<String> {
        const SOURCE: &str = include_str!("dispatch.rs");

        // Scoped to the body of `dispatch`'s `match name` and stopping at its
        // catch-all, so no other string-literal match in the file can leak in.
        let (_, after_fn) = SOURCE
            .split_once("pub async fn dispatch(")
            .expect("dispatch.rs must define `pub async fn dispatch(`");
        let (_, body) = after_fn
            .split_once("match name {")
            .expect("`dispatch` must route on `match name {`");
        let end = body
            .find("\n        other =>")
            .expect("`match name` must end in its `other =>` catch-all");

        let found: std::collections::BTreeSet<String> =
            body[..end].lines().filter_map(arm_name).collect();

        // A parse that silently matched nothing would make every assertion
        // below vacuously true, which is worse than the hand-kept list it
        // replaced. The floor is deliberately far under the real count.
        assert!(
            found.len() > 50,
            "only {} command(s) parsed out of dispatch.rs, so the parse broke \
             rather than the dispatcher shrinking: {found:?}",
            found.len()
        );
        found
    }

    /// The command name in a `"name" => ...` match arm, if the line is one.
    fn arm_name(line: &str) -> Option<String> {
        let rest = line.trim_start().strip_prefix('"')?;
        let (name, after) = rest.split_once('"')?;
        after
            .trim_start()
            .starts_with("=>")
            .then(|| name.to_string())
    }

    /// The parser finds real arms, in both shapes `dispatch.rs` writes them
    /// (a braced block and a bare `Ok(reply(...))`), and does not invent any.
    #[test]
    fn the_dispatch_parse_reads_the_real_arms() {
        let found = dispatch_commands();
        assert!(found.contains("admission_baseline"), "a braced arm");
        assert!(found.contains("money_status"), "a bare-expression arm");
        assert!(found.contains("agent_block"));
        assert!(
            !found.contains("other"),
            "the catch-all is not a command name"
        );
        assert!(
            !found.contains("agent"),
            "`block_action`'s own tuple match must not leak in: {found:?}"
        );
    }

    #[test]
    fn every_dispatch_command_has_exactly_one_row() {
        use std::collections::BTreeSet;
        let mut seen = BTreeSet::new();
        let duplicated: Vec<&str> = COMMANDS
            .iter()
            .map(|g| g.name)
            .filter(|name| !seen.insert(*name))
            .collect();
        assert!(
            duplicated.is_empty(),
            "commands placed twice: {duplicated:?}"
        );

        let classified: BTreeSet<String> = COMMANDS.iter().map(|g| g.name.to_string()).collect();
        let dispatched = dispatch_commands();
        let unclassified: Vec<_> = dispatched.difference(&classified).collect();
        let orphan: Vec<_> = classified.difference(&dispatched).collect();
        assert!(
            unclassified.is_empty(),
            "dispatch commands with no role: {unclassified:?}"
        );
        assert!(
            orphan.is_empty(),
            "classified names not in dispatch (stale): {orphan:?}"
        );
    }

    /// The case file only reads: the same floor as every other read, and no
    /// ceremony, since nothing it does changes a plane.
    /// Felyx reading a case is an explanation: the same floor as
    /// `copilot_explain`, no ceremony, and it proposes nothing.
    #[test]
    fn felyx_reading_a_case_is_a_viewer_read() {
        assert_eq!(required_role("copilot_explain_case"), Role::Viewer);
        assert_eq!(
            required_role("copilot_explain_case"),
            required_role("copilot_explain")
        );
        assert!(!requires_ceremony("copilot_explain_case"));
    }

    /// A policy proof runs `wardryx replay` over the recorded decisions and
    /// applies nothing: a read, with no ceremony.
    #[test]
    fn proving_a_policy_is_a_viewer_read() {
        assert_eq!(required_role("policy_proof"), Role::Viewer);
        assert!(!requires_ceremony("policy_proof"));
    }

    #[test]
    fn proving_a_budget_is_a_viewer_read() {
        assert_eq!(required_role("budget_proof"), Role::Viewer);
        assert!(!requires_ceremony("budget_proof"));
    }

    #[test]
    fn sealing_a_case_is_an_approvers_act_and_reading_a_pack_is_a_read() {
        assert_eq!(required_role("case_seal"), Role::Approver);
        assert!(!requires_ceremony("case_seal"));
        assert_eq!(required_role("case_pack"), Role::Viewer);
        assert!(!requires_ceremony("case_pack"));
    }

    #[test]
    fn checking_a_remedy_is_a_viewer_read() {
        assert_eq!(required_role("remedy_check"), Role::Viewer);
        assert!(!requires_ceremony("remedy_check"));
    }

    /// Where a remedy is routed is a command a person runs through its ceremony,
    /// and nothing in the remedy module runs it. The names are held here, in the
    /// crate that has the table, so a route to a command that does not exist, or
    /// that a viewer could run without a passkey, is a failing test.
    #[test]
    fn every_remedy_route_is_a_real_admin_command_that_needs_the_ceremony() {
        assert!(!genaryx_api::remedy::ROUTED_COMMANDS.is_empty());
        for command in genaryx_api::remedy::ROUTED_COMMANDS {
            assert!(gate_of(command).is_some(), "{command} is not a command");
            assert_eq!(required_role(command), Role::Admin, "{command}");
            assert!(requires_ceremony(command), "{command}");
        }
    }

    #[test]
    fn reading_a_case_is_a_viewer_read() {
        assert_eq!(required_role("case_file"), Role::Viewer);
        assert!(!requires_ceremony("case_file"));
    }

    #[test]
    fn an_unknown_command_requires_admin() {
        assert_eq!(required_role("definitely_not_a_command"), Role::Admin);
        assert_eq!(required_role(""), Role::Admin);
    }

    #[test]
    fn representative_commands_map_to_the_right_floor() {
        assert_eq!(required_role("money_overview"), Role::Viewer);
        assert_eq!(required_role("policy_decide_approval"), Role::Approver);
        assert_eq!(required_role("money_kill_run"), Role::Admin);
        assert_eq!(required_role("onboard_write_passport"), Role::Admin);
        assert_eq!(required_role("delegation_revoke"), Role::Admin);
    }

    /// D8: listing retained reservations is a read; closing one settles
    /// money on the gateway, the same floor as `money_set_budget`.
    #[test]
    fn closing_a_retained_reservation_is_admin_and_listing_is_a_read() {
        assert_eq!(required_role("money_list_retained"), Role::Viewer);
        assert_eq!(
            required_role("money_close_retained"),
            required_role("money_set_budget")
        );
        assert_eq!(required_role("money_close_retained"), Role::Admin);
    }

    /// A unit's central month cap is enforced by every gateway on every call
    /// of the unit: setting one is admin, the floor of `money_set_budget`;
    /// reading the caps is a read.
    #[test]
    fn setting_a_unit_cap_is_admin_and_reading_caps_is_a_read() {
        assert_eq!(required_role("money_units"), Role::Viewer);
        assert_eq!(
            required_role("money_set_unit_budget"),
            required_role("money_set_budget")
        );
        assert_eq!(required_role("money_set_unit_budget"), Role::Admin);
    }

    /// The gateway's posture (its switches, keys summary and price book) is a
    /// read: it carries no secret and changes nothing.
    #[test]
    fn reading_the_gateway_posture_is_a_viewer_read() {
        assert_eq!(required_role("credentials_posture"), Role::Viewer);
    }

    /// Which gateways the Cloud still hears from is a read any operator may
    /// make: a silent gateway must be visible to whoever is watching.
    #[test]
    fn reading_which_gateways_report_is_a_viewer_read() {
        assert_eq!(required_role("money_gateways"), Role::Viewer);
    }

    /// A command that takes a live cloud API token in its request body is not
    /// a viewer read, whatever the upstream HTTP verb is. Its sibling, which
    /// spawns the operator's own already-authenticated CLI and never sees a
    /// credential, still is.
    #[test]
    fn a_command_carrying_a_cloud_token_is_not_a_viewer_read() {
        assert_eq!(required_role("remote_hetzner_list"), Role::Admin);
        assert_eq!(required_role("remote_cloud_list"), Role::Viewer);
    }

    /// The commands that need a fresh passkey assertion per action, written
    /// out once here as the spec for the command table's ceremony column.
    const CEREMONY_SPEC: &[&str] = &[
        "agent_lift",
        "agent_stop",
        "delegation_revoke",
        "money_close_retained",
        "money_kill_run",
        "money_set_budget",
        "money_set_unit_budget",
        "policy_decide_approval",
        "policy_grant_tool",
        "remote_operator_wg_config",
        "remote_operator_wg_revoke",
    ];

    #[test]
    fn the_ceremony_is_a_column_of_the_command_table() {
        let mut got: Vec<&str> = ceremony_commands().collect();
        got.sort_unstable();
        let mut want = CEREMONY_SPEC.to_vec();
        want.sort_unstable();
        assert_eq!(got, want);
    }

    /// Stopping an agent bans it at the issuer and kills its runs, and
    /// lifting the ban lets it back: both are admin rows of the table with
    /// the ceremony, not commands that only fail closed for being unknown.
    #[test]
    fn stopping_and_lifting_an_agent_are_admin_rows_with_the_ceremony() {
        for command in ["agent_stop", "agent_lift"] {
            let row = gate_of(command).unwrap_or_else(|| panic!("{command} has no row"));
            assert_eq!(row.role, Role::Admin, "{command}");
            assert!(row.ceremony, "{command}");
        }
    }

    /// A grant ends one call and a standing grant ends every call after it, so
    /// the ceremony is the same and the role is not.
    #[test]
    fn a_standing_grant_is_an_admin_row_with_the_ceremony() {
        let row = gate_of("policy_grant_tool").expect("policy_grant_tool has no row");
        assert_eq!(row.role, Role::Admin);
        assert!(row.ceremony);
    }

    /// Reading a held call's preview is part of deciding it, so it takes the
    /// approver role; it changes nothing, so it takes no ceremony.
    #[test]
    fn reading_a_held_calls_preview_is_an_approver_read_with_no_ceremony() {
        let row = gate_of("policy_approval_preview").expect("policy_approval_preview has no row");
        assert_eq!(row.role, Role::Approver);
        assert!(!row.ceremony);
    }

    #[test]
    fn an_unknown_command_is_treated_as_a_ceremony_command() {
        assert!(requires_ceremony("definitely_not_a_command"));
        assert!(requires_ceremony(""));
    }

    #[test]
    fn a_read_needs_no_ceremony_and_a_sensitive_write_does() {
        assert!(!requires_ceremony("money_units"));
        assert!(requires_ceremony("money_kill_run"));
        // An approver's grant is a ceremony too, though its role is lower.
        assert_eq!(required_role("policy_decide_approval"), Role::Approver);
        assert!(requires_ceremony("policy_decide_approval"));
    }

    /// The second hand-kept list this file and `main.rs` used to share is
    /// gone: the ceremony set is read from the command table, and `main.rs`
    /// names no list of its own.
    #[test]
    fn the_ceremony_list_is_written_once_in_the_command_table() {
        const MAIN: &str = include_str!("main.rs");
        assert!(
            !MAIN.contains("SENSITIVE_COMMANDS"),
            "main.rs still keeps a ceremony list of its own"
        );
    }

    #[test]
    fn role_ordering_expresses_privilege() {
        assert!(Role::Admin >= Role::Approver);
        assert!(Role::Approver >= Role::Viewer);
        assert!(Role::Viewer < Role::Admin);
    }
}
