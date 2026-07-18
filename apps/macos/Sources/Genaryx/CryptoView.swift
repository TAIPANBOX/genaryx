import GenaryxCoreFFI
import SwiftUI

/// The Crypto panel: the PQC readiness timeline hero (the three NCSC
/// milestones), a quantum-vulnerable findings table, a CBOM inventory table,
/// and an Evidence section (build + Verify) - all driven by an on-demand
/// scan against an operator-editable target path. Fed entirely by
/// `CryptoModel` (the Qryx reads); unlike `PolicyView`/`QualityView`, this
/// panel has no bus-event filter section (Qryx has no "live" signal of its
/// own to tail - docs/PHASE4.md W1) and no periodic auto-refresh - every
/// number on screen is "as of last scan", at parity with the Tauri shell's
/// own Crypto panel.
@MainActor
struct CryptoView: View {
    let model: CryptoModel

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                CryptoEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .onChange(of: model.evidenceScope) { _, _ in
            Task { await model.refreshEvidence() }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                environmentChip
                scanControls

                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                dashboard
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// The PQC readiness timeline as the hero itself (the three NCSC
    /// milestone cards - already documented as this panel's "hero" before
    /// this conversion), beside findings/CBOM/evidence-score tiles, then the
    /// findings + CBOM tables (primary) and the Evidence section (rail).
    /// Every card `.onDemand` - qryx never auto-refreshes, every number here
    /// is honestly "as of last scan" (design spec's Crypto blueprint,
    /// section 5).
    private var dashboard: some View {
        let findings = model.ncscReport?.discovery2028.quantumVulnerableFindings
        let cbomCount = CbomParser.components(fromJson: model.cbomJson).count
        let scorePct = model.evidenceReport?.summary.scorePct
        let lastScan = LastScanFormat.clock(model.lastScanAt)

        return VStack(spacing: 16) {
            HeroBand {
                ncscHero
            } tiles: {
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 14), GridItem(.flexible(), spacing: 14)], spacing: 14) {
                    KpiTile(
                        label: "quantum-vulnerable", value: Dash.int((findings ?? []).count),
                        sub: findingsSeverityBreakdown(findings ?? []), tone: (findings?.isEmpty ?? true) ? nil : Theme.coral)
                    KpiTile(label: "CBOM components", value: Dash.int(cbomCount), sub: "scan_cbom inventory")
                    KpiTile(
                        label: "evidence score", value: scorePct.map { "\($0)%" } ?? "n/a",
                        sub: model.evidenceScope.label, tone: scorePct.map(scoreTone))
                }
            }

