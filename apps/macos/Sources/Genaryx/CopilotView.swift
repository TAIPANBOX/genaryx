import GenaryxCoreFFI
import SwiftUI

/// The Copilot panel (docs/PHASE6.md C0-W2, D13 Felyx): a residency banner
/// naming exactly where inference runs (or why it doesn't yet - "local: ...
/// via Ollama" vs "remote: ..., BYO key" vs "no provider configured"), a
/// scrollable conversation with a per-answer "tools used" disclosure (the
/// anti-hallucination evidence surface: tool-computed numbers, shown
/// verbatim, never the model's own arithmetic), and a question field. Fed
/// entirely by `CopilotModel` (the `CopilotHandle` status/ask calls).
///
/// Unlike every other panel, this one is a conversation, not a dashboard of
/// server-owned data - so `DashKit`'s hero/section chrome wraps the residency
/// summary and the message list rather than a KPI grid of fleet numbers,
/// matching the OTHER panels' look (Theme fonts/colors, `dashCard` chrome,
/// `HeroBand`/`DashSection`/`DashMain`/`KpiTile`) without pretending this is
/// the same kind of read-only snapshot they are.
@MainActor
struct CopilotView: View {
    let model: CopilotModel

    /// C2 "Approve" routing (docs/PHASE6-C2.md): each closure routes
    /// straight into the EXACT existing signed ceremony the Money/Policy/
    /// Identity panels themselves already use - `CopilotView` owns none of
    /// those models itself, mirroring how `MoneyView.onExplainIncident`
    /// reaches INTO `CopilotModel` via a closure `GenaryxApp` wires, just in
    /// the opposite direction (see `GenaryxApp`'s own `approveCopilot*`
    /// doc comments for exactly which model method each one calls).
    /// Kill/Budget report back through `CloudModel.killRun`/`setBudget`
    /// (break-glass: a mandatory typed reason plus Touch ID, both already
    /// gated inside those methods - this view only COLLECTS the reason,
    /// via the same shared `BreakGlassPanel` every other break-glass action
    /// in this app uses); GrantDeny through `PolicyModel.decide` (Touch ID,
    /// no reason); Rescan through `IdentityModel.rescan` (no gate at all -
    /// see that model's own doc). Every closure reports whether the
    /// underlying mutation actually went through, so a card can show its
    /// outcome instead of guessing. Defaulted to an always-false no-op so
    /// every other `CopilotView` call site (none exist as of C2, but this
    /// mirrors `onExplainIncident`'s own default) keeps compiling.
    var onApproveKill: (_ runId: String, _ reason: String) async -> Bool = { _, _ in false }
    var onApproveBudget: (_ runId: String, _ usdCap: Double, _ reason: String) async -> Bool = { _, _, _ in false }
    var onApproveGrantDeny: (_ approvalId: String, _ verdict: ApprovalVerdict) async -> Bool = { _, _ in false }
    var onApproveRescan: () async -> Bool = { false }

