import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Crypto view: the "not ready yet" empty
/// state, the "as of last scan" label, the NCSC verdict color/label mapping,
/// and a best-effort CBOM JSON parser. Mirrors `QualityComponents.swift`'s
/// own role and distribution: view-specific sections (the NCSC hero, the
/// findings table, the CBOM table, the Evidence section) live in
/// `CryptoView.swift`, exactly where `EvalRunsSection`/`RunDetailSection`
/// live in `QualityView.swift` rather than here.

// MARK: - CryptoEmptyStateView

/// Shared "not ready yet" rendering for the Crypto view: three honest,
/// distinct states, plus the docs/PHASE4.md-mandated clean "no crypto plane"
/// outcome for a box with no `qryx` binary at all. Mirrors
/// `QualityEmptyStateView` field-for-field, swapped to `CryptoConnection`.
@MainActor
struct CryptoEmptyStateView: View {
    let connection: CryptoConnection

    var body: some View {
        centered {
            switch connection {
            case .connecting:
                Text("connecting to a Qryx crypto plane...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)

            case .noEnvironment:
                card {
                    Text("No crypto plane found")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.textPrimary)
                    Text(
                        "No qryx binary found at ~/.taipan/bin/qryx. Run taipan up --with qryx to install one."
                    )
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                }

            case .connectFailed(let reason):
                card {
                    Text("Could not run qryx")
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

// MARK: - "as of last scan" formatting

/// The Crypto panel's "as of last scan" label - docs/PHASE4.md W1: Qryx is
/// on-demand, never a live feed, so every reading is labeled honestly against
/// when it was actually pulled. Deliberately a SEPARATE formatter from
/// `LoadedAtFormat` (`IdentityComponents.swift`, "as of load"): that phrase
/// describes a load-once REST snapshot; this one describes an
/// operator-triggered subprocess scan - different mechanisms, so a shared
/// label would blur a real distinction the operator should be able to see.
enum LastScanFormat {
    static func label(_ date: Date?) -> String {
        guard let date else { return "no scan yet" }
        let iso = ISO8601DateFormatter().string(from: date)
        return "as of last scan \u{00B7} \(MoneyFormat.timestamp(iso))"
    }
}

// MARK: - NCSC verdict presentation

/// Color/label mapping for an NCSC milestone's `verdict` string
/// (`on-track` | `at-risk` | `not-started` - `crates/connectors/src/qryx.rs`'s
/// own doc). Shared by all three hero cards in `CryptoView.swift`.
enum NcscVerdictFormat {
    static func color(_ verdict: String) -> Color {
        switch verdict.lowercased() {
        case "on-track": return Theme.mint
        case "at-risk": return Theme.amber
        case "not-started": return Theme.coral
        default: return Theme.steel
        }
    }

    static func label(_ verdict: String) -> String {
        verdict.isEmpty ? "unknown" : verdict
    }
}

// MARK: - CBOM JSON parsing (best-effort, never force-unwrapped)

/// One CycloneDX `components[]` row, as much as this panel needs to render an
/// inventory table - a handful of best-effort optional lookups, NOT a full
/// CycloneDX 1.6 model (see `crypto::dto`'s own module doc on why CBOM
/// crosses FFI as a plain JSON string rather than a typed Record tree).
struct CbomComponent {
    let name: String
    let type: String
    let version: String?
    /// `cryptoProperties.assetType` when present (e.g. `algorithm`,
    /// `certificate`, `related-material`, `protocol` - CycloneDX's
    /// cryptographic-asset extension); `nil` for a component with no crypto
    /// properties at all (an ordinary library dependency alongside the
    /// crypto-relevant ones).
    let cryptoAssetType: String?
}

/// Best-effort extraction from `CryptoModel.cbomJson` - the same
/// `JSONSerialization` + manual dictionary access idiom
/// `UiEvent.wardryxFields`/`UiEvent.qualityDriftFields` already use for the
/// bus's own raw lines. Never force-unwrapped: any parse failure yields an
/// empty result, never a crash.
enum CbomParser {
    static func components(fromJson json: String?) -> [CbomComponent] {
        guard let root = rootObject(json), let raw = root["components"] as? [[String: Any]] else {
            return []
        }
        return raw.map { component in
            let cryptoProperties = component["cryptoProperties"] as? [String: Any]
            return CbomComponent(
                name: component["name"] as? String ?? "(unnamed)",
                type: component["type"] as? String ?? "-",
                version: component["version"] as? String,
                cryptoAssetType: cryptoProperties?["assetType"] as? String
            )
        }
    }

    /// `bomFormat`/`specVersion`, for the inventory table's summary caption.
    static func specVersion(fromJson json: String?) -> String? {
        rootObject(json)?["specVersion"] as? String
    }

    private static func rootObject(_ json: String?) -> [String: Any]? {
        guard let json, let bytes = json.data(using: .utf8) else { return nil }
        return try? JSONSerialization.jsonObject(with: bytes) as? [String: Any]
    }
}
