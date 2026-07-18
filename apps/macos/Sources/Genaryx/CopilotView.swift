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
                            MessageBubble(message: message).id(message.id)
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
/// can render evidence next to the model text"). User bubbles align right
/// (a leading `Spacer`), assistant bubbles align left, the common chat-UI
/// convention.
@MainActor
private struct MessageBubble: View {
    let message: CopilotMessage

    var body: some View {
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
