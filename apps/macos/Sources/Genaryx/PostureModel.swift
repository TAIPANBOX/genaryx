import Foundation
import GenaryxCoreFFI

/// One Posture-lite finding - docs/PHASE2.md Wave 3: "each finding =
/// {severity, title, why it matters, how to fix (a concrete command / env
/// var)}". `severity` is a plain lowercase string ("high" / "medium" /
/// "info") straight into `Theme.severityColor`/`severityLabel` and
/// `SeverityPill`, the exact same severity ladder every other panel already
/// renders (mirrors `Incident.severity`'s own shape from the generated FFI
/// bindings, which `MoneyView`'s `IncidentsList` already feeds to
/// `SeverityPill` the same way).
struct PostureFinding: Identifiable, Equatable {
    let id: String
    let severity: String
    let title: String
    let whyItMatters: String
    let howToFix: String
}

/// Computes the 4 v0 Posture-lite zonds (PHASE2.md Wave 3) purely from state
/// the shell already holds live - `CloudModel`/`PolicyModel`'s resolved
/// connections, `PolicyModel.policies`, and the `FleetModel` bus feed -
/// never a new read of its own (no handle, no FFI call, no network),
/// matching PHASE2.md: "computed from already-observable signals". A plain
/// enum namespace rather than an `@Observable` class: there is no state to
/// own here, only a pure function of the three models' current values, so
/// `PostureView` calls it directly from its `body` and SwiftUI's normal
/// `@Observable` tracking re-renders it exactly when one of those inputs
/// changes - see `PostureView.swift`. `@MainActor` because `CloudModel`,
/// `PolicyModel`, and `FleetModel` are all themselves `@MainActor`-isolated
/// classes (each reads/mutates its state only from the main actor - see
/// each model's own doc), so reading `cloud.connection`, `policy.policies`,
/// `fleet.events`, etc. below requires the same isolation.
@MainActor
enum PostureModel {
    /// PHASE2.md zond 4: "no events observed recently" - "recently" pinned
    /// at the ~60s PHASE2.md itself names ("Wave 3 - actionable
    /// notifications + Posture-lite": "no bus event in ~60s").
    static let staleThreshold: TimeInterval = 60

    static func findings(
        cloud: CloudModel, policy: PolicyModel, fleet: FleetModel, now: Date = Date()
    ) -> [PostureFinding] {
        [
            devkeyFinding(cloud: cloud, policy: policy),
            governanceFailOpenFinding(policy: policy),
            schemaMixFinding(fleet: fleet),
            busStaleFinding(fleet: fleet, now: now),
        ]
        .compactMap { $0 }
    }

    // MARK: - 1. devkey in use

    /// PHASE2.md: "the environment authenticates via a devkey /
    /// ALLOW_DEVKEY fallback (org resolved to `default`, or the bearer is
    /// literally `devkey`)". Checked against BOTH shells' resolved
    /// connections - `CloudModel`'s (server-confirmed `org_domain`, from the
    /// real pairing response - `orgDomain == "default"` is the literal
    /// devkey tell there) and `PolicyModel`'s (no pairing at all, so
    /// `WardryxEnvSource.envFallback` - a `WARDRYX_ADMIN_KEY` used directly
    /// rather than a `taipan up`-minted per-device key - is the meaningful
    /// signal; see `crates/ffi/src/wardryx/mod.rs`'s own module doc, "No
    /// pairing means no server-confirmed org", for why an `orgDomain`
    /// equality check alone would never fire on the Wardryx side).
    private static func devkeyFinding(cloud: CloudModel, policy: PolicyModel) -> PostureFinding? {
        var flagged = false
        if case .ready(let source, _, let orgDomain) = cloud.connection {
            flagged = flagged || isEnvFallback(source) || orgDomain == "default"
        }
        if case .ready(let source, _, let orgDomain) = policy.connection {
            flagged = flagged || isEnvFallback(source) || orgDomain == "default"
        }
        guard flagged else { return nil }
        return PostureFinding(
            id: "devkey-in-use",
            severity: "high",
            title: "Devkey in use",
            whyItMatters:
                "This environment authenticates with a devkey / ALLOW_DEVKEY fallback: org resolved to default, or the bearer is literally devkey.",
            howToFix: "Mint real keys: taipan up mints them, or set real TOKENFUSE_CLOUD_KEYS / WARDRYX_KEYS."
        )
    }