            DashMain {
                DashSection(title: "Quantum-Vulnerable Findings", badge: .onDemand(last: lastScan)) {
                    QuantumVulnerableFindingsSection(findings: findings, isScanning: model.isScanning)
                }
                DashSection(title: "CBOM Inventory", badge: .onDemand(last: lastScan)) {
                    CbomInventorySection(json: model.cbomJson, isScanning: model.isScanning)
                }
            } rail: {
                DashSection(title: "Evidence", badge: .onDemand(last: lastScan)) {
                    EvidenceSection(
                        report: model.evidenceReport,
                        scope: Binding(get: { model.evidenceScope }, set: { model.evidenceScope = $0 }),
                        verifyFilePath: Binding(get: { model.verifyFilePath }, set: { model.verifyFilePath = $0 }),
                        verifyOutcome: model.verifyOutcome,
                        isVerifying: model.isVerifying,
                        onVerify: { await model.verifyEvidence() }
                    )
                }
            }
        }
    }

    @ViewBuilder
    private var ncscHero: some View {
        if let ncscReport = model.ncscReport {
            NcscTimelineHero(report: ncscReport)
        } else {
            VStack(alignment: .leading, spacing: 8) {
                Text("PQC readiness timeline")
                    .font(Theme.mono(10.5, weight: .semibold))
                    .tracking(1.6)
                    .foregroundStyle(Theme.textTertiary)
                Text(model.isScanning ? "scanning..." : "run a scan to see the PQC readiness timeline.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 22)
            .frame(maxWidth: .infinity, alignment: .leading)
            .dashCard()
        }
    }

    /// "findings by severity" for the hero tile's sub-line - the same
    /// named-bucket shape `IdentityView.severityBreakdown` uses, duplicated
    /// locally rather than shared (each view's own small, panel-specific
    /// derivation, matching this codebase's established convention).
    private func findingsSeverityBreakdown(_ findings: [NcscFindingRecord]) -> String {
        let counts = ["critical", "high", "medium", "low"].map { severity in
            (severity, findings.filter { $0.severity.lowercased() == severity }.count)
        }
        let parts = counts.filter { $0.1 > 0 }.map { "\($0.1) \($0.0)" }
        if !parts.isEmpty { return parts.joined(separator: " \u{00B7} ") }
        return findings.isEmpty ? "none found" : "unclassified"
    }

    private func scoreTone(_ pct: Int64) -> Color {
        if pct >= 90 { return Theme.mint }
        if pct >= 60 { return Theme.amber }
        return Theme.coral
    }

    @ViewBuilder
    private var environmentChip: some View {
        // Defensive-only: `body` already gates `content` (and therefore this
        // chip) on `model.connection.isReady` - same convention
        // `IdentityView.environmentChip` documents for its own unreachable
        // non-`.ready` branch.
        if case .ready(let source, let qryxBin) = model.connection {
            HStack(spacing: 6) {
                Circle().fill(Theme.sourceColor("qryx")).frame(width: 6, height: 6)
                Text("\(sourceLabel(source)) \u{00B7} \(qryxBin)")
                    .font(Theme.mono(11, weight: .medium))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.panelElevated))
            .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
        }
    }

    private func sourceLabel(_ source: CryptoEnvSource) -> String {
        switch source {
        case .taipan:
            "taipan \u{00B7} well-known"
        case .explicit:
            "explicit path"
        }
    }

    /// The on-demand scan controls: an editable target path (pre-filled from
    /// `CryptoHandle.defaultScanTarget()`, never enforced - docs/PHASE4.md
    /// W1: "operator can see/set it") and a Scan button. The "as of last
    /// scan" time itself now lives in every section's own `.onDemand` badge
    /// below, rather than a second, separately-worded copy up here.
    private var scanControls: some View {
        HStack(spacing: 8) {
            TextField(
                "path to scan", text: Binding(get: { model.scanTarget }, set: { model.scanTarget = $0 })
            )
            .textFieldStyle(.plain)
            .font(Theme.mono(11.5))
            .foregroundStyle(Theme.textPrimary)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panelElevated))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))

            scanButton
        }
    }

    private var scanButton: some View {
        Button {
            Task { await model.runScan() }
        } label: {
            HStack(spacing: 5) {
                if model.isScanning {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "shield.lefthalf.filled")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(model.isScanning ? "Scanning..." : "Scan")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.violet)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.violet.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.violet.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isScanning)
    }
}

// MARK: - NcscTimelineHero

/// docs/PHASE4.md W1: "PQC readiness timeline HERO = the three NCSC
/// milestones each with verdict (on-track/at-risk/not-started, color-coded) +
/// counts; migrated_count labeled honestly".
@MainActor
private struct NcscTimelineHero: View {
    let report: NcscReportRecord

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("generated \(MoneyFormat.timestamp(report.generatedAt)) \u{00B7} root \(report.root)")
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textTertiary)
                .lineLimit(1)
                .truncationMode(.middle)

            HStack(alignment: .top, spacing: 12) {
                MilestoneCard(
                    year: "2028", title: "Complete Discovery", verdict: report.discovery2028.verdict,
                    stats: [
                        ("inventoried", String(report.discovery2028.totalInventoried)),
                        ("quantum-vulnerable", String(report.discovery2028.quantumVulnerableCount)),
                    ],
                    note: report.discovery2028.migrationPlanExists
                        ? "migration plan on file" : "no migration plan on file")
                MilestoneCard(
                    year: "2031", title: "Highest-Priority Systems", verdict: report.highestPriority2031.verdict,
                    stats: [
                        ("in scope", String(report.highestPriority2031.count)),
                        ("remaining", String(report.highestPriority2031.remainingCount)),
                    ],
                    // docs/PHASE4.md W1 guard: migrated_count is ALWAYS 0 -
                    // labeled as "not tracked", never as real progress.
                    note:
                        "migrated: not tracked (\(report.highestPriority2031.migratedCount)) \u{00B7} qryx keeps no cross-run remediation state"
                )
                MilestoneCard(
                    year: "2035", title: "Full Migration", verdict: report.fullMigration2035.verdict,
                    stats: [("in scope", String(report.fullMigration2035.count))], note: nil)
            }
        }
        .padding(.horizontal, 24)
        .padding(.top, 22)
        .padding(.bottom, 18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .dashCard()
    }

    private struct MilestoneCard: View {
        let year: String
        let title: String
        let verdict: String
        let stats: [(String, String)]
        let note: String?

        var body: some View {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(year)
                            .font(Theme.mono(18, weight: .bold))
                            .foregroundStyle(Theme.textPrimary)
                        Text(title.uppercased())
                            .font(Theme.mono(9, weight: .semibold))
                            .tracking(0.8)
                            .foregroundStyle(Theme.textTertiary)
                    }
                    Spacer(minLength: 4)
                    verdictBadge
                }
                ForEach(Array(stats.enumerated()), id: \.offset) { _, stat in
                    HStack {
                        Text(stat.0)
                            .font(Theme.mono(10.5))
                            .foregroundStyle(Theme.textSecondary)
                        Spacer(minLength: 4)
                        Text(stat.1)
                            .font(Theme.mono(12, weight: .semibold))
                            .monospacedDigit()
                            .foregroundStyle(Theme.textPrimary)
                    }
                }
                if let note {
                    Text(note)
                        .font(Theme.mono(9.5))
                        .foregroundStyle(Theme.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .strokeBorder(NcscVerdictFormat.color(verdict).opacity(0.35), lineWidth: 1)
            )
        }

        private var verdictBadge: some View {
            let color = NcscVerdictFormat.color(verdict)
            return Text(NcscVerdictFormat.label(verdict))
                .font(Theme.mono(10, weight: .semibold))
                .foregroundStyle(color)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(Capsule().fill(color.opacity(0.16)))
                .overlay(Capsule().strokeBorder(color.opacity(0.45), lineWidth: 1))
        }
    }
}