    @State private var draft: String = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                residencyBanner

                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                dashboard
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
    }

    // MARK: - residency banner

    /// Green when inference never leaves this box, amber for a BYO-key
    /// remote provider, muted when no provider is configured at all - the
    /// exact three states docs/PHASE6.md C0-W2 names for this banner.
    @ViewBuilder
    private var residencyBanner: some View {
        if let status = model.status {
            if status.enabled, status.local == true {
                residencyLine(
                    text: "Local: \(status.model ?? "unknown model") via \(status.provider ?? "unknown provider")",
                    color: Theme.mint)
            } else if status.enabled {
                residencyLine(text: "Remote: \(status.provider ?? "unknown provider") (BYO key)", color: Theme.amber)
            } else {
                residencyLine(
                    text: "No provider configured - \(status.disabledReason ?? "no reason given")",
                    color: Theme.textTertiary)
            }
        } else {
            residencyLine(text: "Connecting to Felyx...", color: Theme.textTertiary)
        }
    }

    private func residencyLine(text: String, color: Color) -> some View {
        HStack(spacing: 8) {
            Circle().fill(color).frame(width: 7, height: 7)
            Text(text)
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(color)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).fill(color.opacity(0.1)))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).strokeBorder(
                color.opacity(0.35), lineWidth: 1))
    }

    // MARK: - dashboard

    /// A Felyx status hero (online/offline + provider/model) beside a small
    /// KPI grid (messages, tools used, token usage this session), over one
    /// full-width Conversation section (message list + composer) - no rail
    /// content would be a fleet number here, so the rail instead carries a
    /// short "About Felyx" note on the read/propose/never-act model, the
    /// same safety story `crates/copilot/src/lib.rs`'s own doc comment
    /// tells.
    private var dashboard: some View {
        VStack(spacing: 16) {
            HeroBand {
                heroCard
            } tiles: {
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 14), GridItem(.flexible(), spacing: 14)], spacing: 14) {
                    KpiTile(label: "messages", value: Dash.int(model.messages.count))
                    KpiTile(
                        label: "tools used", value: Dash.int(totalToolCalls),
                        tone: totalToolCalls > 0 ? Theme.iris : nil)
                    KpiTile(label: "prompt tokens", value: Dash.int(Int(totalPromptTokens)))
                    KpiTile(label: "completion tokens", value: Dash.int(Int(totalCompletionTokens)))
                }
            }

            DashMain {
                DashSection(title: "Conversation", right: subtitle) {
                    VStack(spacing: 0) {
                        conversation
                        Divider().overlay(Theme.hairline)
                        composer
                    }
                }
            } rail: {
                DashSection(title: "About Felyx") {
                    aboutText
                }
            }
        }
    }

    private var heroCard: some View {
        HeroCard(cap: "Felyx \u{00B7} AI copilot", value: heroValue, sub: Text(heroSub))
    }

    private var heroValue: String {
        guard let status = model.status else { return "..." }
        return status.enabled ? "Online" : "Offline"
    }

    private var heroSub: String {
        guard let status = model.status else { return "connecting..." }
        guard status.enabled else { return status.disabledReason ?? "no provider configured" }
        return "\(status.provider ?? "?") \u{00B7} \(status.model ?? "?")"
    }

    private var subtitle: String {
        "\(model.messages.count) message\(model.messages.count == 1 ? "" : "s")"
    }

    private var totalToolCalls: Int {
        model.messages.reduce(0) { $0 + $1.toolTrace.count }
    }

    private var totalPromptTokens: UInt32 {
        model.messages.reduce(0) { $0 + $1.promptTokens }
    }

    private var totalCompletionTokens: UInt32 {
        model.messages.reduce(0) { $0 + $1.completionTokens }
    }

    private var aboutText: some View {
        Text(
            "Felyx reads your fleet through the same typed connectors this console already uses (money, policy, identity). It can look things up and recommend, but it can never act on its own: the copilot crate holds no signer, so any change still goes through the existing human-signed approval ceremony. Every number in an answer comes from a tool call, shown in \u{201C}tools used\u{201D} below the reply, never from the model doing arithmetic in prose."
        )
        .font(Theme.mono(10.5))
        .foregroundStyle(Theme.textTertiary)
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, 20)
        .padding(.top, 6)
        .padding(.bottom, 16)
    }

    // MARK: - conversation

    private var conversation: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    if model.messages.isEmpty {
                        Text(
                            "Ask Felyx about your fleet - spend, runs, policies, identities. Every number it gives you comes from a tool call, never from memory."
                        )
                        .font(Theme.mono(11.5))
                        .foregroundStyle(Theme.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                    } else {
                        ForEach(model.messages) { message in
                            MessageBubble(
                                message: message,
                                onApproveKill: onApproveKill,
                                onApproveBudget: onApproveBudget,
                                onApproveGrantDeny: onApproveGrantDeny,
                                onApproveRescan: onApproveRescan
                            )
                            .id(message.id)
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 14)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(minHeight: 280, maxHeight: 440)
            .onChange(of: model.messages.count) { _, _ in
                guard let last = model.messages.last else { return }
                withAnimation {
                    proxy.scrollTo(last.id, anchor: .bottom)
                }
            }
        }
    }

    // MARK: - composer

    private var composer: some View {
        HStack(spacing: 8) {
            TextField("Ask about spend, runs, policies, identities...", text: $draft)
                .textFieldStyle(.plain)
                .font(Theme.mono(11.5))
                .foregroundStyle(Theme.textPrimary)
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panelElevated))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))
                .onSubmit(sendDraft)

            sendButton
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
    }

    private var sendButton: some View {
        Button(action: sendDraft) {
            HStack(spacing: 5) {
                if model.isSending {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "paperplane.fill")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(model.isSending ? "Sending..." : "Send")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.iris)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(Capsule().fill(Theme.iris.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.iris.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isSending || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }

    private func sendDraft() {
        let text = draft
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        draft = ""
        Task { await model.send(text) }
    }
}

// MARK: - MessageBubble

/// One chat bubble: role label, text, and (assistant only) the tool-trace
/// disclosure - the evidence surface docs/PHASE6.md calls out ("the shell
/// can render evidence next to the model text") - plus, below it (C2,
/// docs/PHASE6-C2.md), one [`ProposalCard`] per `message.proposals`. User
/// bubbles align right (a leading `Spacer`), assistant bubbles align left,
/// the common chat-UI convention; proposal cards only ever appear on an
/// assistant message (`CopilotMessage.proposals`' own doc comment), so they
/// share the assistant bubble's left alignment and trailing margin
/// (`Spacer(minLength: 48)`) rather than needing their own role switch.
@MainActor
private struct MessageBubble: View {
    let message: CopilotMessage
    let onApproveKill: (String, String) async -> Bool
    let onApproveBudget: (String, Double, String) async -> Bool
    let onApproveGrantDeny: (String, ApprovalVerdict) async -> Bool
    let onApproveRescan: () async -> Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            bubble
            if !message.proposals.isEmpty {
                VStack(spacing: 8) {
                    ForEach(Array(message.proposals.enumerated()), id: \.offset) { _, proposal in
                        ProposalCard(
                            proposal: proposal,
                            onApproveKill: onApproveKill,
                            onApproveBudget: onApproveBudget,
                            onApproveGrantDeny: onApproveGrantDeny,
                            onApproveRescan: onApproveRescan)
                    }
                }
                .padding(.trailing, 48)
            }
        }
    }

    private var bubble: some View {
        HStack(alignment: .top, spacing: 0) {
            if message.role == .user { Spacer(minLength: 48) }
            VStack(alignment: .leading, spacing: 6) {
                Text(message.role == .user ? "YOU" : "FELYX")
                    .font(Theme.mono(9, weight: .semibold))
                    .tracking(0.8)
                    .foregroundStyle(message.role == .user ? Theme.textTertiary : Theme.iris)
                Text(message.text)
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                if !message.toolTrace.isEmpty {
                    ToolTraceDisclosure(trace: message.toolTrace)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(message.role == .user ? Theme.panelElevated : Theme.iris.opacity(0.08))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .strokeBorder(message.role == .user ? Theme.hairline : Theme.iris.opacity(0.25), lineWidth: 1)
            )
            if message.role == .assistant { Spacer(minLength: 48) }
        }
    }
}

// MARK: - ToolTraceDisclosure

/// A collapsed-by-default "N tools used" row that expands into each
/// `CopilotToolDto`: name, ok/failed, and a result preview - so an operator
/// can check exactly which reads backed a given answer (docs/PHASE6.md: the
/// `tool_trace` "so the shell can render evidence next to the model text").
@MainActor
private struct ToolTraceDisclosure: View {
    let trace: [CopilotToolDto]

    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation { expanded.toggle() }
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 8, weight: .bold))
                    Text("\(trace.count) tool\(trace.count == 1 ? "" : "s") used")
                }
                .font(Theme.mono(10, weight: .semibold))
                .foregroundStyle(Theme.textSecondary)
            }
            .buttonStyle(.plain)

            if expanded {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(trace.enumerated()), id: \.offset) { _, call in
                        toolRow(call)
                    }
                }
                .padding(.leading, 4)
            }
        }
    }

    private func toolRow(_ call: CopilotToolDto) -> some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: call.ok ? "checkmark.circle.fill" : "xmark.circle.fill")
                .font(.system(size: 9))
                .foregroundStyle(call.ok ? Theme.mint : Theme.coral)
            VStack(alignment: .leading, spacing: 1) {
                Text(call.name)
                    .font(Theme.mono(10, weight: .semibold))
                    .foregroundStyle(Theme.textPrimary)
                Text(call.resultPreview)
                    .font(Theme.mono(9.5))
                    .foregroundStyle(Theme.textTertiary)
                    .lineLimit(3)
                    .textSelection(.enabled)
            }
        }
    }
}

