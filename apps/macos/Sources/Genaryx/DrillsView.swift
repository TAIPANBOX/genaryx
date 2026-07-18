import GenaryxCoreFFI
import SwiftUI

/// The Drills panel: on-demand "Run drills" controls, an overall verdict
/// (held vs gap, from `has_gaps`), per-scenario results with their findings
/// surfaced as clear action items (skipped findings shown separately), and a
/// fail-on-skip toggle. Fed entirely by `DrillsModel` (the Mockryx reads);
/// like `CryptoView`, this panel has no bus-event filter section (mockryx has
/// no "live" signal of its own - every rehearsal is an explicit run) and no
/// periodic auto-refresh - every number on screen is "as of last run", at
/// parity with the Tauri shell's own Drills panel.
@MainActor
struct DrillsView: View {
    let model: DrillsModel

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                DrillsEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                environmentChip
                runControls

                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                dashboard
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Verdict hero (held/gap/skip + a FuseBar) over a single full-width
    /// Scenario Results section - the design spec's Drills blueprint
    /// (section 5): `.onDemand(last:)`, since mockryx never auto-refreshes
    /// and every number here is "as of last run". No natural rail content
    /// exists for a one-report-at-a-time panel (unlike Policy/Quality/
    /// Crypto's multi-section tabs), so this skips `DashMain`'s primary/rail
    /// split and renders the one section at full width, matching
    /// `DashSection`'s own `.frame(maxWidth: .infinity)`.
    private var dashboard: some View {
        VStack(spacing: 16) {
            HeroBand {
                verdictHero
            } tiles: {
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 14), GridItem(.flexible(), spacing: 14)], spacing: 14) {
                    KpiTile(label: "held", value: model.report.map { String($0.heldCount) } ?? "-", tone: Theme.mint)
                    KpiTile(
                        label: "gap", value: model.report.map { String($0.gapCount) } ?? "-",
                        tone: (model.report?.hasGaps ?? false) ? Theme.coral : nil)
                    KpiTile(label: "skip", value: model.report.map { String($0.skippedCount) } ?? "-")
                    KpiTile(
                        label: "budget burned", value: model.report.map { MoneyFormat.usd($0.totalBudgetBurnedUsd) } ?? "-",
                        tone: Theme.amber)
                }
            }

            DashSection(title: "Scenario Results", badge: .onDemand(last: DrillRunFormat.clock(model.report))) {
                if let report = model.report {
                    ScenarioResultsSection(results: report.results)
                        .padding(.top, 6)
                        .padding(.bottom, 12)
                } else {
                    Text(model.isRunning ? "running..." : "run a drill to see the verdict.")
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textTertiary)
                        .padding(.horizontal, 20)
                        .padding(.vertical, 16)
                }
            }
        }
    }

    /// Falls back to an honest "no run yet" card before the first run (or
    /// past-saved-report load) completes, mirroring `CryptoView.ncscHero`'s
    /// own optional-hero precedent.
    @ViewBuilder
    private var verdictHero: some View {
        if let report = model.report {
            VerdictHero(report: report)
        } else {
            VStack(alignment: .leading, spacing: 8) {
                Text("Drills \u{00B7} verdict")
                    .font(Theme.mono(10.5, weight: .semibold))
                    .tracking(1.6)
                    .foregroundStyle(Theme.textTertiary)
                Text(model.isRunning ? "running..." : "run a drill to see the verdict.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            }
            .padding(.horizontal, 24)
            .padding(.top, 22)
            .padding(.bottom, 18)
            .frame(maxWidth: .infinity, alignment: .leading)
            .dashCard()
        }
    }

    @ViewBuilder
    private var environmentChip: some View {
        // Defensive-only: `body` already gates `content` (and therefore this
        // chip) on `model.connection.isReady` - same convention
        // `CryptoView.environmentChip` documents for its own unreachable
        // non-`.ready` branch.
        if case .ready(let source, let mockryxBin) = model.connection {
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    Circle().fill(Theme.sourceColor("mockryx")).frame(width: 6, height: 6)
                    Text("\(sourceLabel(source)) \u{00B7} \(mockryxBin)")
                        .font(Theme.mono(11, weight: .medium))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Capsule().fill(Theme.panelElevated))
                .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))

                Spacer(minLength: 0)
            }
        }
    }

    private func sourceLabel(_ source: DrillsEnvSource) -> String {
        switch source {
        case .checkout:
            "checkout \u{00B7} bin/mockryx"
        case .explicit:
            "explicit path"
        }
    }

    /// docs/PHASE4.md W2: "a 'Run drills' action calling `run(...)` on
    /// demand"; "(4) a fail-on-skip toggle" - never auto-run (see the type
    /// doc).
    private var runControls: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                labeledField("scenarios", text: Binding(get: { model.scenarioDir }, set: { model.scenarioDir = $0 }))
                labeledField("gateway", text: Binding(get: { model.gateway }, set: { model.gateway = $0 }))
                labeledField("api key (optional)", text: Binding(get: { model.apiKey }, set: { model.apiKey = $0 }))
            }
            HStack(spacing: 12) {
                Toggle(
                    "fail on skip", isOn: Binding(get: { model.failOnSkip }, set: { model.failOnSkip = $0 })
                )
                .toggleStyle(.checkbox)
                .font(Theme.mono(11, weight: .medium))
                .foregroundStyle(Theme.textSecondary)

                runButton

                Text(DrillRunFormat.label(model.report))
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)

                Spacer(minLength: 0)
            }
            // A dim, always-visible awareness caption - Run drills sends
            // real adversarial traffic at a live TokenFuse gateway and
            // really burns budget (see `DrillsModel`'s own type doc: "genuinely
            // consequential"), never a dry-run.
            Text("Runs real gateway calls and burns real budget.")
                .font(Theme.mono(10, weight: .medium))
                .foregroundStyle(Theme.textTertiary)
        }
    }

    private func labeledField(_ placeholder: String, text: Binding<String>) -> some View {
        TextField(placeholder, text: text)
            .textFieldStyle(.plain)
            .font(Theme.mono(11.5))
            .foregroundStyle(Theme.textPrimary)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panelElevated))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))
    }

    private var runButton: some View {
        Button {
            Task { await model.run() }
        } label: {
            HStack(spacing: 5) {
                if model.isRunning {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "bolt.horizontal.fill")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(model.isRunning ? "Running..." : "Run drills")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.coral)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.coral.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.coral.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isRunning)
    }
}

