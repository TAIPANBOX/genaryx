import GenaryxCoreFFI
import SwiftUI

/// Agent 360: one agent's cross-plane card, assembled from every plane the
/// shell can readily reach (PHASE3.md position 4 / the Ф3 exit gate: "a
/// click on any agent, from anywhere, opens a full cross-plane Agent 360
/// card"). Delegation + Events are freshly read through `Agent360Model`
/// (`FleetHandle.agentSlice`/`eventsForAgent`); Identity/Money/Policy are
/// each a plain filter over `identityModel`/`cloudModel`/`policyModel`/
/// `fleetModel` state the shell already holds live - never a second read of
/// those three planes (mirrors `PolicyView.wardryxEvents`'s own "a FILTER
/// over the same live tail... never a new REST read").
///
/// Presented as a sheet from `GenaryxApp` (`agentFocus`), reachable from a
/// delegation-graph node tap (`DelegationGraphView`) and an Identity row tap
/// (`IdentityView`) alike - PHASE3.md's parity checklist: "a click on an
/// agent from any panel / menu-bar / notification opens its 360 card."
/// Money/Policy mutations (kill a run, grant/deny an approval) are NOT
/// re-implemented here: each section instead links back to the panel that
/// already owns that privileged, Touch-ID-gated ceremony
/// (`onOpenMoney`/`onOpenPolicy`), so there is exactly one place in the app
/// that can perform them.
@MainActor
struct Agent360View: View {
    let agentId: String
    let fleetModel: FleetModel
    let cloudModel: CloudModel
    let policyModel: PolicyModel
    let identityModel: IdentityModel
    let onClose: () -> Void
    let onOpenMoney: () -> Void
    let onOpenPolicy: () -> Void
    /// PHASE3 W4: opens Run Replay (`GenaryxApp.openReplay`) focused on one
    /// of this agent's runs - the "Agent 360" entry point PHASE3.md's Run
    /// Replay brief names, wired down to `Agent360MoneySection`'s own run
    /// rows below.
    let onOpenReplay: (String) -> Void

    @State private var model: Agent360Model

    init(
        agentId: String, fleetModel: FleetModel, cloudModel: CloudModel, policyModel: PolicyModel,
        identityModel: IdentityModel, onClose: @escaping () -> Void, onOpenMoney: @escaping () -> Void,
        onOpenPolicy: @escaping () -> Void, onOpenReplay: @escaping (String) -> Void
    ) {
        self.agentId = agentId
        self.fleetModel = fleetModel
        self.cloudModel = cloudModel
        self.policyModel = policyModel
        self.identityModel = identityModel
        self.onClose = onClose
        self.onOpenMoney = onOpenMoney
        self.onOpenPolicy = onOpenPolicy
        self.onOpenReplay = onOpenReplay
        _model = State(initialValue: Agent360Model(agentId: agentId))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider().overlay(Theme.hairline)
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    if let bannerMessage = model.bannerMessage {
                        ErrorBannerView(message: bannerMessage)
                    }
                    section(title: "Delegation") {
                        DelegationSection(slice: model.slice, isLoading: model.isLoading)
                    }
                    section(title: "Events") {
                        Agent360EventsSection(events: model.events, isLoading: model.isLoading)
                    }
                    section(title: "Identity") {
                        Agent360IdentitySection(
                            identity: identity, alerts: identityAlerts,
                            connectionReady: identityModel.connection.isReady)
                    }
                    section(title: "Money") {
                        Agent360MoneySection(
                            runs: agentRuns, incidents: agentIncidents, connectionReady: cloudModel.connection.isReady,
                            onOpenMoney: onOpenMoney, onOpenReplay: onOpenReplay)
                    }
                    section(title: "Policy") {
                        Agent360PolicySection(
                            events: policyEvents, approvals: agentApprovals,
                            connectionReady: policyModel.connection.isReady, onOpenPolicy: onOpenPolicy)
                    }
                }
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(minWidth: 560, idealWidth: 640, minHeight: 520, idealHeight: 700)
        .background(Theme.background)
        .task(id: agentId) {
            await model.refresh(fleet: fleetModel)
        }
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text("AGENT 360")
                    .font(Theme.mono(10, weight: .semibold))
                    .tracking(1.4)
                    .foregroundStyle(Theme.textTertiary)
                Text(agentId)
                    .font(Theme.mono(13, weight: .semibold))
                    .foregroundStyle(Theme.textPrimary)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 8)
            Button("Close", action: onClose)
                .buttonStyle(.plain)
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.textSecondary)
        }
        .padding(16)
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

    // MARK: - Identity/Money/Policy: plain filters over already-live state
    // (no new FFI read - see the type doc's second paragraph).

    private var identity: IdentityRecord? {
        identityModel.identities.first { $0.id == agentId }
    }

    private var identityAlerts: [AlertRecord] {
        identityModel.alerts.filter { $0.identity == agentId }
    }

    private var agentRuns: [Run] {
        cloudModel.runs.filter { $0.agentId == agentId }
    }

    private var agentIncidents: [Incident] {
        cloudModel.incidents.filter { $0.agentId == agentId }
    }

    /// PHASE3.md W3 brief: "this agent's recent wardryx.* bus decisions
    /// (from the live event stream the app already holds)" - the exact same
    /// `fleetModel.events` feed `PolicyView.wardryxEvents` filters, narrowed
    /// one step further to this one agent.
    private var policyEvents: [UiEvent] {
        fleetModel.events.filter { $0.source.lowercased() == "wardryx" && $0.agentId == agentId }
    }

    private var agentApprovals: [ApprovalRecord] {
        policyModel.approvals.filter { $0.agentId == agentId }
    }
}