// MARK: - ProposalCard (C2, docs/PHASE6-C2.md)

/// One `ProposedAction` rendered as a card: the recommendation (kind +
/// target + params), why, how confident Felyx is, the evidence/policy
/// backing it, and Approve/Dismiss underneath. "Approve" NEVER signs
/// anything itself - it calls straight into one of the four `onApprove*`
/// closures `CopilotView` was handed from `GenaryxApp`, which themselves
/// call straight into the EXACT existing signed ceremony the Money/Policy/
/// Identity panels already use. For Kill/Budget, tapping "Approve" arms the
/// SAME shared `BreakGlassPanel` (`MoneyComponents.swift`) every other
/// break-glass action in this app reveals - the mandatory typed reason plus
/// the actual "Touch ID to override" trigger both live there unchanged, so
/// this view collects a reason but never invents its own confirm ceremony.
/// GrantDeny/Rescan need no reason (Touch ID alone, or nothing at all for
/// Rescan - both gates already live inside `PolicyModel.decide`/
/// `IdentityModel.rescan` themselves), so their "Approve" calls straight
/// through.
@MainActor
private struct ProposalCard: View {
    let proposal: CopilotProposalDto
    let onApproveKill: (String, String) async -> Bool
    let onApproveBudget: (String, Double, String) async -> Bool
    let onApproveGrantDeny: (String, ApprovalVerdict) async -> Bool
    let onApproveRescan: () async -> Bool