// MARK: - QuantumVulnerableFindingsSection

/// docs/PHASE4.md W1: "quantum-vulnerable findings table (algorithm, type,
/// severity, occurrences, locations, externally-facing/long-lived,
/// planned)". Sourced from the 2028 discovery milestone's own finding list -
/// qryx's complete quantum-vulnerable inventory (the 2031/2035 milestones'
/// own `findings` are narrower subsets of this same pool, filtered by
/// priority criteria - `crates/connectors/src/qryx.rs`'s own doc on
/// `NcscPriority`/`NcscFullMigration`).
@MainActor
private struct QuantumVulnerableFindingsSection: View {
    /// `nil` before the first scan this session (honestly distinct from a
    /// scan that came back clean) - mirrors `CbomInventorySection.json`'s
    /// own nil-vs-empty split just below.
    let findings: [NcscFindingRecord]?
    let isScanning: Bool

    private static let displayLimit = 100

    var body: some View {
        if let findings {
            if findings.isEmpty {
                Text("no quantum-vulnerable findings in the 2028 discovery milestone.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.horizontal, 20)
                    .padding(.vertical, 20)
            } else {
                let shown = Array(findings.prefix(Self.displayLimit))
                VStack(spacing: 0) {
                    header
                    Divider().overlay(Theme.hairlineStrong)
                    ForEach(Array(shown.enumerated()), id: \.offset) { index, finding in
                        if index > 0 { Divider().overlay(Theme.hairline) }
                        FindingRow(finding: finding)
                    }
                }
                .padding(.bottom, 4)
                if findings.count > Self.displayLimit {
                    Text("+\(findings.count - Self.displayLimit) more (showing \(Self.displayLimit))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                        .padding(.horizontal, 20)
                        .padding(.bottom, 8)
                }
            }
        } else {
            Text(isScanning ? "scanning..." : "run a scan to see quantum-vulnerable findings.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.horizontal, 20)
                .padding(.vertical, 20)
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            columnLabel("ALGORITHM").frame(width: 130, alignment: .leading)
            columnLabel("TYPE").frame(width: 110, alignment: .leading)
            columnLabel("SEVERITY").frame(width: 90, alignment: .leading)
            columnLabel("OCC.").frame(width: 46, alignment: .trailing)
            columnLabel("LOCATIONS").frame(maxWidth: .infinity, alignment: .leading)
            columnLabel("FLAGS").frame(width: 160, alignment: .trailing)
        }
        .padding(.horizontal, 20)
        .padding(.top, 6)
        .padding(.vertical, 8)
    }

    private func columnLabel(_ text: String) -> some View {
        Text(text)
            .font(Theme.mono(10, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(Theme.textTertiary)
    }

    private struct FindingRow: View {
        let finding: NcscFindingRecord

        var body: some View {
            HStack(alignment: .top, spacing: 10) {
                Text(finding.algorithm)
                    .font(Theme.mono(11.5, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(width: 130, alignment: .leading)
                Text(finding.assetType)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .frame(width: 110, alignment: .leading)
                SeverityPill(severity: finding.severity)
                    .frame(width: 90, alignment: .leading)
                Text(String(finding.occurrences))
                    .font(Theme.mono(11))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: 46, alignment: .trailing)
                Text(finding.locations.isEmpty ? "-" : finding.locations.joined(separator: ", "))
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .lineLimit(2)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                flags
                    .frame(width: 160, alignment: .trailing)
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 8)
        }

        private var flags: some View {
            HStack(spacing: 4) {
                if finding.externallyFacing {
                    flag("external", tone: Theme.coral)
                }
                if finding.longLivedData {
                    flag("long-lived", tone: Theme.amber)
                }
                if finding.planned {
                    flag("planned", tone: Theme.mint)
                }
            }
        }

        private func flag(_ text: String, tone: Color) -> some View {
            Text(text)
                .font(Theme.mono(9, weight: .semibold))
                .foregroundStyle(tone)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Capsule().fill(tone.opacity(0.14)))
        }
    }
}

// MARK: - CbomInventorySection

/// docs/PHASE4.md W1: "CBOM inventory table from scan_cbom (CycloneDX
/// components[])" - parsed best-effort from `CryptoModel.cbomJson` via
/// `CbomParser` (`CryptoComponents.swift`).
@MainActor
private struct CbomInventorySection: View {
    let json: String?
    let isScanning: Bool

    private static let displayLimit = 150

    private var components: [CbomComponent] {
        CbomParser.components(fromJson: json)
    }

    var body: some View {
        if json == nil {
            Text(isScanning ? "scanning..." : "run a scan to see the CBOM inventory.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.horizontal, 20)
                .padding(.vertical, 20)
        } else if components.isEmpty {
            Text("no components in this CBOM.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.horizontal, 20)
                .padding(.vertical, 20)
        } else {
            VStack(alignment: .leading, spacing: 8) {
                if let specVersion = CbomParser.specVersion(fromJson: json) {
                    Text(
                        "CycloneDX \(specVersion) \u{00B7} \(components.count) component\(components.count == 1 ? "" : "s")"
                    )
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.horizontal, 20)
                }
                let shown = Array(components.prefix(Self.displayLimit))
                VStack(spacing: 0) {
                    header
                    Divider().overlay(Theme.hairlineStrong)
                    ForEach(Array(shown.enumerated()), id: \.offset) { index, component in
                        if index > 0 { Divider().overlay(Theme.hairline) }
                        ComponentRow(component: component)
                    }
                }
                if components.count > Self.displayLimit {
                    Text("+\(components.count - Self.displayLimit) more (showing \(Self.displayLimit))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                        .padding(.horizontal, 20)
                }
            }
            .padding(.top, 6)
            .padding(.bottom, 8)
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            columnLabel("NAME").frame(maxWidth: .infinity, alignment: .leading)
            columnLabel("TYPE").frame(width: 110, alignment: .leading)
            columnLabel("CRYPTO ASSET TYPE").frame(width: 150, alignment: .leading)
            columnLabel("VERSION").frame(width: 90, alignment: .trailing)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(Theme.panelElevated)
    }

    private func columnLabel(_ text: String) -> some View {
        Text(text)
            .font(Theme.mono(10, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(Theme.textTertiary)
    }

    private struct ComponentRow: View {
        let component: CbomComponent

        var body: some View {
            HStack(spacing: 10) {
                Text(component.name)
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text(component.type)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .frame(width: 110, alignment: .leading)
                Text(component.cryptoAssetType ?? "-")
                    .font(Theme.mono(11))
                    .foregroundStyle(component.cryptoAssetType == nil ? Theme.textTertiary : Theme.violet)
                    .lineLimit(1)
                    .frame(width: 150, alignment: .leading)
                Text(component.version ?? "-")
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(width: 90, alignment: .trailing)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 7)
        }
    }
}

// MARK: - EvidenceSection

/// docs/PHASE4.md W1: "evidence = build bundle (scan_evidence, unsigned fine
/// for W1), show summary (score %, by-severity, digest, signature alg if
/// present) + a Verify action showing VerifyOutcome". The scope toggle
/// (repository vs. agent stack) additionally covers `agents_evidence` -
/// wrapped on the FFI handle per the task's connector list even though the
/// panel spec names only `scan_evidence`; both return the identical
/// `EvidenceReportRecord` shape, so this section renders either one without
/// any special-casing.
@MainActor
private struct EvidenceSection: View {
    let report: EvidenceReportRecord?
    @Binding var scope: EvidenceScope
    @Binding var verifyFilePath: String
    let verifyOutcome: VerifyOutcomeRecord?
    let isVerifying: Bool
    let onVerify: () async -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            scopePicker

            if let report {
                summaryTiles(report)
                digestAndSignature(report)
            } else {
                Text("run a scan to build an evidence bundle.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            }

            verifyRow
            if let verifyOutcome {
                verifyOutcomeCard(verifyOutcome)
            }
        }
    }

    private var scopePicker: some View {
        Picker("scope", selection: $scope) {
            ForEach(EvidenceScope.allCases) { scope in
                Text(scope.label).tag(scope)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .frame(width: 220)
    }

    private func summaryTiles(_ report: EvidenceReportRecord) -> some View {
        HStack(spacing: 12) {
            StatTile(
                label: "score", value: "\(report.summary.scorePct)%", tone: scoreTone(report.summary.scorePct))
            StatTile(label: "compliant", value: String(report.summary.compliant), tone: Theme.mint)
            StatTile(label: "non-compliant", value: String(report.summary.nonCompliant), tone: Theme.coral)
            StatTile(label: "issues", value: String(report.summary.issues))
        }
    }

    private func scoreTone(_ pct: Int64) -> Color {
        if pct >= 90 { return Theme.mint }
        if pct >= 60 { return Theme.amber }
        return Theme.coral
    }

    private func digestAndSignature(_ report: EvidenceReportRecord) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            if !report.summary.bySeverity.isEmpty {
                HStack(spacing: 6) {
                    ForEach(report.summary.bySeverity, id: \.key) { entry in
                        severityChip(entry)
                    }
                }
            }
            Text("digest \u{00B7} \(report.digest)")
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textSecondary)
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.middle)
            if let signature = report.signature {
                Text("signed \u{00B7} \(signature.alg) \u{00B7} \(truncatedKey(signature.publicKey))")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.mint)
                    .lineLimit(1)
                    .truncationMode(.middle)
            } else {
                Text("unsigned (fine for W1 - a later Evidence Center build adds --sign-key)")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
            }
        }
    }

    private func severityChip(_ entry: CountEntry) -> some View {
        let color = Theme.severityColor(entry.key)
        return Text("\(entry.key) \(entry.count)")
            .font(Theme.mono(10, weight: .semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(Capsule().fill(color.opacity(0.14)))
            .overlay(Capsule().strokeBorder(color.opacity(0.4), lineWidth: 1))
    }

    private func truncatedKey(_ key: String) -> String {
        key.count > 20 ? "\(key.prefix(10))...\(key.suffix(6))" : key
    }

    private var verifyRow: some View {
        HStack(spacing: 8) {
            TextField("path to an evidence report file to verify", text: $verifyFilePath)
                .textFieldStyle(.plain)
                .font(Theme.mono(11.5))
                .foregroundStyle(Theme.textPrimary)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panelElevated))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))

            Button {
                Task { await onVerify() }
            } label: {
                HStack(spacing: 5) {
                    if isVerifying {
                        ProgressView().controlSize(.small)
                    } else {
                        Image(systemName: "checkmark.seal")
                            .font(.system(size: 10, weight: .bold))
                    }
                    Text(isVerifying ? "Verifying..." : "Verify")
                }
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.mint)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Capsule().fill(Theme.mint.opacity(0.14)))
                .overlay(Capsule().strokeBorder(Theme.mint.opacity(0.4), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .disabled(isVerifying)
        }
    }

    private func verifyOutcomeCard(_ outcome: VerifyOutcomeRecord) -> some View {
        HStack(spacing: 8) {
            Image(systemName: outcome.verified ? "checkmark.circle.fill" : "xmark.circle.fill")
                .foregroundStyle(outcome.verified ? Theme.mint : Theme.coral)
            Text(outcome.message.isEmpty ? (outcome.verified ? "verified" : "not verified") : outcome.message)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textSecondary)
                .lineLimit(2)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill((outcome.verified ? Theme.mint : Theme.coral).opacity(0.08))
        )
    }
}