// MARK: - DrillReportRecord verdict derivations

/// Held/gap/skip counts + total budget burned, shared by `VerdictHero` and
/// `DrillsView.dashboard`'s own KPI tiles - one small file-local extension
/// rather than two copies of the same three filters (`ScenarioResultsSection`
/// and the old `VerdictHero.summaryCounts` duplicated this logic before the
/// dashboard conversion; folded into one place now that two call sites need
/// it). Mirrors `CryptoView.findingsSeverityBreakdown` in spirit (a small
/// panel-specific derivation), just factored as a type extension instead of
/// a free function since two different views in this file both need it.
extension DrillReportRecord {
    fileprivate var heldCount: Int { results.filter { $0.status == "passed" && $0.findings.isEmpty }.count }
    fileprivate var gapCount: Int { results.filter { !$0.findings.isEmpty || $0.status == "failed" }.count }
    fileprivate var skippedCount: Int { results.filter { $0.status == "skipped_not_configured" && $0.findings.isEmpty }.count }
    fileprivate var totalBudgetBurnedUsd: Double { results.reduce(0) { $0 + $1.metrics.budgetBurnedUsd } }
}

// MARK: - VerdictHero

/// docs/PHASE4.md W2: "rendering the MockryxReport with an overall verdict
/// from has_gaps()". The dashboard conversion adds a `FuseBar` reading
/// held/total as its fraction, toned mint when every scenario held and ember
/// the moment any gap exists (design spec section 5: "verdict hero with
/// FuseBar, mint all-held, ember gaps") - the color is driven by
/// `report.hasGaps` itself, not a numeric threshold, so a single gap among a
/// hundred held scenarios still reads unmistakably as ember.
@MainActor
private struct VerdictHero: View {
    let report: DrillReportRecord

