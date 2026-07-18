import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Drills view: the "not ready yet" empty
/// state and the overall-verdict badge formatter. Mirrors
/// `CryptoComponents.swift`'s own role and distribution: view-specific
/// sections (Run controls, per-scenario results, findings) live in
/// `DrillsView.swift` rather than here.

// MARK: - DrillsEmptyStateView

/// Shared "not ready yet" rendering for the Drills view: three honest,
/// distinct states, plus the docs/PHASE4.md-mandated clean "no drills plane"
/// outcome for a box with no `mockryx` binary at all. Mirrors
/// `CryptoEmptyStateView` field-for-field, swapped to `DrillsConnection`.
@MainActor
struct DrillsEmptyStateView: View {
    let connection: DrillsConnection

    var body: some View {
        centered {
            switch connection {
            case .connecting:
                Text("connecting to a drills plane...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)

            case .noEnvironment:
                card {
                    Text("No drills plane found")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.textPrimary)
                    Text(
                        "No mockryx binary found. Set MOCKRYX_BIN, or build one at ~/Development/mockryx/bin/mockryx (go build -o bin/mockryx ./cmd/mockryx)."
                    )
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                }

            case .connectFailed(let reason):
                card {
                    Text("Could not set up mockryx")
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

// MARK: - "as of last run" formatting

/// The Drills panel's "as of last run" label - docs/PHASE4.md W2: mockryx is
/// on-demand, never a live feed. Reads the REPORT'S OWN `generatedAt` field
/// (mockryx's own `generated_at`, RFC3339Nano UTC) rather than a
/// separately-tracked "when did the Swift model last touch this" timestamp:
/// `DrillsModel` may show a report it loaded from a PAST session's saved
/// file (`DrillsModel.connect`'s own doc: "the 'last run' view survives an
/// app restart"), and only the report's own field is honest about when that
/// run actually happened. Deliberately a separate formatter from
/// `LastScanFormat`/`RecallFormat` for the same reason those two stay
/// separate from each other - different mechanisms, so a shared label would
/// blur a real distinction.
enum DrillRunFormat {
    static func label(_ report: DrillReportRecord?) -> String {
        guard let report else { return "no run yet" }
        return "as of last run \u{00B7} \(MoneyFormat.timestamp(report.generatedAt))"
    }

    /// Bare "HH:mm" clock for a `FreshBadge.onDemand`, e.g. "ON-DEMAND ·
    /// 14:32" - `nil` before any report (this session's own run, or a past
    /// session's saved one) has ever loaded, matching
    /// `FreshBadge.onDemand(last:)`'s own "no last action" case (the same
    /// reasoning `LastScanFormat.clock`/`RecallFormat.clock` document for
    /// Crypto/Memory). The compact counterpart to `label(_:)`'s fuller "as
    /// of last run · ..." line - reads the report's own `generatedAt` for
    /// the same reason `label(_:)` does (see this type's own doc).
    static func clock(_ report: DrillReportRecord?) -> String? {
        guard let report, let date = isoDate(report.generatedAt) else { return nil }
        return clockFormatter.string(from: date)
    }

    private static func isoDate(_ iso: String) -> Date? {
        isoFrac.date(from: iso) ?? isoPlain.date(from: iso)
    }

    // `nonisolated(unsafe)`: configured once at first access and only ever
    // read afterward, the same reasoning `MoneyFormat`'s own formatters
    // document - `ISO8601DateFormatter` does not conform to `Sendable` on
    // this SDK, unlike `DateFormatter` below.
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
    private static let clockFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        f.locale = Locale(identifier: "en_US_POSIX")
        return f
    }()
}

// MARK: - overall verdict

/// docs/PHASE4.md W2: "rendering the report with an overall verdict from
/// `has_gaps()`". `DrillReportRecord.hasGaps` is already precomputed on the
/// Rust side (`crates/ffi/src/drills/dto.rs`'s own doc: "UniFFI Records
/// cannot carry methods"), so this is presentation only.
enum DrillVerdictFormat {
    static func label(hasGaps: Bool) -> String {
        hasGaps ? "GAP FOUND" : "GUARDRAILS HELD"
    }

    static func color(hasGaps: Bool) -> Color {
        hasGaps ? Theme.coral : Theme.mint
    }
}

/// docs/PHASE4.md W2: "per-scenario status (passed = held / failed = GAP /
/// skipped_not_configured = skip) + metrics; read 'gap' from
/// findings/failed, not status alone". `status` alone still drives the
/// LABEL/color (it is always one of exactly three wire values - `MockryxResult`'s
/// own doc, "exhaustive"), but a `skipped_not_configured` row that was
/// promoted into `findings` (via `fail_on_skip`) still needs to read as a
/// gap - callers pass `!result.findings.isEmpty` alongside `status` so this
/// can upgrade that one case rather than trusting `status` in isolation.
enum DrillStatusFormat {
    static func label(status: String, hasFindings: Bool) -> String {
        switch status {
        case "passed": return "held"
        case "failed": return "GAP"
        case "skipped_not_configured": return hasFindings ? "GAP (skip promoted)" : "skip"
        default: return status
        }
    }

    static func color(status: String, hasFindings: Bool) -> Color {
        switch status {
        case "passed": return hasFindings ? Theme.coral : Theme.mint
        case "failed": return Theme.coral
        case "skipped_not_configured": return hasFindings ? Theme.coral : Theme.textTertiary
        default: return Theme.steel
        }
    }
}