    /// Only Kill/Budget ever arm this (see the type doc) - GrantDeny/Rescan
    /// go straight from "Approve" to their closure, never through here.
    private enum ArmedReason { case kill, budget(usdCap: Double) }

    private enum Settled { case approved, dismissed }

    @State private var armed: ArmedReason?
    @State private var pending = false
    @State private var settled: Settled?

    private var params: ProposalParams { ProposalParams(kind: proposal.kind, json: proposal.paramsJson) }

    var body: some View {
        Group {
            if let settled {
                settledRow(settled)
            } else {
                card
            }
        }
    }

    // MARK: - the live card

    private var card: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            Text(proposal.rationale)
                .font(Theme.mono(11.5))
                .foregroundStyle(Theme.textPrimary)
                .fixedSize(horizontal: false, vertical: true)

            if !proposal.evidenceRefs.isEmpty {
                Text("evidence: \(proposal.evidenceRefs.joined(separator: ", "))")
                    .font(Theme.mono(10))
                    .foregroundStyle(Theme.textSecondary)
                    .textSelection(.enabled)
            }
            if !proposal.policyContext.isEmpty {
                Text("Governed by policy: \(proposal.policyContext.joined(separator: ", "))")
                    .font(Theme.mono(10))
                    .foregroundStyle(Theme.textTertiary)
            }

