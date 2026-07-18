import GenaryxCoreFFI
import SwiftUI

/// The Money panel: a spend hero + KPI tiles, an interactive runs board (the
/// full break-glass kill / inline budget / replay actions, plus agent drill-in),
/// an incidents feed with ack, and a savings composition. Fed by `CloudModel`,
/// styled with the shared dashboard kit.
@MainActor
struct MoneyView: View {
    let model: CloudModel
    /// PHASE3 W4: opens Run Replay focused on one run.
    let onOpenReplay: (String) -> Void
    /// Opens the agent's Agent 360 card in place (a sheet), never a tab switch.
    var onOpenAgent: (String) -> Void = { _ in }

    private static let refreshInterval: Duration = .seconds(20)
    private static let runsShown = 18

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
            VStack(alignment: .leading, spacing: 16) {
                if let notice = model.mutationNotice {
                    noticeBar(notice)
                }
                if let planRequired = model.planRequired {
                    UpsellBannerView(notice: planRequired)
                }
                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }
                dashboard
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var dashboard: some View {
        let runs = model.runs
        let totalSpent = runs.reduce(0) { $0 + $1.spentUsd }
        let totalCalls = runs.reduce(UInt64(0)) { $0 + $1.calls }
        let activeRuns = runs.filter { !$0.killed }.count
        let saved = model.savings?.totalSavedUsd ?? 0
        let gross = totalSpent + saved
        let savePct = gross > 0 ? Int((saved / gross * 100).rounded()) : 0
        let blocked = model.savings?.blockedSpendUsd ?? 0
        let openIncidents = model.incidents.filter { !$0.acknowledged }.count
        let series = Dash.spendSeries(runs)
        let topRuns = runs.sorted {
            $0.killed != $1.killed ? (!$0.killed && $1.killed) : $0.spentUsd > $1.spentUsd
        }.prefix(Self.runsShown)
        let topIncidents = model.incidents.sorted {
            Dash.sevRank($0.severity) != Dash.sevRank($1.severity)
                ? Dash.sevRank($0.severity) > Dash.sevRank($1.severity)
                : $0.occurrences > $1.occurrences
        }.prefix(7)

        let compItems: [DashCompItem] = model.savings.map { s in
            [
                DashCompItem(id: "blocked", label: "Runaway blocked", value: s.blockedSpendUsd, total: saved, tone: .ember, valueText: MoneyFormat.usd(s.blockedSpendUsd)),
                DashCompItem(id: "cache", label: "Semantic cache", value: s.cacheSavedUsd, total: saved, tone: .mint, valueText: MoneyFormat.usd(s.cacheSavedUsd)),
                DashCompItem(id: "router", label: "Model router", value: s.routerSavedUsd, total: saved, tone: .iris, valueText: MoneyFormat.usd(s.routerSavedUsd)),
            ]
        } ?? []

        return VStack(spacing: 16) {
            HeroBand {
                HeroCard(
                    cap: "AI spend \u{00B7} live fleet",
                    value: Dash.usd0(totalSpent),
                    sub: Text("governed savings ") + Text(MoneyFormat.usd(saved)).foregroundColor(Theme.mint).fontWeight(.semibold),
                    series: series.isEmpty ? nil : series,
                    fuseFraction: gross > 0 ? saved / gross : 0,
                    fuseTone: .iris,
                    note: (
                        left: Text("prevented ") + Text(MoneyFormat.usd(blocked)).foregroundColor(Theme.textPrimary).fontWeight(.semibold) + Text(" runaway spend"),
                        right: Text("recovered ") + Text("\(savePct)%").foregroundColor(Theme.textPrimary).fontWeight(.semibold) + Text(" of gross draw")
                    )
                )
            } tiles: {
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 14), GridItem(.flexible(), spacing: 14)], spacing: 14) {
                    KpiTile(label: "active runs", value: Dash.int(activeRuns), sub: "\(Dash.int(runs.count)) in window")
                    KpiTile(label: "model calls", value: Dash.int(Int(totalCalls)), sub: "metered through gateway")
                    KpiTile(label: "governed saved", value: MoneyFormat.usd(saved), sub: "\(model.savings?.budgetBreaks ?? 0) budget breaks", tone: Theme.mint)
                    KpiTile(label: "open incidents", value: String(openIncidents), sub: "\(model.incidents.count) detected", tone: openIncidents > 0 ? Theme.coral : nil)
                }
            }

            DashMain {
                DashSection(title: "Runs", right: "top \(min(Self.runsShown, runs.count)) by spend \u{00B7} full stream in Bus") {
                    RunsBoard(
                        runs: Array(topRuns),
                        onKill: { runId, reason in await model.killRun(runId, reason: reason) },
                        onSetBudget: { runId, usd, reason in await model.setBudget(runId: runId, usd: usd, reason: reason) },
                        onOpenReplay: onOpenReplay,
                        onOpenAgent: onOpenAgent)
                }
            } rail: {
                if !compItems.isEmpty {
                    DashSection(title: "Governed savings", right: "prevented + recovered") {
                        DashComposition(items: compItems)
                    }
                }
                DashSection(title: "Incidents", right: "worst first") {
                    IncidentFeed(incidents: Array(topIncidents), onAck: { await model.ackIncident($0) })
                }
            }
        }
    }

    private func noticeBar(_ text: String) -> some View {
        Text(text)
            .font(Theme.mono(11.5))
            .foregroundStyle(Theme.mint)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .dashCard()
    }
}

