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

/// Commands that grant or deny a Wardryx approval: the human-in-the-loop
/// sanctioned path, `approver` and up.
const APPROVER_COMMANDS: &[&str] = &["policy_decide_approval"];

/// Commands that change enforcement state, sign an action, delete data, drive
/// a remote box, spawn real work, or pair a device: `admin` only.
const ADMIN_COMMANDS: &[&str] = &[
    // Money plane: the signed Cloud mutations.
    "money_kill_run",
    "money_set_budget",
    "money_ack_incident",
    // Lifecycle: freezing an agent, or stopping a whole unit or user, writes
    // deny-all wardryx policies and records the block. A real enforcement
    // change on the fleet - admin only. Reading the blocks back
    // (`lifecycle_blocks`) is an ordinary viewer read, listed below.
    "agent_block",
    "unit_block",
    "user_block",
    // Identity: a rescan spawns a detector run.
    "identity_rescan",
    // Drills fire crafted traffic at the operator's own gateway.
    "drills_run",
    // Evidence assembles AND signs a pack.
    "evidence_build",
    // Memory forget deletes recorded memory.
    "memory_forget",
    // Onboard generates a minted secret and stages a passport write.
    "onboard_generate",
    "onboard_write_passport",
    // Admission's baseline leg fires real gateway calls under the newcomer's
    // own key and spends real provider money (docs/ADMISSION.md) - the same
    // "admin only" posture as Drills' `drills_run` just above.
    "admission_baseline",
    // Copilot: logging a proposal as approved leads to a signed action.
    "copilot_log_proposal_approved",
    // Remote: everything that touches or reaches the client-hosted box.
    "remote_set_environment",
    "remote_wg_connect",
    "remote_wg_disconnect",
    "remote_ssh_read_file",
    "remote_ssh_tail_start",
    "remote_ssh_tail_stop",
    // Mutates THIS box's own local WireGuard server (adds a peer) rather
    // than reaching a client-hosted one, but it is still a real mutation
    // that hands out tunnel access to this box, the same posture as every
    // other admin-only Remote command above.
    "remote_operator_wg_config",
    // Revoking cuts a device's access to the control plane. Listing the peers
    // is deliberately NOT here: seeing who holds access is a read, and a
    // reviewer should be able to look without being able to change anything.
    "remote_operator_wg_revoke",
    // Hetzner inventory takes a live cloud API token as a plain command
    // argument, so this is the one "read" on the console that carries a
    // secret in its request body. A viewer could hand the console any token
    // and have it call out to Hetzner holding it. That is a different act
    // from the sibling `remote_cloud_list`, which spawns the operator's own
    // already-authenticated CLI on their own machine and never sees a
    // credential at all, and which stays a viewer read for exactly that
    // reason. Classified by what crosses the boundary, not by whether the
    // upstream call happens to be a GET.
    "remote_hetzner_list",
];

/// The minimum role that may run `command`. Everything not named in the
/// approver/admin sets is a read (`viewer`), EXCEPT an entirely unknown name,
/// which requires `admin` (fail closed).
pub fn required_role(command: &str) -> Role {
    if ADMIN_COMMANDS.contains(&command) {
        Role::Admin
    } else if APPROVER_COMMANDS.contains(&command) {
        Role::Approver
    } else if VIEWER_COMMANDS.contains(&command) {
        Role::Viewer
    } else {
        // Unknown / not-yet-classified: deny to all but admin. Dispatch also
        // 404s a truly unknown name, but the gate must not be the layer that
        // fails open.
        Role::Admin
    }
}

/// Reads and non-mutating inspections: `viewer` and up. Named explicitly (not
/// a fallthrough) so the completeness test can prove every dispatch command is
/// placed on purpose and an unknown name still fails closed in
/// [`required_role`].
const VIEWER_COMMANDS: &[&str] = &[
    "admission_check",
    "admission_status",
    "agent_events",
    "agent_graph",
    "agent_slice",
    "bus_status",
    "copilot_ask",
    "copilot_explain",
    "copilot_status",
    "credentials_keys",
    "credentials_status",
    "crypto_scan_cbom",
    "crypto_scan_evidence",
    "crypto_scan_ncsc",
    "crypto_status",
    "crypto_verify_evidence",
    "drills_status",
    "evidence_status",
    "identity_list_alerts",
    "identity_list_identities",
    "identity_list_remediations",
    "identity_status",
    "lifecycle_blocks",
    "memory_recall",
    "memory_stats",
    "memory_status",
    "memory_why",
    "money_incidents",
    "money_overview",
    "money_owners",
    "money_runs",
    "money_savings",
    "egress_recent",
    "money_status",
    "onboard_status",
    "policy_list_approvals",
    "policy_list_policies",
    "policy_status",
    "quality_list_baselines",
    "quality_list_run_summaries",
    "quality_run_scores",
    "quality_status",
    "policy_enforcement_status",
    "recent_events",
    "remote_cloud_list",
    // Which devices hold tunnel access is a read. Seeing it without being able
    // to change it is exactly what a reviewer needs.
    "remote_operator_wg_peers",
    "remote_ssh_check_reachable",
    "remote_status",
    "routines_history",
    "routines_status",
    "run_events",
    "stats_counts",
];

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
    fn every_dispatch_command_is_classified_exactly_once() {
        use std::collections::BTreeSet;
        let viewer: BTreeSet<_> = VIEWER_COMMANDS.iter().collect();
        let approver: BTreeSet<_> = APPROVER_COMMANDS.iter().collect();
        let admin: BTreeSet<_> = ADMIN_COMMANDS.iter().collect();

        // No command appears in two sets.
        assert!(viewer.is_disjoint(&approver), "viewer/approver overlap");
        assert!(viewer.is_disjoint(&admin), "viewer/admin overlap");
        assert!(approver.is_disjoint(&admin), "approver/admin overlap");

        // The union is exactly the live dispatch set.
        let classified: BTreeSet<_> = viewer
            .iter()
            .chain(approver.iter())
            .chain(admin.iter())
            .copied()
            .collect();
        let classified: BTreeSet<String> = classified.into_iter().map(|s| s.to_string()).collect();
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

    #[test]
    fn role_ordering_expresses_privilege() {
        assert!(Role::Admin >= Role::Approver);
        assert!(Role::Approver >= Role::Viewer);
        assert!(Role::Viewer < Role::Admin);
    }
}
