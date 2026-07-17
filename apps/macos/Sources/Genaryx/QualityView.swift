import GenaryxCoreFFI
import SwiftUI

/// The Quality panel: the Eval Runs history (selectable, newest first), a
/// Run Detail section for whichever row is selected (summary header +
/// per-case scores), a Baselines list, and live Drift Alerts filtered from
/// the shared bus feed. Fed by `QualityModel` (the Verdryx reads) plus the
/// app's own `FleetModel` bus events - Drift Alerts is a FILTER over the same
/// live tail the Bus Explorer renders, never a new read through
/// `QualityHandle` (docs/PHASE4.md W1 grounding: the panel's drift signal IS
/// the `quality_drift` bus event) - at parity with the Tauri shell's own
/// Quality panel.
@MainActor
struct QualityView: View {
    let model: QualityModel
    /// The app-wide bus feed (`FleetModel.events`), filtered below to
    /// `source == "verdryx"` / `eventType == "quality_drift"` for Drift
    /// Alerts - see the type doc.
    let busEvents: [UiEvent]

    private static let refreshInterval: Duration = .seconds(20)

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                QualityEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task(id: model.connection.isReady) {
            guard model.connection.isReady else { return }
            while !Task.isCancelled {
                await model.refresh()
                try? await Task.sleep(for: Self.refreshInterval)
            }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                environmentChip

                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                section(title: "Eval Runs") {
                    EvalRunsSection(
                        runs: model.evalRuns,
                        summaries: model.runSummaries,
                        selectedRunId: model.selectedRunId,
                        onSelect: { runId in await model.selectRun(runId) }
                    )
                }
                section(title: "Run Detail") {
                    RunDetailSection(
                        summary: model.selectedRunSummary, scores: model.scores, isLoading: model.isLoadingDetail)
                }
                section(title: "Baselines") {
                    BaselinesSection(baselines: model.baselines)
                }
                section(title: "Drift Alerts") {
                    DriftAlertsSection(events: driftEvents)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// docs/PHASE4.md W1: `quality_drift` is high-severity and fires ONLY on
    /// a real regression (source `verdryx`) - the panel's live drift signal.
    private var driftEvents: [UiEvent] {
        busEvents.filter { $0.source.lowercased() == "verdryx" && $0.eventType == "quality_drift" }
    }

    @ViewBuilder
    private var environmentChip: some View {
        // Defensive-only: `body` already gates `content` (and therefore this
        // chip) on `model.connection.isReady` - same convention
        // `IdentityView.environmentChip` documents for its own unreachable
        // non-`.ready` branch.
        if case .ready(let source, let dbPath) = model.connection {
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    Circle().fill(Theme.sourceColor("verdryx")).frame(width: 6, height: 6)
                    Text("\(sourceLabel(source)) \u{00B7} \(dbPath)")
                        .font(Theme.mono(11, weight: .medium))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Capsule().fill(Theme.panelElevated))
                .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))

                Text(LoadedAtFormat.label(model.loadedAt))
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)

