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
        cloud: CloudModel, policy: PolicyModel, identity: IdentityModel, fleet: FleetModel, now: Date = Date()
    ) -> [PostureFinding] {
        [
            devkeyFinding(cloud: cloud, policy: policy),
            governanceFailOpenFinding(policy: policy),
            schemaMixFinding(fleet: fleet),
            busStaleFinding(fleet: fleet, now: now),
            idryxExposedFinding(identity: identity),
            attestationCoverageFinding(identity: identity),
            identitySnapshotAgeFinding(identity: identity, now: now),
            detectorFeedFreshnessFinding(identity: identity, now: now),
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

    // MARK: - PHASE3 W4: the identity-plane zonds ("Posture full")
    //
    // Four new zonds, computed the exact same way as the four v0 zonds
    // above: a pure function over state a model already holds live
    // (`IdentityModel`'s `connection`/`identities`/`alerts`/`loadedAt`, fed
    // by `IdryxHandle` - see `IdentityModel.swift`), never a new FFI call or
    // handle of this model's own. Each returns `nil` (no finding) whenever
    // its signal is unavailable or nothing is wrong, matching every zond
    // above's "no finding when things are fine" convention - never a
    // fabricated row.
    //
    // PHASE3.md position 6 also names a fifth: "keyless-admin Wardryx (07
    // §4.3)". `itrat-console/07 §4.3`'s own grounded note is "без ключів =
    // devkey/admin; role за замовчуванням admin" (no keys -> devkey/admin;
    // role defaults to admin) - i.e. a Wardryx bearer that never went
    // through `taipan up`'s own key-minting lands on an implicit admin
    // role. That is EXACTLY what `devkeyFinding` above already flags for
    // Wardryx, via `isEnvFallback(source)` on `policy.connection`
    // (`WardryxEnvSource.envFallback` - a bare `WARDRYX_ADMIN_KEY` used
    // directly rather than a taipan-minted per-device key; see that
    // finding's own doc comment, "an `orgDomain` equality check alone would
    // never fire on the Wardryx side"). A second, differently-worded zond
    // over the SAME signal would be noise dressed up as a new finding, so
    // per this wave's own instruction ("omit a zond rather than fabricate
    // one whose signal is unavailable") it is intentionally not duplicated
    // here. What this wave genuinely CANNOT add: a true server-side "this
    // Wardryx has zero WARDRYX_KEYS configured at all" probe -
    // `WardryxHandle` deliberately never exposes the raw bearer or any
    // keys-configured signal across FFI (`crates/ffi/src/wardryx/mod.rs`'s
    // own "the admin bearer must never cross into Swift as a plain value
    // beyond what construction already consumes it for" rule), and this
    // wave's guardrails forbid touching that module to add one. That
    // sharper signal stays a real, honestly-acknowledged residual gap
    // rather than an approximated finding.

    // MARK: - 5. idryx_exposed

    /// PHASE3.md position 6, zond `idryx_exposed`: "the discovered idryx
    /// URL is non-loopback... a real posture signal" (docs/PHASE3.md's own
    /// grounded contract: idryx's `--addr` defaults to `:8080` on ALL
    /// interfaces, not loopback, and `SECURITY.md` states outright that
    /// `serve` "has no authentication; run it behind your own auth/network
    /// controls" - `taipan up` remaps it to `127.0.0.1:8081`, but a
    /// hand-started idryx, or a broken remap, can still bind wide open with
    /// zero auth of any kind).
    private static func idryxExposedFinding(identity: IdentityModel) -> PostureFinding? {
        guard case .ready(_, let idryxUrl) = identity.connection, !isLoopbackUrl(idryxUrl) else { return nil }
        return PostureFinding(
            id: "idryx-exposed",
            severity: "high",
            title: "Idryx bound off loopback",
            whyItMatters:
                "The discovered idryx URL (\(idryxUrl)) is not loopback. idryx serve has no authentication of any kind, so a non-loopback bind exposes every identity, permission, and alert to the network unauthenticated.",
            howToFix: "Bind idryx to loopback (idryx serve --addr 127.0.0.1:8081), or put it behind a tunnel / your own auth proxy."
        )
    }

    /// `true` only for a URL whose host is confirmed loopback
    /// (`127.0.0.1`/`localhost`/`::1`); an unparseable or hostless URL is
    /// treated as NOT confirmed loopback (fails closed toward flagging the
    /// finding, never toward silently assuming safety) rather than crashing
    /// or guessing.
    private static func isLoopbackUrl(_ urlString: String) -> Bool {
        guard let host = URL(string: urlString)?.host else { return false }
        let lowered = host.lowercased()
        return lowered == "localhost" || lowered == "127.0.0.1" || lowered == "::1"
    }

    // MARK: - 6. attestation coverage

    /// PHASE3.md position 6, zond "attestation coverage": of PRIVILEGED
    /// identities (`IdentityRecord.privileged`), how many carry an
    /// `attestation_missing`/`bom_incomplete` alert
    /// (`AlertRecord.detector`, joined on `AlertRecord.identity ==
    /// IdentityRecord.id`) - attestation is never a clean identity field
    /// (docs/PHASE3.md: "not a structured field on the identity"), so a
    /// coverage gap is only visible through these two detectors, exactly
    /// how the Identity panel's own Attestation section already surfaces it
    /// per-identity (PHASE3.md W2). Severity scales with how incomplete the
    /// coverage is: "medium" once at least half of the privileged
    /// identities are missing attestation (a majority gap), "info" for a
    /// smaller, partial one - omitted entirely when there are no privileged
    /// identities to begin with, or when every one of them is attested.
    private static func attestationCoverageFinding(identity: IdentityModel) -> PostureFinding? {
        guard identity.connection.isReady else { return nil }
        let privileged = identity.identities.filter(\.privileged)
        guard !privileged.isEmpty else { return nil }

        let unattestedIds = Set(
            identity.alerts
                .filter { $0.detector == "attestation_missing" || $0.detector == "bom_incomplete" }
                .map(\.identity)
        )
        let missingCount = privileged.filter { unattestedIds.contains($0.id) }.count
        guard missingCount > 0 else { return nil }

        let fraction = Double(missingCount) / Double(privileged.count)
        return PostureFinding(
            id: "attestation-coverage",
            severity: fraction >= 0.5 ? "medium" : "info",
            title: "Attestation gap on privileged identities",
            whyItMatters:
                "\(missingCount) of \(privileged.count) privileged identities carry an attestation_missing or bom_incomplete alert - unattested privileged identities are exactly the excessive_agency / tainted_agent blast radius those detectors exist to catch.",
            howToFix: "Attest them (oidc / spiffe-svid / enclave-key / mtls-cert), then Rescan on the Identity panel to confirm the alert clears."
        )
    }

    // MARK: - 7. identity snapshot age

    /// PHASE3.md position 6, zond "identity snapshot age": idryx `serve` is
    /// LOAD-ONCE (docs/PHASE3.md's own grounded contract: "no file-watch, no
    /// SIGHUP, no reload endpoint, no polling, no TTL... polling `/api/*`
    /// returns byte-identical data for the process lifetime"), so
    /// `identity.loadedAt` (this console's own last successful pull, labeled
    /// "as of load" - `IdentityModel`'s own doc comment) is the only honest
    /// freshness signal there is. Always shown while ready (an "info" - the
    /// point is transparency about staleness, not a defect to clear),
    /// mirroring `schemaMixFinding`'s own "informational, not a defect"
    /// precedent for a finding that is not itself a problem.
    private static func identitySnapshotAgeFinding(identity: IdentityModel, now: Date) -> PostureFinding? {
        guard identity.connection.isReady, let loadedAt = identity.loadedAt else { return nil }
        let age = now.timeIntervalSince(loadedAt)
        return PostureFinding(
            id: "identity-snapshot-age",
            severity: "info",
            title: "Identity snapshot is \(formatAge(age)) old",
            whyItMatters:
                "idryx serve loads its data once at startup and never reloads (no file-watch, no SIGHUP, no TTL) - every identity/alert/remediation shown is exactly as of that load, never live.",
            howToFix: "Rescan (Identity panel) to recompute alerts over the current bus, or restart idryx to reload identities/permissions themselves."
        )
    }

    // MARK: - 8. detector-feed freshness

    /// A detector feed that has produced nothing in over this long is worth
    /// a nudge to Rescan, not just an FYI - see
    /// `detectorFeedFreshnessFinding`'s own doc comment.
    private static let staleDetectorFeedThreshold: TimeInterval = 24 * 60 * 60

    /// PHASE3.md position 6, zond "detector-feed freshness": how long since
    /// the most recent alert idryx produced (`AlertRecord.time`), vs now.
    /// Distinct from `identitySnapshotAgeFinding` above: that one measures
    /// when THIS CONSOLE last pulled the snapshot; this one measures how
    /// old the detectors' own most recent finding is, which stays fixed at
    /// whatever idryx computed at load/Rescan time regardless of how often
    /// the console re-pulls the same byte-identical snapshot. Omitted (no
    /// fabricated signal) when there are no alerts at all - "freshness" has
    /// no meaning over an empty feed.
    private static func detectorFeedFreshnessFinding(identity: IdentityModel, now: Date) -> PostureFinding? {
        guard identity.connection.isReady, !identity.alerts.isEmpty else { return nil }
        let mostRecent = identity.alerts.compactMap { parseTimestamp($0.time) }.max()
        guard let mostRecent else { return nil }
        let age = now.timeIntervalSince(mostRecent)
        return PostureFinding(
            id: "detector-feed-freshness",
            severity: age > staleDetectorFeedThreshold ? "medium" : "info",
            title: "Most recent detector alert is \(formatAge(age)) old",
            whyItMatters:
                "The freshest of the \(identity.alerts.count) loaded alerts fired \(formatAge(age)) ago. idryx never recomputes on its own (see the snapshot-age zond above), so a stale feed usually means the loaded findings predate the stack's current state.",
            howToFix: "Rescan (Identity panel) to recompute the 21 detectors over the current bus."
        )
    }

    /// Compact "4m12s" / "1h03m" / "37s" age formatting for the two zonds
    /// above - deliberately not `MoneyFormat.timestamp` (that formats an
    /// absolute clock reading, not an elapsed duration).
    private static func formatAge(_ interval: TimeInterval) -> String {
        let totalSeconds = max(0, Int(interval))
        let hours = totalSeconds / 3_600
        let minutes = (totalSeconds % 3_600) / 60
        let seconds = totalSeconds % 60
        if hours > 0 { return "\(hours)h\(String(format: "%02d", minutes))m" }
        if minutes > 0 { return "\(minutes)m\(String(format: "%02d", seconds))s" }
        return "\(seconds)s"
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
