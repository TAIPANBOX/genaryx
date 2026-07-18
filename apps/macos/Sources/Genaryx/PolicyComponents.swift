import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Policy view: the "not ready yet" empty
/// state, the decoded-grant detail card, and a small raw-line parsing helper
/// for the Decision Stream. Mirrors `MoneyComponents.swift`'s own role and
/// distribution: view-specific tables/rows (the Decision Stream, Approvals
/// Inbox, and Policy list themselves) live in `PolicyView.swift`, exactly
/// where `RunsTable`/`IncidentsList`/`SavingsBreakdown` live in
/// `MoneyView.swift` rather than in `MoneyComponents.swift`; only the pieces
/// that render one self-contained DTO shape (a whole-panel state, a decoded
/// outcome) live here, alongside `StatTile`/`ConfirmButton`/`SeverityPill`/
/// `MoneyFormat` (all reused directly from `MoneyComponents.swift` - see
/// `OverviewView.swift` for the established precedent of one panel reusing
/// another's generic, non-`private` atoms).

// MARK: - PolicyEmptyStateView

/// Shared "not ready yet" rendering for the Policy view: three honest,
/// distinct states (never a generic spinner-forever or error toast), plus
/// the PHASE2.md-mandated clean "no policy plane" outcome for an environment
/// that never ran `taipan up --with wardryx` at all. Mirrors
/// `MoneyEmptyStateView` field-for-field, swapped to `WardryxConnection`.
@MainActor
struct PolicyEmptyStateView: View {
    let connection: WardryxConnection

    var body: some View {
        centered {
            switch connection {
            case .connecting:
                Text("connecting to a Wardryx policy plane...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)

            case .noEnvironment:
                card {
                    Text("No policy plane found")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.textPrimary)
                    Text(
                        "Run taipan up --with wardryx to start a policy plane, or set WARDRYX_ADMIN_KEY for a Wardryx already running at 127.0.0.1:8090."
                    )
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                }

            case .connectFailed(let reason):
                card {
                    Text("Could not connect to Wardryx")
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

// MARK: - GrantTokenCard

/// Renders a GRANTED `ApprovalDecideOutcome`: exactly what the operator just
/// authorized (PHASE2.md - "show the operator exactly what they authorized:
/// agent/run, tools, cost ceiling, expiry countdown"), plus a fixed
/// single-use awareness caption. Never rendered for a deny (`PolicyModel`
/// only ever sets `lastGrant` when `outcome.granted` is `true` - see its own
/// doc comment).
@MainActor
struct GrantTokenCard: View {
    let outcome: ApprovalDecideOutcome
    let onDismiss: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text("APPROVAL GRANTED")
                    .font(Theme.mono(10, weight: .semibold))
                    .tracking(1.0)
                    .foregroundStyle(Theme.mint)
                Text(outcome.approvalId)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 8)
                Button("Dismiss", action: onDismiss)
                    .buttonStyle(.plain)
                    .font(Theme.mono(10.5, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
            }

            HStack(spacing: 12) {
                if let ceiling = outcome.costCeilingUsd {
                    StatTile(label: "cost ceiling", value: MoneyFormat.usd(ceiling), tone: Theme.mint)
                }
                if let expiresAtUnix = outcome.expiresAtUnix {
                    TTLCountdownTile(expiresAtUnix: expiresAtUnix)
                }
            }

            if !outcome.tools.isEmpty {
                Text("tools authorized: \(outcome.tools.joined(separator: ", "))")
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
            }

            // A static awareness caption, not a fact this shell can confirm:
            // whether the server actually enforces single-use redemption is
            // a server-side `WARDRYX_APPROVAL_SINGLE_USE` setting with no
            // wire signal exposed anywhere this client can read (PHASE2.md:
            // "Caption single-use awareness"). Never presented as a
            // definitive per-token property.
            Text(
                "If this Wardryx server enforces single-use tokens (WARDRYX_APPROVAL_SINGLE_USE), this token can be redeemed only once."
            )
            .font(Theme.mono(10.5))
            .foregroundStyle(Theme.textTertiary)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .fill(Theme.mint.opacity(0.08))
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .strokeBorder(Theme.mint.opacity(0.35), lineWidth: 1)
        )
    }
}

/// A live "MM:SS remaining" tile, ticking every second off `expiresAtUnix`
/// (an absolute Unix timestamp - see `ApprovalDecideOutcome.expiresAtUnix`'s
/// own doc comment for why an absolute instant is carried across FFI rather
/// than a snapshot duration) so the countdown stays accurate for as long as
/// the card is on screen rather than freezing at its fetch-time value.
@MainActor
private struct TTLCountdownTile: View {
    let expiresAtUnix: Int64

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            StatTile(
                label: "expires in",
                value: remainingText(at: context.date),
                tone: remainingSeconds(at: context.date) <= 30 ? Theme.coral : Theme.mint
            )
        }
    }

    private func remainingSeconds(at date: Date) -> Int {
        let expiry = Date(timeIntervalSince1970: TimeInterval(expiresAtUnix))
        return max(0, Int(expiry.timeIntervalSince(date)))
    }

    private func remainingText(at date: Date) -> String {
        let seconds = remainingSeconds(at: date)
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }
}

// MARK: - PolicyDate

/// Parses the RFC3339 UTC timestamps `ApprovalRecord.decidedAt`/`requestedAt`
/// carry (see the generated binding's own doc comment) into a `Date`, for the
/// hero band's "decided today" count - the one derived stat this view
/// computes locally rather than reading directly off a DTO field. Mirrors
/// `Dash`'s own private `isoFrac`/`isoPlain` pair (`DashKit.swift`), kept as
/// a second small copy rather than exposed from there so `DashKit.swift`
/// stays a pure, panel-agnostic kit.
enum PolicyDate {
    static func parse(_ iso: String) -> Date? {
        isoFrac.date(from: iso) ?? isoPlain.date(from: iso)
    }

    private nonisolated(unsafe) static let isoFrac: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    private nonisolated(unsafe) static let isoPlain: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()
}

// MARK: - UiEvent wardryx `data` parsing (Decision Stream)

/// Best-effort extraction of the couple of `data.*` fields the Decision
/// Stream row needs, plus `approval_id` for `ApprovalNotificationModel`
/// (docs/PHASE2.md Wave 3: de-dupe "at most one notification per
/// approval_id"), the same shape the Decision Stream already keys its own
/// rows off of. `UiEvent` only carries the envelope's typed fields
/// (`crates/ffi/src/lib.rs`'s own doc comment: "`data`... omitted until a
/// view needs them"), so this parses `raw` directly - the same thing
/// `RawJsonView` already does for its full pretty-print
/// (`BusExplorerView.swift`) - rather than adding a `data` field to the
/// shared `UiEvent` Record for one panel's use. Never force-unwrapped: any
/// parse failure yields the empty shape, never a crash.
extension UiEvent {
    struct WardryxFields {
        let reason: String?
        let toolNames: [String]
        let approvalId: String?
    }

    var wardryxFields: WardryxFields {
        guard
            let bytes = raw.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any],
            let data = object["data"] as? [String: Any]
        else {
            return WardryxFields(reason: nil, toolNames: [], approvalId: nil)
        }
        return WardryxFields(
            reason: data["reason"] as? String,
            toolNames: data["tool_names"] as? [String] ?? [],
            approvalId: data["approval_id"] as? String
        )
    }
}
