import GenaryxCoreFFI
import SwiftUI

/// The Bus Explorer: a live list of `UiEvent`s across all six bus sources.
/// Fed by `FleetModel`, which seeds from `FleetHandle.recentEvents` and
/// prepends live pushes from the `EventListener` callback (see
/// `FleetModel.swift`).
@MainActor
struct BusExplorerView: View {
    let events: [UiEvent]

    @State private var expandedKey: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider().overlay(Theme.hairline)
            List {
                ForEach(events, id: \.rowKey) { event in
                    EventRow(
                        event: event,
                        isExpanded: expandedKey == event.rowKey,
                        onToggle: { toggle(event.rowKey) }
                    )
                    .listRowInsets(EdgeInsets(top: 5, leading: 16, bottom: 5, trailing: 16))
                    .listRowSeparator(.hidden)
                    .listRowBackground(Color.clear)
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
        }
        .background(Theme.background)
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text("Genaryx")
                .font(Theme.display(20, weight: .bold))
                .foregroundStyle(Theme.textPrimary)
            Text("BUS EXPLORER")
                .font(Theme.mono(11, weight: .semibold))
                .tracking(1.6)
                .foregroundStyle(Theme.textTertiary)
            Spacer()
            HStack(spacing: 7) {
                Circle()
                    .fill(Theme.mint)
                    .frame(width: 7, height: 7)
                    .shadow(color: Theme.mint.opacity(0.7), radius: 4)
                Text("\(events.count) events")
                    .font(Theme.mono(12, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
    }

    private func toggle(_ key: String) {
        withAnimation(.easeInOut(duration: 0.15)) {
            expandedKey = (expandedKey == key) ? nil : key
        }
    }
}

/// Stable SwiftUI list identity for one event row. `UiEvent.id` alone is
/// not safe to key on: it is the SQLite rowid for stored rows but a fixed
/// `0` for every live-pushed row (the rowid is not known on the broadcast
/// path; see `crates/ffi/README.md`), so two live rows shown together would
/// collide on `id`. Combining it with fields that vary per event keeps
/// rows distinct regardless of provenance.
extension UiEvent {
    var rowKey: String {
        "\(id)|\(ts)|\(agentId)|\(raw)"
    }
}

/// One row: severity badge, source chip, type, agent id, timestamp, and a
/// disclosure that reveals the raw NDJSON line, so every byte shown can
/// point back to its source.
@MainActor
private struct EventRow: View {
    let event: UiEvent
    let isExpanded: Bool
    let onToggle: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                SeverityBadge(severity: event.severity)
                SourceChip(source: event.source)

                Text(event.eventType)
                    .font(Theme.mono(12, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)

                Spacer(minLength: 8)

                Text(event.agentId)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.head)
                    .frame(maxWidth: 260, alignment: .trailing)

                Text(shortTime(event.ts))
                    .font(Theme.mono(11))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
                    .frame(width: 74, alignment: .trailing)

                Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
            }

            if isExpanded {
                RawJsonView(raw: event.raw)
            }
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panel)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .strokeBorder(Theme.hairline, lineWidth: 1)
        )
        .contentShape(Rectangle())
        .onTapGesture(perform: onToggle)
    }

    /// RFC 3339 down to a compact `HH:mm:ss`. Falls back to the raw string
    /// rather than force-unwrapping if the shape is ever unexpected.
    private func shortTime(_ ts: String) -> String {
        guard let tIndex = ts.firstIndex(of: "T") else { return ts }
        let afterT = ts[ts.index(after: tIndex)...]
        let end = afterT.firstIndex(of: ".") ?? afterT.firstIndex(of: "Z") ?? afterT.endIndex
        return String(afterT[..<end])
    }
}

/// Severity pill: a glowing dot in the severity's color plus its label, both
/// sitting on a low-opacity tint of that same color. Mirrors the it-rat2
/// `.pill` pattern, where label text stays neutral and only the dot and
/// border carry the hue.
@MainActor
private struct SeverityBadge: View {
    let severity: String?

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

/// Source chip: the emitting service's name on a tint of its it-rat2 brand
/// hue (see `Theme.sourceColor`).
@MainActor
private struct SourceChip: View {
    let source: String

    var body: some View {
        let color = Theme.sourceColor(source)
        Text(source)
            .font(Theme.mono(10, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(Theme.textSecondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(color.opacity(0.14)))
            .overlay(Capsule().strokeBorder(color.opacity(0.4), lineWidth: 1))
    }
}

/// The disclosure body: the original NDJSON line, pretty-printed when it
/// parses as JSON, verbatim otherwise. Never force-unwrapped.
@MainActor
private struct RawJsonView: View {
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