// MARK: - RunsBoard

/// The interactive runs board: a readable, dashboard-styled table that keeps
/// every operator action (open the agent's 360 in place, replay, inline budget,
/// break-glass kill). The caller passes a curated, capped slice.
@MainActor
private struct RunsBoard: View {
    let runs: [Run]
    let onKill: (String, String) async -> Bool
    let onSetBudget: (String, Double, String) async -> Bool
    let onOpenReplay: (String) -> Void
    let onOpenAgent: (String) -> Void

    var body: some View {
        if runs.isEmpty {
            Text("no runs yet.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.horizontal, 20)
                .padding(.vertical, 20)
        } else {
            VStack(spacing: 0) {
                header
                ForEach(Array(runs.enumerated()), id: \.element.runId) { index, run in
                    Divider().overlay(Theme.hairline)
                    RunRow(run: run, onKill: onKill, onSetBudget: onSetBudget, onOpenReplay: onOpenReplay, onOpenAgent: onOpenAgent)
                }
            }
            .padding(.bottom, 4)
        }
    }

    private var header: some View {
        HStack(spacing: 12) {
            label("RUN \u{00B7} AGENT").frame(maxWidth: .infinity, alignment: .leading)
            label("SPENT / BUDGET").frame(width: 168, alignment: .leading)
            label("CALLS").frame(width: 70, alignment: .trailing)
            label("STATUS").frame(width: 84, alignment: .leading)
            Color.clear.frame(width: 210)
        }
        .padding(.horizontal, 20)
        .padding(.bottom, 10)
    }

    private func label(_ text: String) -> some View {
        Text(text).font(Theme.mono(9.5, weight: .semibold)).tracking(0.8).foregroundStyle(Theme.textTertiary)
    }

    private struct RunRow: View {
        let run: Run
        let onKill: (String, String) async -> Bool
        let onSetBudget: (String, Double, String) async -> Bool
        let onOpenReplay: (String) -> Void
        let onOpenAgent: (String) -> Void

        private enum BreakGlassAction { case kill, budget(usd: Double) }
        @State private var armed: BreakGlassAction?

        private var fraction: Double {
            guard let b = run.budgetUsd, b > 0 else { return 0 }
            return run.spentUsd / b
        }

        var body: some View {
            VStack(alignment: .leading, spacing: 0) {
                row
                if let armed {
                    BreakGlassPanel(
                        summary: summary(for: armed),
                        onConfirm: { reason in
                            switch armed {
                            case .kill: _ = await onKill(run.runId, reason)
                            case .budget(let usd): _ = await onSetBudget(run.runId, usd, reason)
                            }
                            self.armed = nil
                        },
                        onCancel: { self.armed = nil })
                    .padding(.top, 8)
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 11)
        }

        private var row: some View {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(run.runId)
                        .font(.system(size: 12.5))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1).truncationMode(.middle)
                    Button {
                        if !run.agentId.isEmpty { onOpenAgent(run.agentId) }
                    } label: {
                        Text(run.agentId.isEmpty ? "-" : Dash.agentShort(run.agentId))
                            .font(Theme.mono(10.5))
                            .foregroundStyle(Theme.textSecondary)
                            .lineLimit(1).truncationMode(.tail)
                    }
                    .buttonStyle(.plain)
                    .disabled(run.agentId.isEmpty)
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        Text(MoneyFormat.usd(run.spentUsd)).font(Theme.mono(12.5)).monospacedDigit().foregroundStyle(Theme.textPrimary)
                        Spacer(minLength: 4)
                        Text(run.budgetUsd.map(MoneyFormat.usd) ?? "no cap").font(Theme.mono(11)).foregroundStyle(Theme.textSecondary)
                    }
                    if let b = run.budgetUsd, b > 0 {
                        FuseBar(fraction: fraction, height: 6)
                    }
                }
                .frame(width: 168, alignment: .leading)

                Text("\(run.calls) \u{00B7} \(run.steps)")
                    .font(Theme.mono(12)).monospacedDigit().foregroundStyle(Theme.textSecondary)
                    .frame(width: 70, alignment: .trailing)

                statusPill.frame(width: 84, alignment: .leading)

                actions.frame(width: 210, alignment: .trailing)
            }
        }

        @ViewBuilder private var statusPill: some View {
            let (text, color): (String, Color) = run.killed
                ? ("killed", Theme.textTertiary)
                : fraction >= 1 ? ("over cap", Theme.ember)
                : fraction >= 0.8 ? ("near cap", Theme.amber)
                : ("live", Theme.mint)
            Text(text.uppercased())
                .font(Theme.mono(9, weight: .semibold)).tracking(0.5)
                .foregroundStyle(color)
                .padding(.horizontal, 9).padding(.vertical, 3)
                .background(Capsule().fill(color.opacity(0.12)))
                .overlay(Capsule().strokeBorder(color.opacity(0.35), lineWidth: 1))
        }

        private var actions: some View {
            HStack(spacing: 10) {
                Button { onOpenReplay(run.runId) } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "play.circle").font(.system(size: 9, weight: .bold))
                        Text("Replay")
                    }
                    .font(Theme.mono(11, weight: .semibold)).foregroundStyle(Theme.iris)
                }
                .buttonStyle(.plain)

                if run.killed {
                    Text("killed").font(Theme.mono(10, weight: .semibold)).foregroundStyle(Theme.textTertiary)
                        .padding(.horizontal, 8).padding(.vertical, 3)
                        .background(Capsule().fill(Theme.textTertiary.opacity(0.12)))
                } else {
                    BudgetEditor(runId: run.runId, currentUsd: run.budgetUsd, onArm: { usd in armed = .budget(usd: usd) })
                    Button { armed = .kill } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 8, weight: .bold))
                            Text("Kill")
                        }
                        .font(Theme.mono(11, weight: .semibold)).foregroundStyle(Theme.ember)
                    }
                    .buttonStyle(.plain)
                }
            }
        }

        private func summary(for action: BreakGlassAction) -> String {
            switch action {
            case .kill: return "Kill run \(run.runId) immediately."
            case .budget(let usd): return "Set run \(run.runId)'s budget to \(MoneyFormat.usd(usd))."
            }
        }
    }
}

