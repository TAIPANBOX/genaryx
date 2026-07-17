import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Overview and Money views: a stat tile, a
/// two-step confirm button, an inline budget editor, a severity pill, the
/// "not ready yet" empty state, and an upsell banner - each a native SwiftUI
/// analog of the Tauri shell's own `StatTile.tsx` / `ConfirmButton.tsx` /
/// `BudgetEditor.tsx` / `SeverityBadge.tsx` / `MoneyEmptyState.tsx` /
/// `UpsellBanner.tsx`, so both shells present the same information with the
/// same interaction shape (always a confirm step before a mutation), even
/// though the visual chrome follows native macOS conventions instead of
/// pixel parity with the web shell (see `Theme.swift`'s own doc).

// MARK: - StatTile

/// One Overview/Savings tile: a label, a big tabular number, and an optional
/// secondary line. Mirrors `StatTile.tsx`.
@MainActor
struct StatTile: View {
    let label: String
    let value: String
    var sub: String?
    var tone: Color?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label.uppercased())
                .font(Theme.mono(10, weight: .semibold))
                .tracking(1.0)
                .foregroundStyle(Theme.textTertiary)
            Text(value)
                .font(Theme.mono(22, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(tone ?? Theme.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            if let sub {
                Text(sub)
                    .font(.system(size: 11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panelElevated)
        )
    }
}

// MARK: - ConfirmButton

/// A privileged action button that always shows an inline confirm step
/// before calling `onConfirm` - never a single click straight to a
/// signed mutation. Three states: idle -> confirming (Confirm/Cancel) ->
/// pending (disabled, mid-flight). Mirrors `ConfirmButton.tsx`.
@MainActor
struct ConfirmButton: View {
    let label: String
    let confirmLabel: String
    var tone: Color = Theme.coral
    let onConfirm: () async -> Void

    @State private var confirming = false
    @State private var pending = false

    var body: some View {
        if pending {
            pill(pendingLabel, color: Theme.textTertiary)
        } else if confirming {
            HStack(spacing: 6) {
                Button {
                    pending = true
                    Task {
                        await onConfirm()
                        pending = false
                        confirming = false
                    }
                } label: {
                    pill(confirmLabel, color: tone, filled: true)
                }
                .buttonStyle(.plain)

                Button {
                    confirming = false
                } label: {
                    Text("Cancel")
                        .font(Theme.mono(11, weight: .semibold))
                        .foregroundStyle(Theme.textTertiary)
                }
                .buttonStyle(.plain)
            }
        } else {
            Button {
                confirming = true
            } label: {
                Text(label)
                    .font(Theme.mono(11, weight: .semibold))
                    .foregroundStyle(tone)
            }
            .buttonStyle(.plain)
        }
    }

    private var pendingLabel: String { "Working..." }

    private func pill(_ text: String, color: Color, filled: Bool = false) -> some View {
        Text(text)
            .font(Theme.mono(10, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(filled ? color : Theme.textSecondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(color.opacity(filled ? 0.16 : 0.1)))
            .overlay(Capsule().strokeBorder(color.opacity(0.4), lineWidth: 1))
    }
}

// MARK: - BudgetEditor

/// Inline per-run budget editor: "Budget" reveals a number field, "Set"
/// moves to an explicit confirm step (the same rule `ConfirmButton` follows),
/// and only then calls `onSubmit`. Mirrors `BudgetEditor.tsx`.
@MainActor
struct BudgetEditor: View {
    let runId: String
    let currentUsd: Double?
    let onSubmit: (String, Double) async -> Void

    @State private var editing = false
    @State private var text = ""
    @State private var confirming = false
    @State private var pending = false
    @FocusState private var fieldFocused: Bool

    var body: some View {
        if !editing {
            Button {
                text = currentUsd.map { String(format: "%.2f", $0) } ?? ""
                confirming = false
                editing = true
                fieldFocused = true
            } label: {
                Text("Budget")
                    .font(Theme.mono(11, weight: .semibold))
                    .foregroundStyle(Theme.textSecondary)
            }
            .buttonStyle(.plain)
        } else {
            HStack(spacing: 6) {
                TextField("USD", text: $text)
                    .textFieldStyle(.plain)
                    .font(Theme.mono(11, weight: .medium))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textPrimary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .frame(width: 72)
                    .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panelElevated))
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))
                    .disabled(pending)
                    .focused($fieldFocused)
                    .onChange(of: text) { confirming = false }

                if confirming {
                    Button {
                        let value = parsedValue
                        pending = true
                        Task {
                            await onSubmit(runId, value)
                            pending = false
                            confirming = false
                            editing = false
                        }
                    } label: {
                        Text(pending ? "Setting..." : "Confirm $\(formattedParsed)")
                            .font(Theme.mono(10, weight: .semibold))
                            .foregroundStyle(Theme.amber)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(Capsule().fill(Theme.amber.opacity(0.16)))
                            .overlay(Capsule().strokeBorder(Theme.amber.opacity(0.4), lineWidth: 1))
                    }
                    .buttonStyle(.plain)
                    .disabled(pending)

                    Button("Cancel") { confirming = false }
                        .buttonStyle(.plain)
                        .font(Theme.mono(11, weight: .semibold))
                        .foregroundStyle(Theme.textTertiary)
                        .disabled(pending)
                } else {
                    Button("Set") { confirming = true }
                        .buttonStyle(.plain)
                        .font(Theme.mono(11, weight: .semibold))
                        .foregroundStyle(validAmount ? Theme.textSecondary : Theme.textTertiary)
                        .disabled(!validAmount)

                    Button("Close") { editing = false }
                        .buttonStyle(.plain)
                        .font(Theme.mono(11, weight: .semibold))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
        }
    }

    private var parsedValue: Double { Double(text) ?? 0 }
    private var formattedParsed: String { String(format: "%.2f", parsedValue) }
    private var validAmount: Bool {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, let value = Double(trimmed) else { return false }
        return value >= 0
    }
}

// MARK: - SeverityPill

/// Severity pill for the Money view's incidents list: a glowing dot in the
/// severity's color plus its label, on a low-opacity tint of the same hue.
/// A separate, small copy of `BusExplorerView.swift`'s private
/// `SeverityBadge` (same visual shape, reused `Theme` tokens) rather than a
/// shared import, so the Bus Explorer file stays completely untouched.
@MainActor
struct SeverityPill: View {
    let severity: String

    var body: some View {
        let color = Theme.severityColor(severity)
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 7, height: 7)
                .shadow(color: color.opacity(0.6), radius: 3)
            Text(Theme.severityLabel(severity))
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

// MARK: - MoneyEmptyStateView

/// Shared "not ready yet" rendering for the Overview and Money views: three
/// honest, distinct states (never a generic spinner-forever or error toast).
/// Mirrors `MoneyEmptyState.tsx`.
@MainActor
struct MoneyEmptyStateView: View {
    let connection: CloudConnection

    var body: some View {
        centered {
            switch connection {
            case .connecting:
                Text("connecting to a TokenFuse Cloud environment...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)

            case .noEnvironment:
                card {
                    Text("No environment found")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.textPrimary)
                    Text(
                        "Run taipan up to start a stack, or set TOKENFUSE_CLOUD_ADMIN_KEY for a Cloud already running at 127.0.0.1:8080."
                    )
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                }

            case .pairingFailed(let reason):
                card {
                    Text("Could not pair with the Cloud")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.coral)
                    Text(reason)
                        .font(Theme.mono(11.5))
                        .foregroundStyle(Theme.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }

            case .ready:
                EmptyView()
            }
        }
    }

    @ViewBuilder
    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8, content: content)
            .padding(20)
            .frame(maxWidth: 460, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                    .fill(Theme.panelElevated)
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                    .strokeBorder(Theme.hairline, lineWidth: 1)
            )
    }

    @ViewBuilder
    private func centered<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack {
            Spacer(minLength: 0)
            HStack {
                Spacer(minLength: 0)
                content()
                Spacer(minLength: 0)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(24)
    }
}

// MARK: - UpsellBannerView

/// Renders a `CloudError.PlanRequired` rejection as an upsell tile, never as
/// an error banner. The upgrade URL is plain, selectable text rather than a
/// clickable link (consistent with the Tauri shell's own choice - see
/// `UpsellBanner.tsx`'s doc - and this app never opens external links on the
/// operator's behalf without being asked).
@MainActor
struct UpsellBannerView: View {
    let notice: PlanRequiredNotice

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Text("UPGRADE")
                .font(Theme.mono(10, weight: .semibold))
                .tracking(0.8)
                .foregroundStyle(Theme.amber)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(Capsule().fill(Theme.amber.opacity(0.16)))
                .overlay(Capsule().strokeBorder(Theme.amber.opacity(0.4), lineWidth: 1))

            VStack(alignment: .leading, spacing: 3) {
                Text("\(notice.feature) is not available on \(notice.org)'s current plan.")
                    .font(.system(size: 12.5))
                    .foregroundStyle(Theme.textPrimary)
                Text(notice.upgradeUrl)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .textSelection(.enabled)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.amber.opacity(0.08))
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .strokeBorder(Theme.amber.opacity(0.35), lineWidth: 1)
        )
    }
}

