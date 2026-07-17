import GenaryxCoreFFI
import SwiftUI

/// The Money panel: runs table (kill + inline budget), incidents list
/// (ack), and a savings breakdown. Fed by `CloudModel`, at parity with the
/// Tauri shell's `MoneyView.tsx` / `RunsTable.tsx` / `IncidentsList.tsx` /
/// `SavingsBreakdown.tsx`.
@MainActor
struct MoneyView: View {
    let model: CloudModel

    private static let refreshInterval: Duration = .seconds(20)

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                MoneyEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task(id: model.connection.isReady) {
            guard model.connection.isReady else { return }
            while !Task.isCancelled {
                await model.refreshMoney()
                try? await Task.sleep(for: Self.refreshInterval)
            }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let notice = model.mutationNotice {
                    noticeBar(notice)
                }
                if let planRequired = model.planRequired {
                    UpsellBannerView(notice: planRequired)
                }
                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                section(title: "Runs") {
                    RunsTable(
                        runs: model.runs,
                        onKill: { runId, reason in await model.killRun(runId, reason: reason) },
                        onSetBudget: { runId, usd, reason in await model.setBudget(runId: runId, usd: usd, reason: reason) }
                    )
                }
                section(title: "Incidents") {
                    IncidentsList(incidents: model.incidents, onAck: { await model.ackIncident($0) })
                }
                section(title: "Savings") {
                    if let savings = model.savings {
                        SavingsBreakdown(savings: savings)
                    } else {
                        Text("loading...")
                            .font(Theme.mono(12))
                            .foregroundStyle(Theme.textTertiary)
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func noticeBar(_ text: String) -> some View {
        Text(text)
            .font(Theme.mono(11.5))
            .foregroundStyle(Theme.mint)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
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

// MARK: - RunsTable

@MainActor
private struct RunsTable: View {
    let runs: [Run]
    let onKill: (String, String) async -> Bool
    let onSetBudget: (String, Double, String) async -> Bool

    private enum Column {
        static let agent: CGFloat = 130
        static let spent: CGFloat = 84
        static let budget: CGFloat = 150
        static let calls: CGFloat = 52
        static let steps: CGFloat = 52
        static let lastSeen: CGFloat = 118
        static let actions: CGFloat = 190
    }

    var body: some View {
        if runs.isEmpty {
            Text("no runs yet.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 12)
        } else {
            VStack(spacing: 0) {
                header
                Divider().overlay(Theme.hairlineStrong)
                ForEach(Array(runs.enumerated()), id: \.element.runId) { index, run in
                    RunRow(run: run, onKill: onKill, onSetBudget: onSetBudget)
                    if index < runs.count - 1 {
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

    private var header: some View {
        HStack(spacing: 12) {
            columnLabel("RUN").frame(maxWidth: .infinity, alignment: .leading)
            columnLabel("AGENT").frame(width: Column.agent, alignment: .leading)
            columnLabel("SPENT").frame(width: Column.spent, alignment: .trailing)
            columnLabel("BUDGET").frame(width: Column.budget, alignment: .trailing)
            columnLabel("CALLS").frame(width: Column.calls, alignment: .trailing)
            columnLabel("STEPS").frame(width: Column.steps, alignment: .trailing)
            columnLabel("LAST SEEN").frame(width: Column.lastSeen, alignment: .trailing)
            Color.clear.frame(width: Column.actions)
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

    /// One run row, PLUS (Phase-2 wave 3B) the shared break-glass ceremony
    /// for whichever privileged action - Kill or Set-budget - the operator
    /// just armed. Both actions arm the SAME `armed` state rather than each
    /// running its own confirm flow, so at most one `BreakGlassPanel` shows
    /// per row and "Kill" / `BudgetEditor`'s "Set" read as two doors into
    /// one ceremony, not two different ones.
    private struct RunRow: View {
        let run: Run
        let onKill: (String, String) async -> Bool
        let onSetBudget: (String, Double, String) async -> Bool

        private enum BreakGlassAction {
            case kill
            case budget(usd: Double)
        }

        @State private var armed: BreakGlassAction?

        var body: some View {
            VStack(alignment: .leading, spacing: 0) {
                row
                if let armed {
                    BreakGlassPanel(
                        summary: summary(for: armed),
                        onConfirm: { reason in
                            switch armed {
                            case .kill:
                                _ = await onKill(run.runId, reason)
                            case .budget(let usd):
                                _ = await onSetBudget(run.runId, usd, reason)
                            }
                            self.armed = nil
                        },
                        onCancel: { self.armed = nil }
                    )
                    .padding(.top, 8)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
        }

        private var row: some View {
            HStack(spacing: 12) {
                Text(run.runId)
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .help(run.runId)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Text(run.agentId.isEmpty ? "-" : run.agentId)
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(width: Column.agent, alignment: .leading)

                Text(MoneyFormat.usd(run.spentUsd))
                    .font(Theme.mono(12))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textPrimary)
                    .frame(width: Column.spent, alignment: .trailing)

                Group {
                    if run.killed {
                        Text(run.budgetUsd.map(MoneyFormat.usd) ?? "-")
                            .font(Theme.mono(12))
                            .monospacedDigit()
                            .foregroundStyle(Theme.textSecondary)
                            .frame(width: Column.budget, alignment: .trailing)
                    } else {
                        HStack(spacing: 6) {
                            Text(run.budgetUsd.map(MoneyFormat.usd) ?? "-")
                                .font(Theme.mono(11.5))
                                .monospacedDigit()
                                .foregroundStyle(Theme.textSecondary)
                            BudgetEditor(runId: run.runId, currentUsd: run.budgetUsd, onArm: { usd in
                                armed = .budget(usd: usd)
                            })
                        }
                        .frame(width: Column.budget, alignment: .trailing)
                    }
                }

                Text(String(run.calls))
                    .font(Theme.mono(12))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: Column.calls, alignment: .trailing)

                Text(String(run.steps))
                    .font(Theme.mono(12))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: Column.steps, alignment: .trailing)

                Text(MoneyFormat.timestamp(run.lastSeen))
                    .font(Theme.mono(11))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
                    .frame(width: Column.lastSeen, alignment: .trailing)

                HStack(spacing: 8) {
                    if run.killed {
                        Text("killed")
                            .font(Theme.mono(10, weight: .semibold))
                            .foregroundStyle(Theme.textTertiary)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(Capsule().fill(Theme.textTertiary.opacity(0.12)))
                    } else {
                        Button {
                            armed = .kill
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "exclamationmark.triangle.fill")
                                    .font(.system(size: 8, weight: .bold))
                                Text("Kill")
                            }
                            .font(Theme.mono(11, weight: .semibold))
                            .foregroundStyle(Theme.ember)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .frame(width: Column.actions, alignment: .trailing)
            }
        }

        private func summary(for action: BreakGlassAction) -> String {
            switch action {
            case .kill:
                return "Kill run \(run.runId) immediately."
            case .budget(let usd):
                return "Set run \(run.runId)'s budget to \(MoneyFormat.usd(usd))."
            }
        }
    }
}

// MARK: - IncidentsList

@MainActor
private struct IncidentsList: View {
    let incidents: [Incident]
    let onAck: (String) async -> Bool

    var body: some View {
        if incidents.isEmpty {
            Text("no incidents.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 12)
        } else {
            VStack(spacing: 8) {
                ForEach(incidents, id: \.id) { incident in
                    IncidentRow(incident: incident, onAck: onAck)
                }
            }
        }
    }

    private struct IncidentRow: View {
        let incident: Incident
        let onAck: (String) async -> Bool

        var body: some View {
            HStack(spacing: 12) {
                SeverityPill(severity: incident.severity)

                VStack(alignment: .leading, spacing: 2) {
                    Text(incident.kind)
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Text(subtitle)
                        .font(Theme.mono(11))
                        .foregroundStyle(Theme.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }

                Spacer(minLength: 8)

                if incident.acknowledged {
                    Text("acked")
                        .font(Theme.mono(10, weight: .semibold))
                        .foregroundStyle(Theme.mint)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(Capsule().fill(Theme.mint.opacity(0.14)))
                        .overlay(Capsule().strokeBorder(Theme.mint.opacity(0.4), lineWidth: 1))
                } else {
                    ConfirmButton(
                        label: "Ack",
                        confirmLabel: "Confirm ack",
                        tone: Theme.amber,
                        onConfirm: { _ = await onAck(incident.id) }
                    )
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
        }

        private var subtitle: String {
            let occurrenceWord = incident.occurrences == 1 ? "occurrence" : "occurrences"
            let runLabel = incident.runId ?? "no run"
            return "\(runLabel) \u{00B7} \(incident.occurrences) \(occurrenceWord) \u{00B7} last \(MoneyFormat.timestamp(incident.lastSeen))"
        }
    }
}

// MARK: - SavingsBreakdown

@MainActor
private struct SavingsBreakdown: View {
    let savings: Savings

    var body: some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 4), spacing: 12) {
            StatTile(label: "blocked spend", value: MoneyFormat.usd(savings.blockedSpendUsd))
            StatTile(label: "cache saved", value: MoneyFormat.usd(savings.cacheSavedUsd))
            StatTile(label: "router saved", value: MoneyFormat.usd(savings.routerSavedUsd))
            StatTile(label: "budget breaks", value: String(savings.budgetBreaks))
        }
    }
}
