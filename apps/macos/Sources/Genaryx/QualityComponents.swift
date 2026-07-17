import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Quality view: the "not ready yet" empty
/// state, mean-score formatting, and the `quality_drift` bus-event parser.
/// Mirrors `IdentityComponents.swift`'s own role and distribution:
/// view-specific lists/rows (Eval Runs, Run Detail, Baselines, Drift Alerts)
/// live in `QualityView.swift`, exactly where `IdentitiesListSection`/
/// `AlertsSection` live in `IdentityView.swift` rather than here.

// MARK: - QualityEmptyStateView

/// Shared "not ready yet" rendering for the Quality view: three honest,
/// distinct states (never a generic spinner-forever or error toast), plus
/// the docs/PHASE4.md-mandated clean "no quality plane" outcome for a box
/// with no `verdryx.db` anywhere this handle knows to look. Mirrors
/// `IdentityEmptyStateView` field-for-field, swapped to `QualityConnection`.
@MainActor
struct QualityEmptyStateView: View {
    let connection: QualityConnection

    var body: some View {
        centered {
            switch connection {
            case .connecting:
                Text("connecting to a Verdryx quality plane...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)

            case .noEnvironment:
                card {
                    Text("No quality plane found")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.textPrimary)
                    Text(
                        "No verdryx.db found at VERDRYX_DB, ~/.taipan/verdryx.db, or ./verdryx.db. Run verdryx eval to create one, or set VERDRYX_DB to point at an existing store."
                    )
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                }

            case .connectFailed(let reason):
                card {
                    Text("Could not open verdryx.db")
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

// MARK: - QualityFormat

/// Mean-score formatting shared by the Eval Runs history and the Run Detail
/// header - docs/PHASE4.md W1 guard: "mean shown 'n/a' when null, never 0".
enum QualityFormat {
    static func meanScore(_ value: Double?) -> String {
        guard let value else { return "n/a" }
        return String(format: "%.3f", value)
    }
}

// MARK: - UiEvent quality_drift `data` parsing (Drift Alerts)

/// Best-effort extraction of the `quality_drift` bus event's `data.*` fields
/// (docs/PHASE4.md's grounding: `quality_drift`(high, ONLY on regression)
/// carries `{baseline_id, window, mean_score, delta, verdict, baseline_n,
/// t_statistic, ci_low, ci_high}`). `UiEvent` only carries the envelope's
/// typed fields (`crates/ffi/src/lib.rs`'s own doc comment: "`data`...
/// omitted until a view needs them"), so this parses `raw` directly - the
/// same technique `UiEvent.wardryxFields` already uses for the Decision
/// Stream (`PolicyComponents.swift`). Never force-unwrapped: any parse
/// failure yields the all-nil shape, never a crash; the Drift Alerts row
/// falls back to the envelope's own plain fields (source/type/time/severity)
/// when a specific `data.*` field is missing.
extension UiEvent {
    struct QualityDriftFields {
        let baselineId: String?
        let window: Int?
        let meanScore: Double?
        let delta: Double?
        let verdict: String?
        let baselineN: Int?
        let tStatistic: Double?
        let ciLow: Double?
        let ciHigh: Double?
    }

    var qualityDriftFields: QualityDriftFields {
        guard
            let bytes = raw.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any],
            let data = object["data"] as? [String: Any]
        else {
            return QualityDriftFields(
                baselineId: nil, window: nil, meanScore: nil, delta: nil, verdict: nil, baselineN: nil,
                tStatistic: nil, ciLow: nil, ciHigh: nil)
        }
        return QualityDriftFields(
            baselineId: data["baseline_id"] as? String,
            window: data["window"] as? Int,
            meanScore: data["mean_score"] as? Double,
            delta: data["delta"] as? Double,
            verdict: data["verdict"] as? String,
            baselineN: data["baseline_n"] as? Int,
            tStatistic: data["t_statistic"] as? Double,
            ciLow: data["ci_low"] as? Double,
            ciHigh: data["ci_high"] as? Double
        )
    }
}