                Spacer(minLength: 0)
            }
        }
    }

    private func sourceLabel(_ source: QualityEnvSource) -> String {
        switch source {
        case .explicit:
            "explicit path"
        case .taipan:
            "taipan \u{00B7} well-known"
        case .workingDirectory:
            "working directory"
        }
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

// MARK: - EvalRunsSection

/// docs/PHASE4.md W1 row shape: model, started/finished, per-run summary
/// (case count, mean score, total cost), newest first, selectable.
@MainActor
private struct EvalRunsSection: View {
    let runs: [EvalRunRecord]
    let summaries: [String: RunSummaryRecord]
    let selectedRunId: String?
    let onSelect: (String) async -> Void

    private static let displayLimit = 100

    var body: some View {
        if runs.isEmpty {
            Text("no eval runs in verdryx.db yet.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 4)
        } else {
            VStack(spacing: 8) {
                ForEach(Array(runs.prefix(Self.displayLimit)), id: \.id) { run in
                    EvalRunRow(
                        run: run, summary: summaries[run.id], isSelected: run.id == selectedRunId,
                        onSelect: { Task { await onSelect(run.id) } })
                }
            }
            if runs.count > Self.displayLimit {
                Text("+\(runs.count - Self.displayLimit) more (showing newest \(Self.displayLimit))")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
            }
        }
    }

    /// One eval run: model + id + started/finished, plus (once its
    /// `run_summary` fan-out resolves) the case-count/mean/cost tri-stat.
    /// The whole row is tappable - selecting it loads Run Detail below.
    private struct EvalRunRow: View {
        let run: EvalRunRecord
        let summary: RunSummaryRecord?
        let isSelected: Bool
        let onSelect: () -> Void

        var body: some View {
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(run.model)
                        .font(Theme.mono(12, weight: .medium))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Text(run.id)
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(timesText)
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                statsColumn
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(isSelected ? Theme.rose.opacity(0.10) : Theme.panelElevated)
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .strokeBorder(isSelected ? Theme.rose.opacity(0.6) : Color.clear, lineWidth: 1.5)
            )
            .contentShape(Rectangle())
            .onTapGesture(perform: onSelect)
            .help("Select \(run.id) to see its run detail")
        }

        private var timesText: String {
            let started = MoneyFormat.timestamp(run.startedAt)
            guard let finishedAt = run.finishedAt else {
                return "started \(started) \u{00B7} in flight"
            }
            return "started \(started) \u{00B7} finished \(MoneyFormat.timestamp(finishedAt))"
        }

        private var statsColumn: some View {
            VStack(alignment: .trailing, spacing: 2) {
                if let summary {
                    Text(
                        "\(summary.caseCount) case\(summary.caseCount == 1 ? "" : "s") \u{00B7} mean \(QualityFormat.meanScore(summary.meanScore))"
                    )
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                    Text(MoneyFormat.usd(summary.totalCostUsd))
                        .font(Theme.mono(11, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(Theme.amber)
                } else {
                    Text("summary loading...")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
        }
    }
}

// MARK: - RunDetailSection

/// docs/PHASE4.md W1: "run detail = summary header (mean shown 'n/a' when
/// null, never 0) + per-case scores table".
@MainActor
private struct RunDetailSection: View {
    let summary: RunSummaryRecord?
    let scores: [ScoreRecord]
    let isLoading: Bool

    private static let displayLimit = 200

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let summary {
                headerStats(summary)
            } else if isLoading {
                Text("loading run detail...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            } else {
                Text("select a run above to see its detail.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            }

            if scores.isEmpty {
                if summary != nil {
                    Text("no per-case scores for this run.")
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textTertiary)
                        .padding(.vertical, 4)
                }
            } else {
                scoresTable
            }
        }
    }

    private func headerStats(_ summary: RunSummaryRecord) -> some View {
        HStack(spacing: 12) {
            StatTile(label: "cases", value: String(summary.caseCount))
            StatTile(
                label: "mean score", value: QualityFormat.meanScore(summary.meanScore),
                tone: summary.meanScore == nil ? Theme.textTertiary : Theme.mint)
            StatTile(label: "tokens", value: String(summary.totalTokens))
            StatTile(label: "total cost", value: MoneyFormat.usd(summary.totalCostUsd), tone: Theme.amber)
        }
    }

    private var scoresTable: some View {
        let shown = Array(scores.prefix(Self.displayLimit))
        return VStack(spacing: 0) {
            scoresHeader
            Divider().overlay(Theme.hairlineStrong)
            ForEach(Array(shown.enumerated()), id: \.element.id) { index, score in
                ScoreRow(score: score)
                if index < shown.count - 1 {
                    Divider().overlay(Theme.hairline)
                }
            }
            if scores.count > Self.displayLimit {
                Text("+\(scores.count - Self.displayLimit) more (showing first \(Self.displayLimit))")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 6)
            }
        }
        .background(Theme.panel)
        .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .strokeBorder(Theme.hairline, lineWidth: 1)
        )
    }

    private var scoresHeader: some View {
        HStack(spacing: 12) {
            columnLabel("CASE").frame(maxWidth: .infinity, alignment: .leading)
            columnLabel("SCORE").frame(width: 70, alignment: .trailing)
            columnLabel("TOKENS").frame(width: 80, alignment: .trailing)
            columnLabel("COST").frame(width: 90, alignment: .trailing)
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

    private struct ScoreRow: View {
        let score: ScoreRecord

        var body: some View {
            HStack(spacing: 12) {
                Text(score.caseId)
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text(String(format: "%.3f", score.value))
                    .font(Theme.mono(11.5))
                    .monospacedDigit()
                    .foregroundStyle(scoreTone)
                    .frame(width: 70, alignment: .trailing)
                Text(String(score.tokens))
                    .font(Theme.mono(11))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: 80, alignment: .trailing)
                Text(MoneyFormat.usd(score.costUsd))
                    .font(Theme.mono(11))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: 90, alignment: .trailing)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 7)
        }

        /// A quick visual cue keyed off the score's own value - verdryx
        /// scores carry no severity field of their own, so this is a
        /// rendering heuristic local to this row (`>= 0.8` mint, `>= 0.5`
        /// amber, else coral), never a value verdryx itself asserts.
        private var scoreTone: Color {
            if score.value >= 0.8 { return Theme.mint }
            if score.value >= 0.5 { return Theme.amber }
            return Theme.coral
        }
    }
}

// MARK: - BaselinesSection

@MainActor
private struct BaselinesSection: View {
    let baselines: [BaselineRecord]

    var body: some View {
        if baselines.isEmpty {
            Text("no baselines saved.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 4)
        } else {
            VStack(spacing: 8) {
                ForEach(baselines, id: \.id) { baseline in
                    BaselineRow(baseline: baseline)
                }
            }
        }
    }

    private struct BaselineRow: View {
        let baseline: BaselineRecord

        var body: some View {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(baseline.label.isEmpty ? baseline.id : baseline.label)
                        .font(Theme.mono(12, weight: .medium))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Text("from run \(baseline.evalRunId) \u{00B7} \(MoneyFormat.timestamp(baseline.createdAt))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                Spacer(minLength: 8)
                // `mean_score` is a plain (non-Optional) f64 on a baseline
                // snapshot (unlike a run's summary) - a baseline is only ever
                // created FROM a run that already had scores, so there is no
                // "n/a" case here; formatted directly rather than through
                // `QualityFormat.meanScore`, which exists for the Optional
                // shape.
                Text(String(format: "%.3f", baseline.meanScore))
                    .font(Theme.mono(13, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(Theme.mint)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
        }
    }
}

// MARK: - DriftAlertsSection

/// docs/PHASE4.md W1: drift alerts from the live `quality_drift` bus event
/// (high severity, fires only on a real regression).
@MainActor
private struct DriftAlertsSection: View {
    let events: [UiEvent]

    private static let displayLimit = 30

    var body: some View {
        if events.isEmpty {
            Text("no quality_drift events yet - drift alerts fire only on a real regression.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 4)
        } else {
            let shown = Array(events.prefix(Self.displayLimit))
            VStack(spacing: 0) {
                ForEach(Array(shown.enumerated()), id: \.element.rowKey) { index, event in
                    DriftAlertRow(event: event)
                    if index < shown.count - 1 {
                        Divider().overlay(Theme.hairline)
                    }
                }
            }
            .background(Theme.panel)
            .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                    .strokeBorder(Theme.hairline, lineWidth: 1)
            )
        }
    }

    private struct DriftAlertRow: View {
        let event: UiEvent

        var body: some View {
            let fields = event.qualityDriftFields
            HStack(spacing: 12) {
                severityDot
                VStack(alignment: .leading, spacing: 2) {
                    Text((fields.verdict ?? "regressed").uppercased())
                        .font(Theme.mono(11, weight: .semibold))
                        .foregroundStyle(Theme.textPrimary)
                    if let baselineId = fields.baselineId {
                        Text("baseline \(baselineId)\(windowSuffix)")
                            .font(Theme.mono(10.5))
                            .foregroundStyle(Theme.textTertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
                Spacer(minLength: 8)
                deltaColumn(fields)
                Text(MoneyFormat.timestamp(event.ts))
                    .font(Theme.mono(11))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
                    .frame(width: 118, alignment: .trailing)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
        }

        private var windowSuffix: String {
            guard let window = event.qualityDriftFields.window else { return "" }
            return " \u{00B7} window \(window)"
        }

        private var severityDot: some View {
            let color = Theme.severityColor(event.severity)
            return Circle()
                .fill(color)
                .frame(width: 8, height: 8)
                .shadow(color: color.opacity(0.6), radius: 3)
        }

        private func deltaColumn(_ fields: UiEvent.QualityDriftFields) -> some View {
            VStack(alignment: .trailing, spacing: 2) {
                if let meanScore = fields.meanScore {
                    Text("mean \(String(format: "%.3f", meanScore))")
                        .font(Theme.mono(11))
                        .monospacedDigit()
                        .foregroundStyle(Theme.textSecondary)
                }
                if let delta = fields.delta {
                    Text("\u{0394} \(String(format: "%+.3f", delta))")
                        .font(Theme.mono(11, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(delta < 0 ? Theme.coral : Theme.mint)
                }
            }
        }
    }
}
