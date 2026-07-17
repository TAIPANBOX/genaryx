import GenaryxCoreFFI
import SwiftUI

/// The Run Replay tab (PHASE3.md position 5, W4): pick a run - either typed
/// directly / handed down as a deep link (`focusedRunId`, from a Money run
/// row or an Agent 360 Money row via `GenaryxApp`'s own
/// `focusedApprovalId`-style tab+focus idiom, see that file's own doc), or
/// chosen from the distinct run ids already visible on the live bus
/// (`fleetModel.events`) - then step through its timeline with play/pause,
/// a scrub slider, and a speed control (`RunReplayModel`'s own "site-sim
/// scrub model" doc comment).
///
/// `events_for_run` (via `FleetModel.eventsForRun`) is the sole source this
/// wave: `CloudHandle` exposes no `/v1/replay/{run}` yet
/// (`crates/ffi/src/lib.rs`'s own doc on `events_for_run`), so a run id
/// that only exists on the Cloud side (e.g. picked from a Money row in a
/// real `taipan up` environment whose bus this console has not ingested)
/// resolves to the same honest empty state as a mistyped id - never a
/// fabricated timeline.
@MainActor
struct RunReplayView: View {
    let fleetModel: FleetModel
    let model: RunReplayModel
    /// A run id handed down from elsewhere in the app - see the type doc's
    /// first paragraph. `nil` in the ordinary case of the operator having
    /// clicked the Replay tab themselves.
    let focusedRunId: String?

    @State private var runIdField = ""

    /// How often the playback clock advances while playing - independent of
    /// `PlaybackSpeed`, which instead controls how MANY events each tick
    /// reveals (see `RunReplayModel.tick()`'s own doc).
    private static let tickInterval: Duration = .milliseconds(450)

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if let bannerMessage = model.bannerMessage {
                ErrorBannerView(message: bannerMessage)
            }
            Group {
                if model.runId == nil {
                    picker
                } else {
                    playbackContent
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .background(Theme.background)
        .task(id: focusedRunId) {
            guard let focusedRunId, !focusedRunId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                return
            }
            runIdField = focusedRunId
            await model.load(runId: focusedRunId, fleet: fleetModel)
        }
        .task(id: model.isPlaying) {
            guard model.isPlaying else { return }
            while !Task.isCancelled && model.isPlaying {
                try? await Task.sleep(for: Self.tickInterval)
                model.tick()
            }
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Text("RUN REPLAY")
                .font(Theme.mono(11, weight: .semibold))
                .tracking(1.4)
                .foregroundStyle(Theme.textTertiary)
            if let runId = model.runId {
                Text(runId)
                    .font(Theme.mono(12, weight: .semibold))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                Spacer(minLength: 8)
                Button("Change run") {
                    model.clear()
                }
                .buttonStyle(.plain)
                .font(Theme.mono(10.5, weight: .semibold))
                .foregroundStyle(Theme.iris)
            } else {
                Spacer(minLength: 0)
            }
        }
    }

    // MARK: - picker (no run loaded yet)