            if let armed {
                BreakGlassPanel(
                    summary: summary(for: armed),
                    onConfirm: { reason in await confirmArmed(armed, reason: reason) },
                    onCancel: { self.armed = nil })
            } else {
                actions
            }
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.iris.opacity(0.06))
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .strokeBorder(Theme.iris.opacity(0.3), lineWidth: 1)
        )
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(verb.uppercased())
                    .font(Theme.mono(11, weight: .bold))
                    .tracking(0.6)
                    .foregroundStyle(Theme.iris)
                HStack(spacing: 6) {
                    Text(proposal.target)
                        .font(Theme.mono(11.5))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if let paramsText {
                        Text(paramsText)
                            .font(Theme.mono(10.5, weight: .semibold))
                            .foregroundStyle(Theme.textSecondary)
                    }
                }
            }
            Spacer(minLength: 8)
            confidenceChip
        }
    }

    private var confidenceChip: some View {
        let pct = Int((proposal.confidence * 100).rounded())
        let tone: Color =
            proposal.confidence >= 0.75 ? Theme.mint : (proposal.confidence >= 0.5 ? Theme.amber : Theme.coral)
        return Text("\(pct)% confidence")
            .font(Theme.mono(10, weight: .semibold))
            .foregroundStyle(tone)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(tone.opacity(0.14)))
            .overlay(Capsule().strokeBorder(tone.opacity(0.4), lineWidth: 1))
    }

    private var actions: some View {
        HStack(spacing: 10) {
            Spacer(minLength: 0)
            Button("Dismiss") { settled = .dismissed }
                .buttonStyle(.plain)
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.textTertiary)
                .disabled(pending)
            approveButton
        }
    }

    /// Kill/Budget arm the reason ceremony; GrantDeny/Rescan call straight
    /// through (see the type doc). An unrecognized `kind` (only reachable if
    /// a future propose tool ships before this shell is updated) renders an
    /// honest note instead of a dead button.
    @ViewBuilder
    private var approveButton: some View {
        switch proposal.kind {
        case "kill":
            approveLabel(tone: Theme.ember) { armed = .kill }
        case "budget":
            if case .budget(let usdCap) = params {
                approveLabel(tone: Theme.ember) { armed = .budget(usdCap: usdCap) }
            } else {
                Text("no budget amount given")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
            }
        case "grant_deny":
            if case .verdict(let verdict) = params, let approvalVerdict = ApprovalVerdict(wire: verdict) {
                approveLabel(tone: approvalVerdict == .grant ? Theme.mint : Theme.ember) {
                    Task { await runDirectly { await onApproveGrantDeny(proposal.target, approvalVerdict) } }
                }
            } else {
                Text("no verdict given")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
            }
        case "rescan":
            approveLabel(tone: Theme.mint) {
                Task { await runDirectly { await onApproveRescan() } }
            }
        default:
            Text("Felyx proposed an unrecognized action kind (\u{201C}\(proposal.kind)\u{201D})")
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textTertiary)
        }
    }

    private func approveLabel(tone: Color, action: @escaping () -> Void) -> some View {
        Button(pending ? "Working..." : "Approve", action: action)
            .buttonStyle(.plain)
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(pending ? Theme.textTertiary : tone)
            .disabled(pending)
    }

    private func settledRow(_ settled: Settled) -> some View {
        let approved = settled == .approved
        return HStack(spacing: 6) {
            Image(systemName: approved ? "checkmark.circle.fill" : "xmark.circle")
                .font(.system(size: 10))
            Text("\(approved ? "approved" : "dismissed") \u{00B7} \(verb.lowercased()) \(proposal.target)")
                .font(Theme.mono(10.5, weight: .semibold))
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .foregroundStyle(approved ? Theme.mint : Theme.textTertiary)
        .padding(.horizontal, 4)
        .padding(.vertical, 4)
    }

    // MARK: - routing

    /// The Kill/Budget `BreakGlassPanel` confirm handler: the SAME
    /// `CloudModel.killRun`/`setBudget` every other break-glass trigger in
    /// this app calls (Touch ID is challenged INSIDE those methods, not
    /// here). Resets `armed` regardless of outcome (mirrors
    /// `RunsBoard.RunRow`'s identical reset in `MoneyView.swift`, so a
    /// declined/failed attempt returns to the idle Approve/Dismiss row
    /// rather than getting stuck on the reason field) and only collapses
    /// the card to its settled state on success - a failure stays
    /// retryable.
    private func confirmArmed(_ kind: ArmedReason, reason: String) async {
        pending = true
        let ok: Bool
        switch kind {
        case .kill:
            ok = await onApproveKill(proposal.target, reason)
        case .budget(let usdCap):
            ok = await onApproveBudget(proposal.target, usdCap, reason)
        }
        pending = false
        armed = nil
        if ok { settled = .approved }
    }

    /// The GrantDeny/Rescan path: no reason to collect, so "Approve" runs
    /// `action` directly - `pending` still drives the same "Working..."
    /// label `approveLabel` shows for Kill/Budget's own Touch-ID round trip.
    private func runDirectly(_ action: @escaping () async -> Bool) async {
        pending = true
        let ok = await action()
        pending = false
        if ok { settled = .approved }
    }

    private func summary(for kind: ArmedReason) -> String {
        switch kind {
        case .kill:
            return "Approve Felyx's proposal to kill run \(proposal.target)."
        case .budget(let usdCap):
            return "Approve Felyx's proposal to cap run \(proposal.target)'s budget at \(MoneyFormat.usd(usdCap))."
        }
    }

    // MARK: - display text

    private var verb: String {
        switch proposal.kind {
        case "kill": return "Kill run"
        case "budget": return "Cap budget"
        case "grant_deny":
            if case .verdict(let v) = params {
                return v == "deny" ? "Deny approval" : "Grant approval"
            }
            return "Grant / deny approval"
        case "rescan": return "Rescan"
        default: return proposal.kind.capitalized
        }
    }

    private var paramsText: String? {
        switch params {
        case .budget(let usdCap): return "\(MoneyFormat.usd(usdCap)) cap"
        case .verdict(let v): return "verdict: \(v)"
        case .none: return nil
        }
    }
}

