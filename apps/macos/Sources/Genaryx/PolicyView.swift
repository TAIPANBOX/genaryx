import GenaryxCoreFFI
import SwiftUI

/// The Policy panel: a live Decision Stream (filtered bus events), the
/// Approvals Inbox (pending queue + decided history, Grant/Deny gated
/// behind Touch ID), and a read-only Policy view. Fed by `PolicyModel` (the
/// Wardryx reads/decisions) plus the app's own `FleetModel` bus events - the
/// Decision Stream is a FILTER over the same live tail the Bus Explorer
/// renders, never a new REST read (PHASE2.md) - at parity with the Tauri
/// shell's own Policy panel (`src-tauri/src/policy/*` + its React panel).
///
/// Phase-2 wave 3 adds `notifications` + `focusedApprovalId`: when an
/// `ApprovalNotificationModel` notification response deep-links back in
/// (PHASE2.md: "DEEP-LINKS into the Approvals Inbox focused on that
/// approval_id"), `GenaryxApp` hands the target id down here, and this view
/// scrolls the Approvals Inbox to that row and highlights it - never a
/// decision by itself; the operator still has to press the existing
/// Touch-ID-gated Grant/Deny on that row (see `ApprovalsInboxSection`).
@MainActor
struct PolicyView: View {
    let model: PolicyModel
    /// The app-wide bus feed (`FleetModel.events`), filtered below to
    /// `source == "wardryx"` for the Decision Stream - see the module doc.
    let busEvents: [UiEvent]
    /// Owns the mute set an operator can toggle per pending approval row
    /// below (PHASE2.md Wave 3: "Mute: per agent / per run / per
    /// environment"). Never used to decide anything - see
    /// `ApprovalNotificationModel`'s own doc for why it cannot be.
    let notifications: ApprovalNotificationModel
    /// The approval id to scroll to and highlight in the Approvals Inbox,
    /// set by `GenaryxApp` from a notification deep link; `nil` in the
    /// ordinary case of the operator just having clicked the Policy tab
    /// themselves.
    let focusedApprovalId: String?

    private static let refreshInterval: Duration = .seconds(20)

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                PolicyEmptyStateView(connection: model.connection)
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
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    environmentChip

                    if let notice = model.mutationNotice {
                        noticeBar(notice)
                    }
                    if let bannerMessage = model.bannerMessage {
                        ErrorBannerView(message: bannerMessage)
                    }
                    if let lastGrant = model.lastGrant {
                        GrantTokenCard(outcome: lastGrant, onDismiss: { model.dismissLastGrant() })
                    }

