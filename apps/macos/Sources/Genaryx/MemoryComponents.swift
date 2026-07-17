import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Memory view: the "not ready yet" empty
/// state, "as of" formatting for stats vs recall, and a raw-JSON disclosure
/// for the `engram.*` Timeline section. Mirrors `CryptoComponents.swift`'s
/// own role and distribution: view-specific sections (Store Stats, Recall,
/// Provenance, Timeline) live in `MemoryView.swift`, exactly where
/// `NcscTimelineHero`/`EvidenceSection` live in `CryptoView.swift` rather
/// than here.

// MARK: - MemoryEmptyStateView

/// Shared "not ready yet" rendering for the Memory view: three honest,
/// distinct states, plus the docs/PHASE4.md-mandated clean "no memory plane"
/// outcome for a box with no `engram-mcp` binary at all. Mirrors
/// `CryptoEmptyStateView` field-for-field, swapped to `MemoryConnection`.
@MainActor
struct MemoryEmptyStateView: View {
    let connection: MemoryConnection

    var body: some View {
        centered {
            switch connection {
            case .connecting:
                Text("connecting to a memory plane...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)

            case .noEnvironment:
                card {
                    Text("No memory plane found")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.textPrimary)
                    Text(
                        "No engram-mcp found on PATH, in a venv, or at ~/.taipan/bin/engram-mcp. Install engdbram (pip install engdbram) so its engram-mcp console script is reachable."
                    )
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                }

            case .connectFailed(let reason):
                card {
                    Text("Could not start engram-mcp")
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

// MARK: - "as of" formatting

/// The Memory panel carries TWO independent "as of" clocks - `stats` (a
/// background-refreshed read) and `recall` (an explicit operator query) -
/// deliberately never sharing one formatter, the same reasoning
/// `LastScanFormat`/`LoadedAtFormat` split on (`CryptoComponents.swift`'s own
/// doc: "a shared label would blur a real distinction the operator should be
/// able to see").
enum MemoryStatsFormat {
    static func label(_ date: Date?) -> String {
        guard let date else { return "not yet loaded" }
        let iso = ISO8601DateFormatter().string(from: date)
        return "as of \u{00B7} \(MoneyFormat.timestamp(iso))"
    }
}

enum RecallFormat {
    static func label(_ date: Date?) -> String {
        guard let date else { return "no query yet" }
        let iso = ISO8601DateFormatter().string(from: date)
        return "as of last query \u{00B7} \(MoneyFormat.timestamp(iso))"
    }
}

// MARK: - UiEvent raw disclosure (Timeline)

/// The `engram.*` Timeline (docs/PHASE4.md W2: `memory_written` /
/// `memory_forgotten` / `reflection_run` / `contradiction_found`, each with
/// its own `data.*` shape none of which PHASE4.md's own grounding enumerates
/// field-by-field - unlike `quality_drift`/wardryx's decision fields). Rather
/// than guess at field names this crate has no grounding for, this section
/// renders the envelope's own typed fields (already real:
/// severity/type/agent/time) plus a tap-to-expand raw JSON disclosure for
/// full detail - a small, self-contained copy of
/// `BusExplorerView.swift`'s own private `RawJsonView`/`rowKey` (that file's
/// versions are file-private, so this mirrors `SeverityPill`'s own
/// "separate small copy... so the Bus Explorer file stays completely
/// untouched" precedent, `MoneyComponents.swift`'s own doc).
@MainActor
struct TimelineRawJsonView: View {
    let raw: String

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Text(prettyPrinted)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textSecondary)
                .textSelection(.enabled)
                .padding(10)
        }
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panelElevated)
        )
    }

    private var prettyPrinted: String {
        guard
            let data = raw.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data),
            let prettyData = try? JSONSerialization.data(
                withJSONObject: object,
                options: [.prettyPrinted, .sortedKeys]
            ),
            let text = String(data: prettyData, encoding: .utf8)
        else {
            return raw
        }
        return text
    }
}
