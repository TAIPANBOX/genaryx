import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Identity view: the "not ready yet" empty
/// state, a small reusable filter chip, "as of load" formatting, and the
/// attestation free-text parser. Mirrors `PolicyComponents.swift`'s own role
/// and distribution: view-specific lists/rows (the Identities list, the
/// Alerts stream, the Attestation section) live in `IdentityView.swift`,
/// exactly where `DecisionStreamSection`/`ApprovalsInboxSection` live in
/// `PolicyView.swift` rather than here.

// MARK: - IdentityEmptyStateView

/// Shared "not ready yet" rendering for the Identity view: three honest,
/// distinct states (never a generic spinner-forever or error toast), plus
/// the PHASE3.md-mandated clean "no identity plane" outcome for an
/// environment that never ran `taipan up --with idryx` at all. Mirrors
/// `PolicyEmptyStateView` field-for-field, swapped to `IdryxConnection`.
@MainActor
struct IdentityEmptyStateView: View {
    let connection: IdryxConnection

    var body: some View {
        centered {
            switch connection {
            case .connecting:
                Text("connecting to an Idryx identity plane...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)

            case .noEnvironment:
                card {
                    Text("No identity plane found")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.textPrimary)
                    Text(
                        "Run taipan up --with idryx to start an identity plane, or set IDRYX_URL for an idryx already running (e.g. http://127.0.0.1:8081)."
                    )
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                }

            case .connectFailed(let reason):
                card {
                    Text("Could not connect to Idryx")
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

// MARK: - FilterChip

/// A small toggle pill for the Identities type filter and the Alerts
/// severity filter - a selectable variant of `PolicyListSection`'s own
/// `tagPill` (that one is a read-only label; this one is a `Button` that
/// reports its own selected state). An empty selection set means "show all"
/// at both call sites, never "show nothing" - see `IdentityView.swift`'s own
/// filtering logic.
@MainActor
struct FilterChip: View {
    let label: String
    let isSelected: Bool
    var tone: Color = Theme.textSecondary
    let onToggle: () -> Void

    var body: some View {
        Button(action: onToggle) {
            Text(label)
                .font(Theme.mono(10, weight: .semibold))
                .tracking(0.6)
                .foregroundStyle(isSelected ? tone : Theme.textTertiary)
                .padding(.horizontal, 9)
                .padding(.vertical, 4)
                .background(Capsule().fill(tone.opacity(isSelected ? 0.16 : 0.06)))
                .overlay(Capsule().strokeBorder(tone.opacity(isSelected ? 0.5 : 0.2), lineWidth: 1))
        }
        .buttonStyle(.plain)
    }
}

// MARK: - "as of load" formatting

/// The Identity panel's "as of load" label text - PHASE3.md: "A clear 'as of
/// load' label (serve is load-once)", never implying the snapshot is live.
enum LoadedAtFormat {
    static func label(_ date: Date?) -> String {
        guard let date else { return "not yet loaded" }
        let iso = ISO8601DateFormatter().string(from: date)
        return "as of load \u{00B7} \(MoneyFormat.timestamp(iso))"
    }
}

// MARK: - AlertRecord attestation parsing

/// Best-effort extraction of the `attestation=<value>` free text idryx embeds
/// in an `attestation_missing` alert's summary (docs/PHASE3.md: "Attestation
/// is NOT a structured field on the identity... the `attestation_missing`
/// detector, which embeds `attestation=<value>` as free text inside
/// `apiAlert.Summary`"). `nil` when the substring is absent - the
/// Attestation section still renders the raw `summary` in that case, never a
/// fabricated value (the parity checklist's own words: "honest: not a clean
/// field").
extension AlertRecord {
    var attestationValue: String? {
        guard let markerRange = summary.range(of: "attestation=") else { return nil }
        let tail = summary[markerRange.upperBound...]
        let endIndex = tail.firstIndex(where: { $0 == " " || $0 == "," }) ?? tail.endIndex
        let value = String(tail[tail.startIndex..<endIndex])
        return value.isEmpty ? nil : value
    }
}
