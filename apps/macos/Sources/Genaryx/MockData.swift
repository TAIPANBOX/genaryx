import Foundation

/// ~40 mock events across all six emitting bus sources, standing in for a
/// live `IngestService` feed until the UniFFI bridge lands (see the doc
/// comment on `UiEvent`). Types, severities, schema versions, and payload
/// shapes mirror the real `taipan demo` generator (`crates/core/src/demo.rs`)
/// so this reads like a slice of an actual campaign rather than placeholder
/// text. `low` severity does not appear in that generator today; a handful
/// of rows here use it anyway so the Bus Explorer's full severity ladder has
/// at least one example of every color.
enum MockData {
    /// Newest-first (highest `id`, most recent `ts`), matching
    /// `Store::recent_events`'s `ORDER BY id DESC`.
    static let events: [UiEvent] = buildEvents()

    /// One event's fixed inputs, prior to timestamp/id assignment.
    private struct Seed {
        let source: String
        let type: String
        let severity: String
        let agent: String
        let run: String
        let delegated: Bool
        let data: String
    }

    private static func buildEvents() -> [UiEvent] {
        let base = baseInstant()
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]

        let baseId: Int64 = 128_460
        let delegationChain = [
            "user://taipanbox.dev/j.doe",
            "agent://taipanbox.dev/demo/orchestrator",
        ]