    private var picker: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                TextField("run_id, e.g. demo-run-000", text: $runIdField)
                    .textFieldStyle(.plain)
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textPrimary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 7)
                    .frame(width: 280)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Theme.panelElevated))
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.hairlineStrong, lineWidth: 1))
                    .onSubmit(loadTypedRunId)

                Button("Load") { loadTypedRunId() }
                    .buttonStyle(.plain)
                    .font(Theme.mono(11, weight: .semibold))
                    .foregroundStyle(canLoadTypedRunId ? Theme.iris : Theme.textTertiary)
                    .disabled(!canLoadTypedRunId)
            }

            Text("KNOWN RUNS ON THE BUS")
                .font(Theme.mono(10, weight: .semibold))
                .tracking(0.8)
                .foregroundStyle(Theme.textTertiary)

            if let unavailableMessage = fleetModel.unavailableMessage {
                Text("Core unavailable: \(unavailableMessage)")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.coral)
            } else if knownRunIds.isEmpty {
                Text("no runs observed on the bus yet.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
            } else {
                ScrollView {
                    VStack(spacing: 6) {
                        ForEach(knownRunIds, id: \.self) { runId in
                            KnownRunRow(runId: runId) {
                                runIdField = runId
                                Task { await model.load(runId: runId, fleet: fleetModel) }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Distinct `run_id`s seen on the live bus feed, in the order they were
    /// first observed among `fleetModel.events` (that array is itself
    /// newest-first - `FleetModel.swift`'s own doc - so this reads as
    /// most-recently-active run first, a reasonable default browsing
    /// order). A plain filter over already-live state, never a new FFI
    /// call - mirrors `PostureModel`'s own "computed from already-observable
    /// signals" precedent.
    private var knownRunIds: [String] {
        var seen = Set<String>()
        var ordered: [String] = []
        for event in fleetModel.events {
            guard let runId = event.runId, !runId.isEmpty, !seen.contains(runId) else { continue }
            seen.insert(runId)
            ordered.append(runId)
        }
        return ordered
    }

    private var canLoadTypedRunId: Bool {
        !runIdField.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func loadTypedRunId() {
        guard canLoadTypedRunId else { return }
        let trimmed = runIdField.trimmingCharacters(in: .whitespacesAndNewlines)
        Task { await model.load(runId: trimmed, fleet: fleetModel) }
    }

    // MARK: - playback (a run is loaded)

    @ViewBuilder
    private var playbackContent: some View {
        if model.isLoading {
            Text("loading run timeline...")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
        } else if model.events.isEmpty {
            // Only render the "legitimately empty" card when the load
            // actually SUCCEEDED with zero events. When `bannerMessage` is
            // set, the load itself failed (a genuine `FfiError`, not "this
            // run has no events") - the banner above already names the real
            // problem, so showing this card's Cloud/bus explanation
            // alongside it would contradict that message rather than
            // clarify it.
            if model.bannerMessage == nil {
                emptyState
            }
        } else {
            VStack(alignment: .leading, spacing: 14) {
                PlaybackControls(model: model)
                timeline
            }
        }
    }

    /// PHASE3 W4 brief: "honest empty state" - a run id that resolved to
    /// zero events on THIS console's own bus Store, never dressed up as a
    /// loading spinner or a silent blank screen. See the type doc's own
    /// paragraph on why a Cloud-only `runId` legitimately lands here.
    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("No events found for this run")
                .font(.system(size: 13))
                .foregroundStyle(Theme.textPrimary)
            Text(
                """
                This console's own event Store has no run with this id. Money's runs come from a \
                separate Cloud connection, so a run_id picked there may never have touched this bus - \
                Cloud /v1/replay/{run} is a second source PHASE3.md names, not yet exposed by CloudHandle.
                """
            )
            .font(Theme.mono(11.5))
            .foregroundStyle(Theme.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(16)
        .frame(maxWidth: 520, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .fill(Theme.panelElevated)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .strokeBorder(Theme.hairline, lineWidth: 1)
        )
    }

    private var timeline: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(Array(model.revealedEvents.enumerated()), id: \.element.rowKey) { index, event in
                        ReplayEventRow(event: event, isLatest: index == model.revealedEvents.count - 1)
                            .id(event.rowKey)
                        if index < model.revealedEvents.count - 1 {
                            Divider().overlay(Theme.hairline)
                        }
                    }
                }
                .background(Theme.panel)
                .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                        .strokeBorder(Theme.hairline, lineWidth: 1)
                )
            }
            .onChange(of: model.revealedCount) { _, _ in
                guard let lastKey = model.revealedEvents.last?.rowKey else { return }
                withAnimation(.easeOut(duration: 0.15)) {
                    proxy.scrollTo(lastKey, anchor: .bottom)
                }
            }
        }
    }
}

// MARK: - KnownRunRow

@MainActor
private struct KnownRunRow: View {
    let runId: String
    let onSelect: () -> Void

    var body: some View {
        Button(action: onSelect) {
            HStack(spacing: 8) {
                Text(runId)
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 8)
                Image(systemName: "play.circle")
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.iris)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
        }
        .buttonStyle(.plain)
    }
}

// MARK: - PlaybackControls

/// Play/pause, the scrub slider, the speed picker, and a small status line -
/// the "site-sim scrub model" controls (`RunReplayModel`'s own doc).
@MainActor
private struct PlaybackControls: View {
    let model: RunReplayModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 14) {
                Button {
                    model.togglePlay()
                } label: {
                    Image(systemName: model.isPlaying ? "pause.fill" : "play.fill")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(Theme.iris)
                        .frame(width: 26, height: 26)
                        .background(Circle().fill(Theme.iris.opacity(0.16)))
                        .overlay(Circle().strokeBorder(Theme.iris.opacity(0.4), lineWidth: 1))
                }
                .buttonStyle(.plain)

                Slider(
                    value: Binding(
                        get: { Double(model.revealedCount) },
                        set: { model.scrub(to: Int($0.rounded())) }
                    ),
                    in: 0...Double(max(model.events.count, 1))
                )

                Picker("speed", selection: Binding(get: { model.speed }, set: { model.speed = $0 })) {
                    ForEach(PlaybackSpeed.allCases) { speed in
                        Text(speed.label).tag(speed)
                    }
                }
                .pickerStyle(.segmented)
                .frame(width: 168)
                .labelsHidden()
            }

            HStack(spacing: 10) {
                Text("\(model.revealedCount) / \(model.events.count) events")
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                if let ts = model.currentTimestamp {
                    Text(MoneyFormat.timestamp(ts))
                        .font(Theme.mono(10.5))
                        .monospacedDigit()
                        .foregroundStyle(Theme.textTertiary)
                }
                Spacer(minLength: 0)
                if let loadedAt = model.loadedAt {
                    Text("loaded \(MoneyFormat.timestamp(ISO8601DateFormatter().string(from: loadedAt)))")
                        .font(Theme.mono(10))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
        }
    }
}

// MARK: - ReplayEventRow

@MainActor
private struct ReplayEventRow: View {
    let event: UiEvent
    let isLatest: Bool

    var body: some View {
        HStack(spacing: 10) {
            Circle()
                .fill(Theme.severityColor(event.severity))
                .frame(width: 6, height: 6)
            Text(event.source)
                .font(Theme.mono(10, weight: .semibold))
                .foregroundStyle(Theme.sourceColor(event.source))
                .frame(width: 70, alignment: .leading)
                .lineLimit(1)
            Text(event.agentId)
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(width: 190, alignment: .leading)
            Text(event.eventType)
                .font(Theme.mono(11.5, weight: isLatest ? .semibold : .regular))
                .foregroundStyle(Theme.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            Text(MoneyFormat.timestamp(event.ts))
                .font(Theme.mono(10.5))
                .monospacedDigit()
                .foregroundStyle(Theme.textTertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(isLatest ? Theme.iris.opacity(0.08) : Color.clear)
    }
}