/// Decodes `CopilotProposalDto.paramsJson` for display - the shell-side half
/// of that field's own doc comment ("UniFFI has no arbitrary-JSON type... the
/// Swift side decodes it itself"). An empty `{}` (Kill/Rescan's actual
/// shape), a params shape this card does not recognize, or plain undecodable
/// text all yield `.none` rather than a crash - a proposal card must always
/// render, even against an unexpected/future params shape.
private enum ProposalParams: Equatable {
    case budget(usdCap: Double)
    case verdict(String)
    case none

    init(kind: String, json: String) {
        guard
            let data = json.data(using: .utf8),
            let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else {
            self = .none
            return
        }
        switch kind {
        case "budget":
            if let cap = obj["usd_cap"] as? Double {
                self = .budget(usdCap: cap)
            } else if let cap = obj["usd_cap"] as? Int {
                self = .budget(usdCap: Double(cap))
            } else {
                self = .none
            }
        case "grant_deny":
            if let verdict = obj["verdict"] as? String {
                self = .verdict(verdict)
            } else {
                self = .none
            }
        default:
            self = .none
        }
    }
}

extension ApprovalVerdict {
    /// Decode Wardryx's own wire string (`"grant"` / `"deny"` - exactly what
    /// a GrantDeny proposal's `params.verdict` carries) into this FFI enum.
    /// `nil` on anything else, so [`ProposalCard`] can fall back to an
    /// honest "no verdict given" instead of guessing.
    init?(wire: String) {
        switch wire {
        case "grant": self = .grant
        case "deny": self = .deny
        default: return nil
        }
    }
}
