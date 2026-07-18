import GenaryxCoreFFI
import SwiftUI

/// The Overview panel: a modern control-room dashboard - a spend hero with a
/// burn sparkline and a governance fuse, four KPI tiles, a ranked spend-by-agent
/// board (each agent drills into its Agent 360 in place), a savings composition,
/// and an incidents feed. Fed by `CloudModel`.
@MainActor
struct OverviewView: View {
    let model: CloudModel
    var onOpenAgent: (String) -> Void = { _ in }

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
                await model.refreshOverview()
                await model.refreshMoney()
                try? await Task.sleep(for: Self.refreshInterval)
            }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                environmentChip
                if let planRequired = model.planRequired {
                    UpsellBannerView(notice: planRequired)
                }
                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }
                if let overview = model.overview {
                    dashboard(overview)
                } else {
                    Text("loading control room...")
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private var environmentChip: some View {
        if case .ready(let source, let cloudUrl, _) = model.connection {
            HStack(spacing: 6) {
                Circle().fill(Theme.mint).frame(width: 6, height: 6)
                Text("\(sourceLabel(source)) \u{00B7} \(cloudUrl)")
                    .font(Theme.mono(11, weight: .medium))
                    .foregroundStyle(Theme.textSecondary)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.panelElevated))
            .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
        }
    }

    private func sourceLabel(_ source: EnvSource) -> String {
        switch source {
        case .taipan(let name): "taipan up \u{00B7} \(name)"
        case .envFallback: "env fallback"
        }
    }

    private func dashboard(_ overview: Overview) -> some View {
        let saved = model.savings?.totalSavedUsd ?? overview.totalSavedUsd
        let spent = overview.totalSpentUsd
        let gross = spent + saved
        let savePct = gross > 0 ? Int((saved / gross * 100).rounded()) : 0
        let blocked = model.savings?.blockedSpendUsd ?? 0
        let agents = Dash.spendByAgent(model.runs)
        let maxAgent = max(1, agents.map { $0.spent }.max() ?? 1)
        let series = Dash.spendSeries(model.runs)
        let topIncidents = model.incidents.sorted {
            Dash.sevRank($0.severity) != Dash.sevRank($1.severity)
                ? Dash.sevRank($0.severity) > Dash.sevRank($1.severity)
                : $0.occurrences > $1.occurrences
        }.prefix(6)

        let agentBars = agents.prefix(8).map { a in
            DashBarItem(
                id: a.agent, label: a.name, sub: a.team.isEmpty ? nil : a.team,
                fraction: a.spent / maxAgent, tone: .amber, value: MoneyFormat.usd(a.spent),
                onTap: { onOpenAgent(a.agent) })
        }
        let compItems: [DashCompItem] = model.savings.map { s in
            [
                DashCompItem(id: "blocked", label: "Runaway blocked", value: s.blockedSpendUsd, total: saved, tone: .ember, valueText: MoneyFormat.usd(s.blockedSpendUsd)),
                DashCompItem(id: "cache", label: "Semantic cache", value: s.cacheSavedUsd, total: saved, tone: .mint, valueText: MoneyFormat.usd(s.cacheSavedUsd)),
                DashCompItem(id: "router", label: "Model router", value: s.routerSavedUsd, total: saved, tone: .iris, valueText: MoneyFormat.usd(s.routerSavedUsd)),
            ]
        } ?? []
        let incidentFeed = topIncidents.map { inc in
            DashFeedItem(
                id: inc.id, color: Theme.severityColor(inc.severity),
                title: inc.kind.replacingOccurrences(of: "_", with: " "),
                sub: "\(inc.runId ?? inc.agentId ?? "fleet") \u{00B7} \(inc.severity)",
                value: String(inc.occurrences), valueColor: Theme.severityColor(inc.severity),
                onTap: inc.agentId.map { aid in { onOpenAgent(aid) } })
        }

        return VStack(spacing: 16) {
            HeroBand {
                HeroCard(
                    cap: "AI spend \u{00B7} rolling window",
                    value: Dash.usd0(spent),
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
                    KpiTile(label: "active runs", value: Dash.int(Int(overview.activeRuns)), sub: "\(overview.killedRuns) killed of \(Dash.int(Int(overview.totalRuns)))")
                    KpiTile(label: "governed saved", value: MoneyFormat.usd(saved), sub: "blocked \u{00B7} cache \u{00B7} router", tone: Theme.mint)
                    KpiTile(label: "open incidents", value: String(overview.openIncidents), sub: "\(overview.totalIncidents) total detected", tone: overview.openIncidents > 0 ? Theme.coral : nil)
                    KpiTile(label: "model calls", value: Dash.int(Int(overview.totalCalls)), sub: "across the fleet")
                }
            }

            DashMain {
                DashSection(title: "Spend by agent", right: "top \(min(8, agents.count)) of \(agents.count)") {
                    DashBars(items: agentBars, empty: "no agent spend yet")
                }
            } rail: {
                if !compItems.isEmpty {
                    DashSection(title: "Governed savings", right: "\(model.savings?.budgetBreaks ?? 0) budget breaks") {
                        DashComposition(items: compItems)
                    }
                }
                DashSection(title: "Incidents", right: "worst first") {
                    DashFeed(items: incidentFeed, empty: "no incidents")
                }
            }
        }
    }
}
