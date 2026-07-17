import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Evidence view: the "not ready yet" empty
/// state (gated on `CloudConnection`, since every source in this panel
/// builds through the same paired `CloudHandle` Money/Overview use - see
/// `EvidenceModel.swift`'s own doc) plus small formatters (short sha256,
/// human-readable byte counts). Mirrors `CryptoComponents.swift`'s own role
/// and distribution: view-specific sections (source toggles, the manifest
/// header, the artifacts table, the "not included" list) live in
/// `EvidenceView.swift` rather than here.

// MARK: - EvidenceEmptyStateView

/// Shared "not ready yet" rendering for the Evidence view - reuses
/// `CloudConnection` rather than a new enum, because every source this panel
/// can build from (Cloud, Qryx, Agent-BOM, FOCUS) is gathered and signed
/// through the SAME paired `CloudHandle` Money/Overview already connect
/// (docs/PHASE4.md W3: "do NOT make a fresh handle that pairs a new
/// device"), so an unpaired Cloud makes the WHOLE Evidence Center
/// unavailable, not just its own "Cloud" source toggle. Mirrors
/// `MoneyEmptyStateView`/`CryptoEmptyStateView` field-for-field.
@MainActor
struct EvidenceEmptyStateView: View {
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
                    Text("No Cloud environment found")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.textPrimary)
                    Text(
                        "The Evidence Center builds through the same paired Cloud connection as Money and Overview. Run taipan up, or set TOKENFUSE_CLOUD_ADMIN_KEY, then revisit this tab."
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

// MARK: - formatting

/// `sha256:<hex>` -> a short, still-unambiguous display form. The full
/// digest stays available via `.textSelection(.enabled)` on the row that
/// uses this, for anyone who needs to copy/verify it in full.
enum EvidenceHashFormat {
    static func short(_ sha256: String) -> String {
        let hex = sha256.hasPrefix("sha256:") ? String(sha256.dropFirst("sha256:".count)) : sha256
        guard hex.count > 14 else { return hex }
        return "\(hex.prefix(10))\u{2026}\(hex.suffix(4))"
    }
}

/// Human-readable artifact size ("2.1 KB", "340 bytes") for the artifacts
/// table - a thin wrapper over `ByteCountFormatter` so every row shares one
/// consistent rounding/unit convention.
enum EvidenceSizeFormat {
    // `nonisolated(unsafe)`: configured once at first access and only ever
    // read afterward (`.string(fromByteCount:)`), so sharing it across
    // threads is genuinely safe even though `ByteCountFormatter` does not
    // conform to `Sendable` - mirrors `MoneyFormat`'s own formatters'
    // identical reasoning for `ISO8601DateFormatter`/`DateFormatter`.
    private nonisolated(unsafe) static let formatter: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f
    }()

    static func label(_ bytes: UInt64) -> String {
        formatter.string(fromByteCount: Int64(bytes))
    }
}