    private static func isEnvFallback(_ source: EnvSource) -> Bool {
        if case .envFallback = source { return true }
        return false
    }

    private static func isEnvFallback(_ source: WardryxEnvSource) -> Bool {
        if case .envFallback = source { return true }
        return false
    }

    // MARK: - 2. governance fail-open

    /// PHASE2.md: "wardryx is reachable but `list_policies()` is empty, so
    /// every action is allowed."
    private static func governanceFailOpenFinding(policy: PolicyModel) -> PostureFinding? {
        guard policy.connection.isReady, policy.policies.isEmpty else { return nil }
        return PostureFinding(
            id: "governance-fail-open",
            severity: "high",
            title: "Governance fail-open: no policies",
            whyItMatters: "Wardryx is reachable but list_policies() is empty, so every action is allowed.",
            howToFix: "PUT policies, or run taipan up --with wardryx with a seeded -policy."
        )
    }

    // MARK: - 3. schema mix

    /// PHASE2.md: "the bus carries both envelope versions (tokenfuse/qryx
    /// emit v0.1, wardryx/verdryx/mockryx v0.2)."
    private static func schemaMixFinding(fleet: FleetModel) -> PostureFinding? {
        var sawV01 = false
        var sawV02 = false
        // The envelope's `schema` is the FULL identifier the core emits and
        // the ffi passes through unchanged (`crates/core/src/event.rs`'s
        // `SCHEMA_V0_1`/`SCHEMA_V0_2`), never a bare "v0.1"/"v0.2" - matching
        // the full strings is what the Tauri track's posture.ts does too, and
        // a bare-suffix compare here would silently never fire this zond.
        for event in fleet.events {
            if event.schema == "taipanbox.dev/agent-event/v0.1" { sawV01 = true }
            if event.schema == "taipanbox.dev/agent-event/v0.2" { sawV02 = true }
            if sawV01 && sawV02 { break }
        }
        guard sawV01 && sawV02 else { return nil }
        return PostureFinding(
            id: "schema-mix",
            severity: "info",
            title: "Schema mix: v0.1 + v0.2",
            whyItMatters:
                "The bus carries both envelope versions: tokenfuse/qryx emit v0.1, wardryx/verdryx/mockryx emit v0.2.",
            howToFix: "Informational, not a defect. Resolved by the tokenfuse-core v0.2 PR (workstream C)."
        )
    }

    // MARK: - 4. bus stale

    /// PHASE2.md: "no events observed recently, or the events source is
    /// empty." `fleet.events` is newest-first (`FleetModel.swift`'s own
    /// doc), so the first element is the most recent event.
    private static func busStaleFinding(fleet: FleetModel, now: Date) -> PostureFinding? {
        let isStale: Bool
        if let newest = fleet.events.first, let newestDate = parseTimestamp(newest.ts) {
            isStale = now.timeIntervalSince(newestDate) > staleThreshold
        } else {
            // No events at all (or an unparseable timestamp on the newest
            // one) - PHASE2.md's own "or the events source is empty" case.
            isStale = true
        }
        guard isStale else { return nil }
        return PostureFinding(
            id: "bus-stale",
            severity: "medium",
            title: "Bus stale",
            whyItMatters: "No bus events observed in the last minute, or the events source is empty.",
            howToFix: "Check the feeder, or the descriptor's events paths."
        )
    }

    // A dedicated small ISO8601 parsing helper, matching
    // `PolicyComponents.swift`'s own precedent of one small parsing helper
    // per file rather than a shared central one (see that file's
    // `UiEvent.wardryxFields`) - `MoneyFormat.timestamp` only ever returns a
    // formatted display *string*, never the underlying `Date` this
    // threshold comparison needs. `nonisolated(unsafe)`: configured once at
    // first access and only ever read afterward, the same reasoning
    // `MoneyFormat`'s own formatters document.
    private nonisolated(unsafe) static let isoFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private nonisolated(unsafe) static let isoFormatterNoFraction: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    private static func parseTimestamp(_ iso: String) -> Date? {
        isoFormatter.date(from: iso) ?? isoFormatterNoFraction.date(from: iso)
    }
}