        return seeds.enumerated().map { index, seed in
            // A varying, deterministic step back in time per row (never
            // `Date()`), so the list looks identical on every launch.
            let offset = Double(index * 17 + (index % 4) * 5)
            let ts = formatter.string(from: base.addingTimeInterval(-offset))
            let agentId = "agent://taipanbox.dev/demo/\(seed.agent)"
            let onBehalfOf = seed.delegated ? delegationChain : []
            let schema = schemaVersion(for: seed.source)
            let raw = rawLine(
                schema: schema,
                ts: ts,
                source: seed.source,
                type: seed.type,
                agentId: agentId,
                severity: seed.severity,
                runId: seed.run,
                onBehalfOf: onBehalfOf,
                data: seed.data
            )
            return UiEvent(
                id: baseId - Int64(index),
                ts: ts,
                source: seed.source,
                type_: seed.type,
                agentId: agentId,
                runId: seed.run,
                severity: seed.severity,
                schema: schema,
                onBehalfOf: onBehalfOf,
                raw: raw
            )
        }
    }

    /// Fixed base instant. Deliberately not `Date()`: the mock feed should
    /// render identically on every launch and every screenshot.
    private static func baseInstant() -> Date {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC") ?? TimeZone(secondsFromGMT: 0) ?? .current

        var components = DateComponents()
        components.year = 2026
        components.month = 7
        components.day = 17
        components.hour = 14
        components.minute = 32
        components.second = 7

        return calendar.date(from: components) ?? Date(timeIntervalSince1970: 1_784_471_527)
    }

    /// tokenfuse/engram/qryx emit under v0.1, wardryx/verdryx/mockryx under
    /// v0.2, exactly matching the `SchemaVersion` each carries in `demo.rs`.
    private static func schemaVersion(for source: String) -> String {
        switch source {
        case "wardryx", "verdryx", "mockryx":
            return "taipanbox.dev/agent-event/v0.2"
        default:
            return "taipanbox.dev/agent-event/v0.1"
        }
    }

    /// Renders one NDJSON line by hand, field order matching `demo.rs`'s
    /// `render_line`: schema, ts, source, type, agent_id, severity, run_id,
    /// an optional on_behalf_of, then data. Fine to hand-format here since
    /// this text exists only to populate `UiEvent.raw` for the mock
    /// disclosure view; the real ingest path always builds these through
    /// `serde_json`.
    private static func rawLine(
        schema: String,
        ts: String,
        source: String,
        type: String,
        agentId: String,
        severity: String,
        runId: String,
        onBehalfOf: [String],
        data: String
    ) -> String {
        var fields = [
            field("schema", schema),
            field("ts", ts),
            field("source", source),
            field("type", type),
            field("agent_id", agentId),
            field("severity", severity),
            field("run_id", runId),
        ]
        if !onBehalfOf.isEmpty {
            let joined = onBehalfOf.map { "\"\($0)\"" }.joined(separator: ",")
            fields.append("\"on_behalf_of\":[\(joined)]")
        }
        fields.append("\"data\":\(data)")
        return "{" + fields.joined(separator: ",") + "}"
    }

    private static func field(_ key: String, _ value: String) -> String {
        "\"\(key)\":\"\(value)\""
    }

    // Newest first. Grouped loosely by run, the same way `demo.rs` emits a
    // handful of related calls per run (e.g. a policy deny followed by an
    // approval request and grant), so adjacent rows often tell one story.
    private static let seeds: [Seed] = [
        Seed(
            source: "wardryx", type: "approval_granted", severity: "info", agent: "deploy-bot",
            run: "demo-run-064", delegated: false,
            data: #"{"policy":"prod-deploy-requires-approval","granted_by":"user://taipanbox.dev/j.doe"}"#
        ),
        Seed(
            source: "wardryx", type: "approval_requested", severity: "medium", agent: "deploy-bot",
            run: "demo-run-064", delegated: false,
            data: #"{"policy":"prod-deploy-requires-approval","reason":"awaiting operator approval"}"#
        ),
        Seed(
            source: "wardryx", type: "policy_deny", severity: "high", agent: "deploy-bot",
            run: "demo-run-064", delegated: false,
            data: #"{"policy":"prod-deploy-requires-approval","reason":"no approval on file for deploy:prod scope"}"#
        ),
        Seed(
            source: "tokenfuse", type: "spend_spike", severity: "high", agent: "fraud-bot",
            run: "demo-run-063", delegated: true,
            data: #"{"window_s":60,"spend_usd":9.87,"baseline_usd":1.42,"multiplier":6.9}"#
        ),
        Seed(
            source: "qryx", type: "evidence_signed", severity: "info", agent: "audit-bot",
            run: "demo-run-063", delegated: false,
            data: #"{"evidence_id":"ev-55063","algorithm":"ml-dsa-65","subject":"agent://taipanbox.dev/demo/audit-bot"}"#
        ),
        Seed(
            source: "qryx", type: "crypto_finding", severity: "medium", agent: "audit-bot",
            run: "demo-run-062", delegated: false,
            data: #"{"algorithm":"rsa-2048","risk":"quantum-vulnerable","recommended":"ml-dsa-65"}"#
        ),
        Seed(
            source: "verdryx", type: "quality_drift", severity: "high", agent: "triage-bot",
            run: "demo-run-061", delegated: false,
            data: #"{"eval_suite":"fraud-triage-qa","baseline_score":0.96,"current_score":0.88,"delta":-0.08}"#
        ),
        Seed(
            source: "verdryx", type: "quality_score", severity: "info", agent: "triage-bot",
            run: "demo-run-061", delegated: false,
            data: #"{"eval_suite":"fraud-triage-qa","current_score":0.88}"#
        ),
        Seed(
            source: "mockryx", type: "blast_radius_measured", severity: "medium", agent: "sentinel",
            run: "demo-run-060", delegated: true,
            data: #"{"scenario":"policy-bypass-drill","blast_radius_score":0.52,"affected_resources":14}"#
        ),
        Seed(
            source: "mockryx", type: "sim_finding", severity: "medium", agent: "sentinel",
            run: "demo-run-060", delegated: true,
            data: #"{"scenario":"policy-bypass-drill","finding":"gap_found"}"#
        ),
        Seed(
            source: "mockryx", type: "sim_run", severity: "info", agent: "sentinel",
            run: "demo-run-060", delegated: true,
            data: #"{"scenario":"policy-bypass-drill","status":"completed"}"#
        ),
        Seed(
            source: "engram", type: "contradiction_found", severity: "medium", agent: "kyc-bot",
            run: "demo-run-059", delegated: false,
            data: #"{"memory_id":"mem-3059","conflicting_memory_id":"mem-2059","topic":"kyc_verification_steps"}"#
        ),
        Seed(
            source: "engram", type: "memory_written", severity: "info", agent: "kyc-bot",
            run: "demo-run-059", delegated: false,
            data: #"{"memory_id":"mem-3059","topic":"kyc_verification_steps"}"#
        ),
        Seed(
            source: "tokenfuse", type: "fanout_explosion", severity: "high", agent: "orchestrator",
            run: "demo-run-058", delegated: false,
            data: #"{"child_agents":11,"depth":4,"budget_usd":3.65}"#
        ),
        Seed(
            source: "wardryx", type: "policy_allow", severity: "info", agent: "billing-bot",
            run: "demo-run-057", delegated: true,
            data: #"{"policy":"default-allow","reason":"within policy"}"#
        ),
        Seed(
            source: "engram", type: "memory_written", severity: "low", agent: "billing-bot",
            run: "demo-run-057", delegated: true,
            data: #"{"memory_id":"mem-3057","topic":"chargeback_procedure"}"#
        ),
        Seed(
            source: "tokenfuse", type: "breaker_tripped", severity: "critical", agent: "collections-bot",
            run: "demo-run-056", delegated: false,
            data: #"{"budget_usd":0.0014,"spent_usd":0.0027,"reason":"budget_exceeded","policy_id":"default"}"#
        ),
        Seed(
            source: "wardryx", type: "policy_allow", severity: "info", agent: "collections-bot",
            run: "demo-run-056", delegated: false,
            data: #"{"policy":"default-allow","reason":"within policy"}"#
        ),
        Seed(
            source: "engram", type: "memory_written", severity: "info", agent: "collections-bot",
            run: "demo-run-056", delegated: false,
            data: #"{"memory_id":"mem-8056","topic":"incident_runbook_step"}"#
        ),
        Seed(
            source: "qryx", type: "evidence_signed", severity: "info", agent: "ci-fixer",
            run: "demo-run-055", delegated: false,
            data: #"{"evidence_id":"ev-55055","algorithm":"ml-dsa-65","subject":"agent://taipanbox.dev/demo/ci-fixer"}"#
        ),
        Seed(
            source: "qryx", type: "crypto_finding", severity: "medium", agent: "ci-fixer",
            run: "demo-run-055", delegated: false,
            data: #"{"algorithm":"rsa-2048","risk":"quantum-vulnerable","recommended":"ml-dsa-65"}"#
        ),
        Seed(
            source: "mockryx", type: "sim_run", severity: "low", agent: "reconciler",
            run: "demo-run-054", delegated: false,
            data: #"{"scenario":"credential-leak-drill","status":"completed"}"#
        ),
        Seed(
            source: "verdryx", type: "quality_score", severity: "info", agent: "reconciler",
            run: "demo-run-054", delegated: false,
            data: #"{"eval_suite":"sla-compliance-qa","current_score":0.95}"#
        ),
        Seed(
            source: "tokenfuse", type: "sustained_loop", severity: "high", agent: "support-bot",
            run: "demo-run-053", delegated: true,
            data: #"{"calls":112,"window_s":120,"pattern":"repeated_tool_call"}"#
        ),
        Seed(
            source: "engram", type: "memory_written", severity: "info", agent: "support-bot",
            run: "demo-run-053", delegated: true,
            data: #"{"memory_id":"mem-3053","topic":"sla_response_times"}"#
        ),
        Seed(
            source: "wardryx", type: "policy_allow", severity: "info", agent: "router",
            run: "demo-run-052", delegated: false,
            data: #"{"policy":"default-allow","reason":"within policy"}"#
        ),
        Seed(
            source: "mockryx", type: "sim_finding", severity: "medium", agent: "scheduler",
            run: "demo-run-051", delegated: false,
            data: #"{"scenario":"runaway-agent-drill","finding":"guardrail_held"}"#
        ),
        Seed(
            source: "mockryx", type: "sim_run", severity: "info", agent: "scheduler",
            run: "demo-run-051", delegated: false,
            data: #"{"scenario":"runaway-agent-drill","status":"completed"}"#
        ),
        Seed(
            source: "verdryx", type: "quality_drift", severity: "high", agent: "analyzer",
            run: "demo-run-050", delegated: false,
            data: #"{"eval_suite":"aml-screening-qa","baseline_score":0.98,"current_score":0.91,"delta":-0.07}"#
        ),
        Seed(
            source: "verdryx", type: "quality_score", severity: "info", agent: "analyzer",
            run: "demo-run-050", delegated: false,
            data: #"{"eval_suite":"aml-screening-qa","current_score":0.91}"#
        ),
        Seed(
            source: "qryx", type: "crypto_finding", severity: "medium", agent: "verifier",
            run: "demo-run-049", delegated: true,
            data: #"{"algorithm":"rsa-2048","risk":"quantum-vulnerable","recommended":"ml-dsa-65"}"#
        ),
        Seed(
            source: "tokenfuse", type: "budget_exhausted", severity: "critical", agent: "onboarding-bot",
            run: "demo-run-048", delegated: false,
            data: #"{"budget_usd":0.0009,"spent_usd":0.0021,"reason":"budget_exceeded","policy_id":"default"}"#
        ),
        Seed(
            source: "wardryx", type: "policy_allow", severity: "info", agent: "onboarding-bot",
            run: "demo-run-048", delegated: false,
            data: #"{"policy":"default-allow","reason":"within policy"}"#
        ),
        Seed(
            source: "engram", type: "memory_written", severity: "info", agent: "onboarding-bot",
            run: "demo-run-048", delegated: false,
            data: #"{"memory_id":"mem-8048","topic":"aml_screening_rules"}"#
        ),
        Seed(
            source: "engram", type: "contradiction_found", severity: "medium", agent: "escalation-bot",
            run: "demo-run-047", delegated: false,
            data: #"{"memory_id":"mem-3047","conflicting_memory_id":"mem-2047","topic":"fraud_hold_criteria"}"#
        ),
        Seed(
            source: "engram", type: "memory_written", severity: "info", agent: "escalation-bot",
            run: "demo-run-047", delegated: false,
            data: #"{"memory_id":"mem-3047","topic":"fraud_hold_criteria"}"#
        ),
        Seed(
            source: "mockryx", type: "blast_radius_measured", severity: "medium", agent: "ci-orchestrator",
            run: "demo-run-046", delegated: false,
            data: #"{"scenario":"prod-deploy-rehearsal","blast_radius_score":0.38,"affected_resources":7}"#
        ),
        Seed(
            source: "tokenfuse", type: "spend_spike", severity: "high", agent: "cache-warmer",
            run: "demo-run-045", delegated: false,
            data: #"{"window_s":60,"spend_usd":5.21,"baseline_usd":0.95,"multiplier":5.5}"#
        ),
        Seed(
            source: "qryx", type: "evidence_signed", severity: "info", agent: "planner",
            run: "demo-run-044", delegated: true,
            data: #"{"evidence_id":"ev-55044","algorithm":"ml-dsa-65","subject":"agent://taipanbox.dev/demo/planner"}"#
        ),
        Seed(
            source: "qryx", type: "crypto_finding", severity: "low", agent: "planner",
            run: "demo-run-044", delegated: true,
            data: #"{"algorithm":"rsa-2048","risk":"quantum-vulnerable","recommended":"ml-dsa-65"}"#
        ),
    ]
}