// MARK: - DelegationSection

@MainActor
private struct DelegationSection: View {
    let slice: AgentSliceRecord?
    let isLoading: Bool

    var body: some View {
        if let slice {
            VStack(alignment: .leading, spacing: 10) {
                if let node = slice.node {
                    HStack(spacing: 10) {
                        NodeKindBadge(kind: node.kind)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("\(node.eventCount) event\(node.eventCount == 1 ? "" : "s") on the bus")
                                .font(Theme.mono(11.5))
                                .foregroundStyle(Theme.textPrimary)
                            Text("last acted \(node.lastTs.isEmpty ? "never" : MoneyFormat.timestamp(node.lastTs))")
                                .font(Theme.mono(10.5))
                                .foregroundStyle(Theme.textTertiary)
                        }
                        Spacer(minLength: 0)
                    }
                } else {
                    Text("this agent has not acted on the bus (delegation-chain mention only, or unseen).")
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textTertiary)
                }
                neighborRow(title: "delegates from (parents)", nodes: slice.parents)
                neighborRow(title: "delegates to (children)", nodes: slice.children)
                if slice.node != nil && slice.parents.isEmpty && slice.children.isEmpty {
                    Text("no delegation edges - this agent neither delegates nor is delegated to.")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
        } else if isLoading {
            Text("loading the delegation neighborhood...")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
        } else {
            Text("no delegation data available.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
        }
    }

    @ViewBuilder
    private func neighborRow(title: String, nodes: [GraphNodeRecord]) -> some View {
        if !nodes.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(title.uppercased())
                    .font(Theme.mono(9.5, weight: .semibold))
                    .tracking(0.6)
                    .foregroundStyle(Theme.textTertiary)
                Text(nodes.map(\.id).joined(separator: ", "))
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A tiny colored pill for one `NodeKind` - mirrors `DelegationGraphView`'s
/// own `color(for:)` choice (`.user` amber, `.agent` iris, `.other` steel)
/// so the graph and the 360 card read as the same color language.
@MainActor
private struct NodeKindBadge: View {
    let kind: NodeKind

    var body: some View {
        Text(label)
            .font(Theme.mono(9, weight: .semibold))
            .foregroundStyle(tone)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(Capsule().fill(tone.opacity(0.14)))
            .overlay(Capsule().strokeBorder(tone.opacity(0.4), lineWidth: 1))
    }

    private var label: String {
        switch kind {
        case .user: return "user"
        case .agent: return "agent"
        case .other: return "other"
        }
    }

    private var tone: Color {
        switch kind {
        case .user: return Theme.amber
        case .agent: return Theme.iris
        case .other: return Theme.steel
        }
    }
}

// MARK: - Agent360EventsSection

@MainActor
private struct Agent360EventsSection: View {
    let events: [UiEvent]
    let isLoading: Bool

    var body: some View {
        if events.isEmpty {
            Text(isLoading ? "loading events..." : "no events for this agent yet.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 4)
        } else {
            VStack(spacing: 0) {
                ForEach(Array(events.enumerated()), id: \.element.rowKey) { index, event in
                    Agent360EventRow(event: event)
                    if index < events.count - 1 {
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

    private struct Agent360EventRow: View {
        let event: UiEvent

        var body: some View {
            HStack(spacing: 10) {
                Circle()
                    .fill(Theme.severityColor(event.severity))
                    .frame(width: 6, height: 6)
                Text(event.source)
                    .font(Theme.mono(10, weight: .semibold))
                    .foregroundStyle(Theme.sourceColor(event.source))
                    .frame(width: 66, alignment: .leading)
                Text(event.eventType)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text(MoneyFormat.timestamp(event.ts))
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
        }
    }
}

// MARK: - Agent360IdentitySection

@MainActor
private struct Agent360IdentitySection: View {
    let identity: IdentityRecord?
    let alerts: [AlertRecord]
    let connectionReady: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let identity {
                HStack(spacing: 8) {
                    tagPill(identity.identityType, tone: Theme.iris)
                    if identity.privileged {
                        tagPill("privileged", tone: Theme.amber)
                    }
                    Text("\(identity.permissionCount) permission\(identity.permissionCount == 1 ? "" : "s")")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                    Spacer(minLength: 0)
                }
            } else {
                Text(connectionReady ? "no Idryx identity record for this agent." : "no Idryx identity plane connected.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            }

            if alerts.isEmpty {
                Text("no identity alerts for this agent.")
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textTertiary)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(alerts.enumerated()), id: \.offset) { index, alert in
                        alertRow(alert)
                        if index < alerts.count - 1 {
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
    }

    private func alertRow(_ alert: AlertRecord) -> some View {
        HStack(spacing: 10) {
            SeverityPill(severity: alert.severity)
            Text(alert.detector)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 8)
            Text(MoneyFormat.timestamp(alert.time))
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textTertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private func tagPill(_ text: String, tone: Color) -> some View {
        Text(text)
            .font(Theme.mono(10, weight: .semibold))
            .foregroundStyle(tone)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(tone.opacity(0.14)))
            .overlay(Capsule().strokeBorder(tone.opacity(0.4), lineWidth: 1))
    }
}

// MARK: - Agent360MoneySection

@MainActor
private struct Agent360MoneySection: View {
    let runs: [Run]
    let incidents: [Incident]
    let connectionReady: Bool
    let onOpenMoney: () -> Void
    /// PHASE3 W4: see `Agent360View`'s own doc comment on `onOpenReplay`.
    let onOpenReplay: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if runs.isEmpty && incidents.isEmpty {
                Text(connectionReady ? "no runs or incidents for this agent." : "no Cloud environment connected.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            } else {
                if !runs.isEmpty {
                    HStack(spacing: 14) {
                        StatTile(label: "spent", value: MoneyFormat.usd(totalSpent))
                        StatTile(label: "runs", value: "\(runs.count)")
                    }
                    VStack(spacing: 6) {
                        ForEach(runs, id: \.runId) { run in
                            runRow(run)
                        }
                    }
                }
                if !incidents.isEmpty {
                    VStack(spacing: 6) {
                        ForEach(incidents, id: \.id) { incident in
                            incidentRow(incident)
                        }
                    }
                }
            }
            Agent360LinkButton(label: "Open in Money panel", action: onOpenMoney)
        }
    }

    private var totalSpent: Double {
        runs.reduce(0) { $0 + $1.spentUsd }
    }

    private func runRow(_ run: Run) -> some View {
        HStack(spacing: 10) {
            Text(run.runId)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textSecondary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 8)
            if run.killed {
                Text("killed")
                    .font(Theme.mono(9, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
            }
            Text(MoneyFormat.usd(run.spentUsd))
                .font(Theme.mono(11))
                .monospacedDigit()
                .foregroundStyle(Theme.textPrimary)
            Button {
                onOpenReplay(run.runId)
            } label: {
                Image(systemName: "play.circle")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.iris)
            }
            .buttonStyle(.plain)
            .help("Replay this run")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).fill(Theme.panelElevated))
    }

    private func incidentRow(_ incident: Incident) -> some View {
        HStack(spacing: 10) {
            SeverityPill(severity: incident.severity)
            Text(incident.kind)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            if incident.acknowledged {
                Text("acked")
                    .font(Theme.mono(9, weight: .semibold))
                    .foregroundStyle(Theme.mint)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).fill(Theme.panelElevated))
    }
}

// MARK: - Agent360PolicySection

@MainActor
private struct Agent360PolicySection: View {
    let events: [UiEvent]
    let approvals: [ApprovalRecord]
    let connectionReady: Bool
    let onOpenPolicy: () -> Void

    private static let eventsDisplayLimit = 20

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if events.isEmpty && approvals.isEmpty {
                Text(connectionReady ? "no wardryx activity for this agent." : "no Wardryx policy plane connected.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            } else {
                if !approvals.isEmpty {
                    VStack(spacing: 6) {
                        ForEach(approvals, id: \.approvalId) { approval in
                            approvalRow(approval)
                        }
                    }
                }
                if !events.isEmpty {
                    VStack(spacing: 0) {
                        let shown = Array(events.prefix(Self.eventsDisplayLimit))
                        ForEach(Array(shown.enumerated()), id: \.offset) { index, event in
                            decisionRow(event)
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
            Agent360LinkButton(label: "Open in Policy panel", action: onOpenPolicy)
        }
    }

    private func decisionRow(_ event: UiEvent) -> some View {
        HStack(spacing: 10) {
            Circle().fill(Theme.severityColor(event.severity)).frame(width: 6, height: 6)
            Text(event.eventType)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 8)
            Text(MoneyFormat.timestamp(event.ts))
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textTertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private func approvalRow(_ approval: ApprovalRecord) -> some View {
        HStack(spacing: 10) {
            Text(approval.pending ? "pending" : (approval.decision ?? "decided"))
                .font(Theme.mono(9, weight: .semibold))
                .foregroundStyle(approval.pending ? Theme.amber : (approval.decision == "grant" ? Theme.mint : Theme.ember))
                .padding(.horizontal, 7)
                .padding(.vertical, 3)
                .background(Capsule().fill((approval.pending ? Theme.amber : Theme.mint).opacity(0.12)))
            Text(approval.runId)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textSecondary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 8)
            Text(MoneyFormat.usd(approval.estCostUsd ?? 0))
                .font(Theme.mono(11))
                .monospacedDigit()
                .foregroundStyle(Theme.textPrimary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).fill(Theme.panelElevated))
    }
}

// MARK: - Agent360LinkButton

/// PHASE3 W3 brief: "Mutations (kill/approve) may LINK to the existing
/// panels rather than re-implement here." A plain link-styled button
/// (never a mutation itself) that switches `GenaryxApp`'s tab and dismisses
/// this sheet - see `onOpenMoney`/`onOpenPolicy` on `Agent360View`.
@MainActor
private struct Agent360LinkButton: View {
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Text(label)
                Image(systemName: "arrow.up.right")
                    .font(.system(size: 9, weight: .bold))
            }
            .font(Theme.mono(10.5, weight: .semibold))
            .foregroundStyle(Theme.iris)
        }
        .buttonStyle(.plain)
    }
}