    private var total: Int { report.results.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Drills \u{00B7} verdict")
                .font(Theme.mono(10.5, weight: .semibold))
                .tracking(1.6)
                .foregroundStyle(Theme.textTertiary)

            HStack(alignment: .lastTextBaseline, spacing: 14) {
                Text(DrillVerdictFormat.label(hasGaps: report.hasGaps))
                    .font(Theme.display(30, weight: .heavy))
                    .foregroundStyle(DrillVerdictFormat.color(hasGaps: report.hasGaps))
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                Spacer(minLength: 8)
                summaryCounts
            }
            .padding(.top, 6)

            if total > 0 {
                FuseBar(fraction: Double(report.heldCount) / Double(total), tone: report.hasGaps ? .ember : .mint)
                    .padding(.top, 6)
            }

            Text("run \(report.runId) \u{00B7} gateway \(report.gateway)")
                .font(Theme.mono(11.5))
                .foregroundStyle(Theme.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .padding(.top, 6)
        }
        .padding(.horizontal, 24)
        .padding(.top, 22)
        .padding(.bottom, 18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .dashCard()
    }

    private var summaryCounts: some View {
        HStack(spacing: 14) {
            countTile("held", report.heldCount, Theme.mint)
            countTile("gap", report.gapCount, Theme.coral)
            countTile("skip", report.skippedCount, Theme.textTertiary)
        }
    }

    private func countTile(_ label: String, _ count: Int, _ tone: Color) -> some View {
        VStack(alignment: .trailing, spacing: 1) {
            Text(String(count))
                .font(Theme.mono(16, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(tone)
            Text(label)
                .font(Theme.mono(9, weight: .semibold))
                .tracking(0.6)
                .foregroundStyle(Theme.textTertiary)
        }
    }
}

// MARK: - ScenarioResultsSection

/// docs/PHASE4.md W2: "per-scenario status (passed=held / failed=GAP /
/// skipped_not_configured=skip) + metrics; read 'gap' from findings/failed,
/// not status alone" + "(3) findings as clear action items... skipped_findings
/// shown separately".
@MainActor
private struct ScenarioResultsSection: View {
    let results: [DrillResultRecord]

    var body: some View {
        if results.isEmpty {
            Text("this run rehearsed no scenarios.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 4)
        } else {
            VStack(spacing: 10) {
                ForEach(results, id: \.scenario) { result in
                    ScenarioResultCard(result: result)
                }
            }
        }
    }
}

@MainActor
private struct ScenarioResultCard: View {
    let result: DrillResultRecord

    private var hasFindings: Bool { !result.findings.isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            if hasFindings {
                findingsList(title: "FINDINGS", findings: result.findings, tone: Theme.coral)
            }
            if !result.skippedFindings.isEmpty {
                findingsList(
                    title: "SKIPPED FINDINGS (guardrail never observed active)", findings: result.skippedFindings,
                    tone: Theme.textTertiary)
            }
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .fill(Theme.panel)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .strokeBorder(Theme.hairline, lineWidth: 1)
        )
    }

    private var header: some View {
        HStack(spacing: 10) {
            statusBadge
            Text(result.scenario)
                .font(Theme.mono(12.5, weight: .medium))
                .foregroundStyle(Theme.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 8)
            metricsText
        }
    }

    private var statusBadge: some View {
        let label = DrillStatusFormat.label(status: result.status, hasFindings: hasFindings)
        let color = DrillStatusFormat.color(status: result.status, hasFindings: hasFindings)
        return Text(label.uppercased())
            .font(Theme.mono(10, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(color)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(color.opacity(0.16)))
            .overlay(Capsule().strokeBorder(color.opacity(0.45), lineWidth: 1))
    }

    private var metricsText: some View {
        Text("\(result.metrics.calls) call\(result.metrics.calls == 1 ? "" : "s") \u{00B7} \(MoneyFormat.usd(result.metrics.budgetBurnedUsd)) burned")
            .font(Theme.mono(10.5))
            .monospacedDigit()
            .foregroundStyle(Theme.textTertiary)
    }

    private func findingsList(title: String, findings: [DrillFindingRecord], tone: Color) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(Theme.mono(9.5, weight: .semibold))
                .tracking(0.6)
                .foregroundStyle(tone)
            VStack(spacing: 6) {
                ForEach(Array(findings.enumerated()), id: \.offset) { _, finding in
                    FindingRow(finding: finding, tone: tone)
                }
            }
        }
    }
}

/// One action item: step, expected vs got status+headers, detail
/// (docs/PHASE4.md W2: "scenario/step, expected vs got status+headers,
/// detail").
@MainActor
private struct FindingRow: View {
    let finding: DrillFindingRecord
    let tone: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(finding.step)
                    .font(Theme.mono(11, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text("attempt \(finding.attempt)")
                    .font(Theme.mono(10))
                    .foregroundStyle(Theme.textTertiary)
                Spacer(minLength: 8)
                Text("expected \(finding.expectStatus) \u{2192} got \(finding.gotStatus)")
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(tone)
            }
            if let expectHeader = finding.expectHeader, !expectHeader.isEmpty {
                headerLine("expect headers", expectHeader)
            }
            if let gotHeaders = finding.gotHeaders, !gotHeaders.isEmpty {
                headerLine("got headers", gotHeaders)
            }
            Text(finding.detail)
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if let source = finding.expectEventSource, let type = finding.expectEventType {
                Text("expected event \u{00B7} \(source)/\(type)")
                    .font(Theme.mono(10))
                    .foregroundStyle(Theme.textTertiary)
            }
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(tone.opacity(0.06))
        )
    }

    private func headerLine(_ label: String, _ headers: [HeaderEntry]) -> some View {
        Text("\(label): \(headers.map { "\($0.key)=\($0.value)" }.joined(separator: ", "))")
            .font(Theme.mono(10))
            .foregroundStyle(Theme.textTertiary)
            .lineLimit(1)
            .truncationMode(.tail)
    }
}