// MARK: - ErrorBannerView

/// A plain error banner for any non-`plan_required` `CloudError` (the
/// upsell tile above handles that one case specially). Mirrors the inline
/// error `<div>` both `OverviewView.tsx` and `MoneyView.tsx` render.
@MainActor
struct ErrorBannerView: View {
    let message: String

    var body: some View {
        HStack {
            Text(message)
                .font(Theme.mono(11.5))
                .foregroundStyle(Theme.coral)
                .lineLimit(2)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panelElevated)
        )
    }
}

// MARK: - formatting

/// Money-figure and timestamp formatting shared by the Overview and Money
/// views. Mirrors `lib/format.ts` (`formatUsd`/`formatTimestamp`) exactly.
enum MoneyFormat {
    /// 2 decimals normally, 4 for sub-dollar amounts (TokenFuse spend is
    /// routinely sub-cent - `$0.0012`, not `$0.00`).
    static func usd(_ value: Double) -> String {
        let decimals = (value != 0 && abs(value) < 1) ? 4 : 2
        return "$" + String(format: "%.\(decimals)f", value)
    }

    /// Compact "Jul 16 14:32:05" clock for table rows, parsed from the RFC
    /// 3339 timestamps `CloudHandle` returns. Falls back to the raw string
    /// if it doesn't parse as ISO 8601, never force-unwrapped.
    static func timestamp(_ iso: String) -> String {
        guard let date = isoFormatter.date(from: iso) ?? isoFormatterNoFraction.date(from: iso) else {
            return iso
        }
        return displayFormatter.string(from: date)
    }

    // `nonisolated(unsafe)`: each formatter is configured exactly once at
    // first access and only ever read afterward (`.date(from:)`/
    // `.string(from:)`), so sharing it across threads is genuinely safe even
    // though `ISO8601DateFormatter`/`DateFormatter` do not conform to
    // `Sendable` - the attribute documents that reasoning instead of
    // isolating these pure-formatting helpers to a single actor.
    private nonisolated(unsafe) static let isoFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private nonisolated(unsafe) static let isoFormatterNoFraction: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    // `DateFormatter` (unlike `ISO8601DateFormatter` above) is already
    // `Sendable` on this SDK, so no `nonisolated(unsafe)` is needed here.
    private static let displayFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "MMM dd HH:mm:ss"
        f.locale = Locale(identifier: "en_US_POSIX")
        return f
    }()
}