// MARK: - IncidentFeed

/// The incidents rail: severity dot + kind + sub + occurrences, with an inline
/// ack ceremony (reusing `ConfirmButton`).
@MainActor
private struct IncidentFeed: View {
    let incidents: [Incident]
    let onAck: (String) async -> Bool

    var body: some View {
        if incidents.isEmpty {
            Text("no incidents.")
                .font(Theme.mono(12)).foregroundStyle(Theme.textTertiary)
                .frame(maxWidth: .infinity).padding(.vertical, 24)
        } else {
            VStack(spacing: 0) {
                ForEach(Array(incidents.enumerated()), id: \.element.id) { i, inc in
                    if i > 0 { Divider().overlay(Theme.hairline) }
                    row(inc)
                }
            }
        }
    }

    private func row(_ inc: Incident) -> some View {
        let color = Theme.severityColor(inc.severity)
        return HStack(spacing: 12) {
            Circle().fill(color).frame(width: 9, height: 9).shadow(color: color.opacity(0.7), radius: 4)
            VStack(alignment: .leading, spacing: 2) {
                Text(inc.kind.replacingOccurrences(of: "_", with: " "))
                    .font(.system(size: 12.5)).foregroundStyle(Theme.textPrimary).lineLimit(1).truncationMode(.tail)
                Text("\(inc.runId ?? inc.agentId ?? "fleet") \u{00B7} \(inc.occurrences)\u{00D7}")
                    .font(Theme.mono(10.5)).foregroundStyle(Theme.textSecondary).lineLimit(1).truncationMode(.tail)
            }
            Spacer(minLength: 8)
            if inc.acknowledged {
                Text("acked").font(Theme.mono(10, weight: .semibold)).foregroundStyle(Theme.mint)
                    .padding(.horizontal, 8).padding(.vertical, 3)
                    .background(Capsule().fill(Theme.mint.opacity(0.14)))
            } else {
                ConfirmButton(label: "Ack", confirmLabel: "Confirm", tone: Theme.amber, onConfirm: { _ = await onAck(inc.id) })
            }
        }
        .padding(.horizontal, 20).padding(.vertical, 12)
    }
}