                    section(title: "Decision Stream") {
                        DecisionStreamSection(events: wardryxEvents)
                    }
                    section(title: "Approvals Inbox") {
                        ApprovalsInboxSection(
                            approvals: model.approvals,
                            focusedApprovalId: focusedApprovalId,
                            onDecide: { id, verdict in await model.decide(id, verdict: verdict) },
                            isMuted: { agentId, runId in
                                notifications.isMuted(agentId: agentId, runId: runId, environment: environmentLabel)
                            },
                            onToggleMute: { agentId, runId in
                                let environment = environmentLabel
                                let alreadyMuted = notifications.isMuted(
                                    agentId: agentId, runId: runId, environment: environment)
                                if alreadyMuted {
                                    notifications.unmute(agentId: agentId, runId: runId, environment: environment)
                                } else {
                                    notifications.mute(agentId: agentId, runId: runId, environment: environment)
                                }
                            }
                        )
                    }
                    section(title: "Policies") {
                        PolicyListSection(policies: model.policies, policyVersion: model.latestPolicyVersion)
                    }
                }
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: focusedApprovalId) { _, newValue in
                guard let newValue else { return }
                withAnimation {
                    proxy.scrollTo(newValue, anchor: .center)
                }
            }
        }
    }

    /// PHASE2.md Wave 3's mute key third component - the same
    /// `wardryxUrl` the environment chip below already shows the operator.
    private var environmentLabel: String {
        ApprovalNotificationModel.environmentLabel(for: model.connection)
    }

    /// PHASE2.md: "a live, filtered view of the shared bus where
    /// `source == "wardryx"`" - reuses the existing event pipeline (the same
    /// live tail the Bus Explorer renders), never a new REST read.
    private var wardryxEvents: [UiEvent] {
        busEvents.filter { $0.source.lowercased() == "wardryx" }
    }

    @ViewBuilder
    private var environmentChip: some View {
        // Defensive-only: `body` already gates `content` (and therefore
        // this chip) on `model.connection.isReady` - same convention
        // `OverviewView.environmentChip` documents for its own unreachable
        // non-`.ready` branch.
        if case .ready(let source, let wardryxUrl, _) = model.connection {
            HStack(spacing: 6) {
                Circle().fill(Theme.sourceColor("wardryx")).frame(width: 6, height: 6)
                Text("\(sourceLabel(source)) \u{00B7} \(wardryxUrl)")
                    .font(Theme.mono(11, weight: .medium))
                    .foregroundStyle(Theme.textSecondary)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.panelElevated))
            .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
        }
    }

    private func sourceLabel(_ source: WardryxEnvSource) -> String {
        switch source {
        case .taipan(let name):
            "taipan up \u{00B7} \(name)"
        case .envFallback:
            "env fallback"
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

// MARK: - DecisionStreamSection

/// PHASE2.md row shape: `ts`, a type badge (allow=info / deny=high /
/// hold=medium), `agent_id`, `data.reason`, `data.tool_names`.
@MainActor
private struct DecisionStreamSection: View {
    let events: [UiEvent]

    /// Bounded preview, mirroring `MenuBarBusView.previewCount`'s "recent
    /// activity feed, not a full archive" rule - `FleetModel` already caps
    /// the underlying feed at 500 events, but this panel renders its rows
    /// in a plain (non-lazy) `VStack`, matching `RunsTable`'s own shape, so
    /// it should not non-lazily render hundreds of rows at once either.
    private static let displayLimit = 60

    var body: some View {
        if events.isEmpty {
            Text("no wardryx bus activity yet.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 12)
        } else {
            let shown = Array(events.prefix(Self.displayLimit))
            VStack(spacing: 0) {
                ForEach(Array(shown.enumerated()), id: \.element.rowKey) { index, event in
                    DecisionStreamRow(event: event)
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

    private struct DecisionStreamRow: View {
        let event: UiEvent

        var body: some View {
            let fields = event.wardryxFields
            HStack(spacing: 12) {
                typeBadge

                Text(event.eventType)
                    .font(Theme.mono(11.5, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(width: 150, alignment: .leading)

                Text(event.agentId)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.head)
                    .frame(width: 210, alignment: .leading)

                VStack(alignment: .leading, spacing: 2) {
                    if let reason = fields.reason, !reason.isEmpty {
                        Text(reason)
                            .font(Theme.mono(11))
                            .foregroundStyle(Theme.textSecondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                    if !fields.toolNames.isEmpty {
                        Text(fields.toolNames.joined(separator: ", "))
                            .font(Theme.mono(10))
                            .foregroundStyle(Theme.textTertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Text(MoneyFormat.timestamp(event.ts))
                    .font(Theme.mono(11))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
                    .frame(width: 118, alignment: .trailing)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        }

        /// `event.severity` is genuinely optional (the envelope field is
        /// optional) - a nil-tolerant badge built directly off
        /// `Theme.severityColor`/`Theme.severityLabel` (both already accept
        /// `String?`), rather than `MoneyComponents.swift`'s `SeverityPill`
        /// (whose `severity` parameter is non-optional, since every DTO that
        /// currently feeds it already guarantees a value). Mirrors
        /// `BusExplorerView.swift`'s own private `SeverityBadge` shape - a
        /// second small copy for the same reason that one gives for not
        /// sharing across panels.
        private var typeBadge: some View {
            let color = Theme.severityColor(event.severity)
            return HStack(spacing: 6) {
                Circle()
                    .fill(color)
                    .frame(width: 7, height: 7)
                    .shadow(color: color.opacity(0.6), radius: 3)
                Text(Theme.severityLabel(event.severity))
                    .font(Theme.mono(10, weight: .semibold))
                    .tracking(0.8)
            }
            .foregroundStyle(Theme.textSecondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(color.opacity(0.14)))
            .overlay(Capsule().strokeBorder(color.opacity(0.4), lineWidth: 1))
        }
    }
}

// MARK: - ApprovalsInboxSection

/// PHASE2.md: the pending queue (full context: who/what/cost/why/chain, each
/// with Grant/Deny) plus a decided history list (`decision`, `decided_by`,
/// `decided_at`).
@MainActor
private struct ApprovalsInboxSection: View {
    let approvals: [ApprovalRecord]
    /// Wave 3: the row to scroll to and highlight - see `PolicyView`'s own
    /// doc comment.
    let focusedApprovalId: String?
    let onDecide: (String, ApprovalVerdict) async -> Void
    /// Wave 3 mute affordance (PHASE2.md: "per agent / per run / per
    /// environment") - keyed by `(agentId, runId)`, `PolicyView` already
    /// closes over the third key component (the environment).
    let isMuted: (String, String) -> Bool
    let onToggleMute: (String, String) -> Void

    private static let decidedDisplayLimit = 20

    private var pending: [ApprovalRecord] {
        approvals.filter(\.pending)
    }

    private var decided: [ApprovalRecord] {
        approvals
            .filter { !$0.pending }
            .sorted { ($0.decidedAt ?? $0.requestedAt) > ($1.decidedAt ?? $1.requestedAt) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if pending.isEmpty {
                Text("no pending approvals.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            } else {
                VStack(spacing: 8) {
                    ForEach(pending, id: \.approvalId) { approval in
                        ApprovalRow(
                            approval: approval,
                            isFocused: approval.approvalId == focusedApprovalId,
                            onDecide: onDecide,
                            isMuted: isMuted(approval.agentId, approval.runId),
                            onToggleMute: { onToggleMute(approval.agentId, approval.runId) }
                        )
                        .id(approval.approvalId)
                    }
                }
            }

            if !decided.isEmpty {
                decidedHistory
            }
        }
    }

    private var decidedHistory: some View {
        let shown = Array(decided.prefix(Self.decidedDisplayLimit))
        return VStack(alignment: .leading, spacing: 6) {
            Text("DECIDED")
                .font(Theme.mono(10, weight: .semibold))
                .tracking(1.0)
                .foregroundStyle(Theme.textTertiary)
            VStack(spacing: 0) {
                ForEach(Array(shown.enumerated()), id: \.element.approvalId) { index, approval in
                    DecidedApprovalRow(approval: approval)
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

    /// One pending hold: who (`agentId` + the `onBehalfOf` chain), what
    /// (`tools`), how much (`estCostUsd`), why (`reason`), when
    /// (`requestedAt`), `policyVersion` - plus Grant/Deny, each reusing
    /// `ConfirmButton`'s existing two-step arm-then-fire ceremony
    /// (`MoneyComponents.swift`) with the actual hardware gate layered
    /// inside `onConfirm`: `PolicyModel.decide` ALWAYS challenges Touch ID
    /// before calling into `WardryxHandle`, regardless of which UI entry
    /// point reaches it - including a notification's Review/Approve/Deny
    /// tap, which only ever sets `isFocused` via `PolicyView`'s
    /// `focusedApprovalId` (Wave 3) and never calls `onDecide` itself.
    private struct ApprovalRow: View {
        let approval: ApprovalRecord
        let isFocused: Bool
        let onDecide: (String, ApprovalVerdict) async -> Void
        let isMuted: Bool
        let onToggleMute: () -> Void

        var body: some View {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top, spacing: 10) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(approval.agentId)
                            .font(Theme.mono(12, weight: .medium))
                            .foregroundStyle(Theme.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.head)
                        Text(chainText)
                            .font(Theme.mono(10.5))
                            .foregroundStyle(Theme.textTertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                    Spacer(minLength: 8)
                    muteButton
                    Text(MoneyFormat.usd(approval.estCostUsd ?? 0))
                        .font(Theme.mono(13, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(Theme.amber)
                }

                if !approval.tools.isEmpty {
                    Text("tools: \(approval.tools.joined(separator: ", "))")
                        .font(Theme.mono(11))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                if let reason = approval.reason, !reason.isEmpty {
                    Text(reason)
                        .font(Theme.mono(11))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(2)
                }

                HStack(spacing: 10) {
                    Text(MoneyFormat.timestamp(approval.requestedAt))
                        .font(Theme.mono(10.5))
                        .monospacedDigit()
                        .foregroundStyle(Theme.textTertiary)
                    if let policyVersion = approval.policyVersion {
                        Text("policy \(policyVersion)")
                            .font(Theme.mono(10.5))
                            .foregroundStyle(Theme.textTertiary)
                    }
                    Spacer(minLength: 8)
                    ConfirmButton(
                        label: "Deny",
                        confirmLabel: "Touch ID to deny",
                        tone: Theme.ember,
                        onConfirm: { await onDecide(approval.approvalId, .deny) }
                    )
                    ConfirmButton(
                        label: "Grant",
                        confirmLabel: "Touch ID to grant",
                        tone: Theme.mint,
                        onConfirm: { await onDecide(approval.approvalId, .grant) }
                    )
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(isFocused ? Theme.amber.opacity(0.1) : Theme.panelElevated)
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .strokeBorder(isFocused ? Theme.amber.opacity(0.7) : Color.clear, lineWidth: 1.5)
            )
        }

        private var chainText: String {
            approval.onBehalfOf.isEmpty
                ? "run \(approval.runId)"
                : "run \(approval.runId) \u{00B7} on behalf of \(approval.onBehalfOf.joined(separator: " \u{2192} "))"
        }

        /// A lightweight, non-privileged toggle (no `ConfirmButton`
        /// ceremony, no Touch ID - muting is a local UI preference, never a
        /// mutation against Wardryx) for PHASE2.md Wave 3's "Mute: per
        /// agent / per run / per environment".
        private var muteButton: some View {
            Button(action: onToggleMute) {
                Image(systemName: isMuted ? "bell.slash.fill" : "bell.slash")
                    .font(.system(size: 11))
                    .foregroundStyle(isMuted ? Theme.amber : Theme.textTertiary)
            }
            .buttonStyle(.plain)
            .help(muteHelpText)
        }

        private var muteHelpText: String {
            let verb = isMuted ? "Unmute" : "Mute"
            return "\(verb) notifications for \(approval.agentId) / \(approval.runId)"
        }
    }

    /// One decided approval: `decision`, `decidedBy`, `decidedAt`.
    private struct DecidedApprovalRow: View {
        let approval: ApprovalRecord

        var body: some View {
            HStack(spacing: 12) {
                decisionPill
                VStack(alignment: .leading, spacing: 2) {
                    Text(approval.agentId)
                        .font(Theme.mono(11.5))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.head)
                    Text(
                        "by \(approval.decidedBy ?? "unknown") \u{00B7} \(MoneyFormat.timestamp(approval.decidedAt ?? approval.requestedAt))"
                    )
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .lineLimit(1)
                }
                Spacer(minLength: 8)
                Text(MoneyFormat.usd(approval.estCostUsd ?? 0))
                    .font(Theme.mono(12))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        }

        private var decisionPill: some View {
            let granted = approval.decision == "grant"
            let tone = granted ? Theme.mint : Theme.ember
            return Text(granted ? "granted" : "denied")
                .font(Theme.mono(10, weight: .semibold))
                .foregroundStyle(tone)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(Capsule().fill(tone.opacity(0.14)))
                .overlay(Capsule().strokeBorder(tone.opacity(0.4), lineWidth: 1))
        }
    }
}

// MARK: - PolicyListSection

/// PHASE2.md: `id`, `target`, `deny_tool`, `allow_domains`,
/// `require_human_above_usd`, `deny_above_usd`, `max_steps`,
/// `deny_if_unattested`, plus the set-level `policy_version` (composed by
/// `PolicyModel.latestPolicyVersion` - see that property's own doc comment
/// for why it is derived here rather than carried on `PolicyRecord`).
/// Read-only in this wave (PHASE2.md: "the guarded PUT/DELETE editor is v1").
@MainActor
private struct PolicyListSection: View {
    let policies: [PolicyRecord]
    let policyVersion: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let policyVersion {
                Text("policy_version \u{00B7} \(policyVersion)")
                    .font(Theme.mono(10.5, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
            }

            if policies.isEmpty {
                Text("no policies configured.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            } else {
                VStack(spacing: 8) {
                    ForEach(policies, id: \.id) { policy in
                        PolicyListRow(policy: policy)
                    }
                }
            }
        }
    }

    private struct PolicyListRow: View {
        let policy: PolicyRecord

        var body: some View {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(policy.id)
                        .font(Theme.mono(12, weight: .semibold))
                        .foregroundStyle(Theme.textPrimary)
                    Spacer(minLength: 8)
                    if let updatedAt = policy.updatedAt {
                        Text(MoneyFormat.timestamp(updatedAt))
                            .font(Theme.mono(10.5))
                            .foregroundStyle(Theme.textTertiary)
                    }
                }
                Text(policy.target)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)

                if !tags.isEmpty {
                    HStack(spacing: 6) {
                        ForEach(Array(tags.enumerated()), id: \.offset) { _, tag in
                            tagPill(tag.0, tone: tag.1)
                        }
                    }
                }

                if !policy.denyTool.isEmpty {
                    Text("deny tools: \(policy.denyTool.joined(separator: ", "))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                }
                if !policy.allowDomains.isEmpty {
                    Text("allow domains: \(policy.allowDomains.joined(separator: ", "))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
        }

        private var tags: [(String, Color)] {
            var tags: [(String, Color)] = []
            if policy.requireHumanAboveUsd > 0 {
                tags.append(("human > \(MoneyFormat.usd(policy.requireHumanAboveUsd))", Theme.amber))
            }
            if policy.denyAboveUsd > 0 {
                tags.append(("deny > \(MoneyFormat.usd(policy.denyAboveUsd))", Theme.ember))
            }
            if policy.maxSteps > 0 {
                tags.append(("max \(policy.maxSteps) steps", Theme.steel))
            }
            if policy.denyIfUnattested {
                tags.append(("unattested denied", Theme.coral))
            }
            return tags
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
}
