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
            VStack(alignment: .leading, spacing: 20) {
                environmentChip
                runControls

                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                if let report = model.report {
                    section(title: "Verdict") {
                        VerdictHero(report: report)
                    }
                    section(title: "Scenario Results") {
                        ScenarioResultsSection(results: report.results)
                    }
                } else {
                    section(title: "Verdict") {
                        Text(model.isRunning ? "running..." : "run a drill to see the verdict.")
                            .font(Theme.mono(12))
                            .foregroundStyle(Theme.textTertiary)
                            .padding(.vertical, 4)
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
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

    @ViewBuilder
    private func section<Content: View>(title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title.uppercased())
                .font(Theme.mono(11, weight: .semibold))
                .tracking(1.4)
                .foregroundStyle(Theme.textTertiary)
            content()
        }
    }
}

// MARK: - VerdictHero

/// docs/PHASE4.md W2: "rendering the MockryxReport with an overall verdict
/// from has_gaps()".
@MainActor
private struct VerdictHero: View {
    let report: DrillReportRecord

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            VStack(alignment: .leading, spacing: 4) {
                Text(DrillVerdictFormat.label(hasGaps: report.hasGaps))
                    .font(Theme.mono(16, weight: .bold))
                    .foregroundStyle(DrillVerdictFormat.color(hasGaps: report.hasGaps))
                Text("run \(report.runId) \u{00B7} gateway \(report.gateway)")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 8)
            summaryCounts
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .fill(Theme.panelElevated)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .strokeBorder(DrillVerdictFormat.color(hasGaps: report.hasGaps).opacity(0.4), lineWidth: 1)
        )
    }

    private var summaryCounts: some View {
        let held = report.results.filter { $0.status == "passed" && $0.findings.isEmpty }.count
        let gaps = report.results.filter { !$0.findings.isEmpty || $0.status == "failed" }.count
        let skipped = report.results.filter { $0.status == "skipped_not_configured" && $0.findings.isEmpty }.count
        return HStack(spacing: 14) {
            countTile("held", held, Theme.mint)
            countTile("gap", gaps, Theme.coral)
            countTile("skip", skipped, Theme.textTertiary)
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
