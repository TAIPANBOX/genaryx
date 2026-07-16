import AppKit
import SwiftUI

/// Genaryx: the native macOS SwiftUI shell (decision D2) over `genaryx-core`.
///
/// Phase 0 scaffold: `events` is a static mock array (`MockData.swift`). The
/// follow-up task replaces this `let` with `@State`, or an `@Observable` view
/// model, fed by the UniFFI-generated binding over
/// `IngestService::subscribe()` (see the doc comment on `UiEvent` for the
/// exact bridge point); at that point the Bus Explorer starts rendering the
/// real broadcast stream instead of this constant.
@main
struct GenaryxApp: App {
    private let events: [UiEvent] = MockData.events

    var body: some Scene {
        WindowGroup("Genaryx") {
            BusExplorerView(events: events)
                .frame(minWidth: 760, minHeight: 520)
        }

        MenuBarExtra {
            MenuBarBusView(events: events)
        } label: {
            MenuBarLabel(count: events.count)
        }
        .menuBarExtraStyle(.window)
    }
}

/// The `NSStatusItem` label: a small glyph plus the live event count, kept
/// terse since the status bar has very little horizontal room.
@MainActor
private struct MenuBarLabel: View {
    let count: Int

    var body: some View {
        Label {
            Text("\(count)")
                .monospacedDigit()
        } icon: {
            Image(systemName: "waveform.path.ecg")
        }
    }
}

/// The MenuBarExtra's popover content: a condensed, read-only slice of the
/// bus feed, so a quick glance never requires opening the main window.
@MainActor
private struct MenuBarBusView: View {
    let events: [UiEvent]

    private static let previewCount = 8

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Genaryx Bus")
                    .font(Theme.display(13, weight: .bold))
                    .foregroundStyle(Theme.textPrimary)
                Spacer()
                HStack(spacing: 6) {
                    Circle()
                        .fill(Theme.mint)
                        .frame(width: 6, height: 6)
                    Text("\(events.count)")
                        .font(Theme.mono(12, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(Theme.textSecondary)
                }
            }
            .padding(12)

            Divider().overlay(Theme.hairline)

            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(events.prefix(Self.previewCount)) { event in
                        MenuBarEventLine(event: event)
                    }
                }
                .padding(12)
            }
            .frame(maxHeight: 300)

            Divider().overlay(Theme.hairline)

            HStack {
                Text("mock data, UniFFI bridge pending")
                    .font(Theme.mono(10))
                    .foregroundStyle(Theme.textTertiary)
                Spacer()
                Button("Quit") {
                    NSApplication.shared.terminate(nil)
                }
                .buttonStyle(.plain)
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.textSecondary)
            }
            .padding(12)
        }
        .frame(width: 320)
        .background(Theme.background)
    }
}

/// One condensed row in the menu-bar quick view: severity dot, source, type.
@MainActor
private struct MenuBarEventLine: View {
    let event: UiEvent

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(Theme.severityColor(event.severity))
                .frame(width: 6, height: 6)
            Text(event.source)
                .font(Theme.mono(10, weight: .semibold))
                .foregroundStyle(Theme.sourceColor(event.source))
                .frame(width: 64, alignment: .leading)
                .lineLimit(1)
            Text(event.type_)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
        }
    }
}
