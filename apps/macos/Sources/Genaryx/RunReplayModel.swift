import Foundation
import GenaryxCoreFFI
import Observation

/// Playback speed for the Run Replay scrub clock (PHASE3.md position 5: "a
/// playback clock... scrub/speed, the mental model of the site sims"). Each
/// case names how many of the run's events the clock reveals per driving
/// tick (`RunReplayView`'s own `.task` loop, `RunReplayModel.tick()`) - a
/// discrete "events per tick" multiplier rather than a continuous
/// wall-clock rate, since the underlying timeline is a discrete event list,
/// not a continuous signal.
enum PlaybackSpeed: Int, CaseIterable, Identifiable {
    case x1 = 1
    case x2 = 2
    case x4 = 4
    case x8 = 8

    var id: Int { rawValue }
    var label: String { "\(rawValue)x" }
}

/// Live state for the Run Replay tab (PHASE3.md position 5, W4): one run's
/// full timeline (`FleetHandle.eventsForRun`, via `FleetModel.eventsForRun`
/// - the sole source this wave; `CloudHandle` exposes no
/// `/v1/replay/{run}` yet, so this model never reaches for one - see
/// `crates/ffi/src/lib.rs`'s own doc comment on `events_for_run`) plus a
/// client-side playback clock (play/pause, scrub, speed) over it - PHASE3.md's
/// "site-sim scrub model": like a strategy-game time control, the operator
/// either drags a scrubber to any point in the run's history or lets it
/// play forward at a chosen speed, and the view reveals events up to that
/// position.
///
/// Owned once by `GenaryxApp` (mirrors `GraphModel`'s own precedent, not
/// `Agent360Model`'s per-presentation one) so the operator's place in a
/// run's playback survives switching to another tab and back.
///
/// `events` is re-sorted by `ts` after loading, never trusted in the order
/// `FleetHandle.eventsForRun` returns it: that call is oldest-first by
/// SQLite `id` (insertion order), which `crates/ffi/src/lib.rs`'s own doc
/// comment on `events_for_run` explains is a source-registration-order
/// guarantee, not a wall-clock guarantee, for a run whose calls span more
/// than one bus source (a common case - see `crates/core/src/demo.rs`'s own
/// block runs, and `crates/ffi/src/lib.rs`'s own
/// `events_for_run_over_the_demo_campaign_is_oldest_first_by_id` test). A
/// faithful playback clock has to scrub through actual elapsed time, so
/// this model does the `ts` sort itself rather than propagating that
/// subtlety into the view.
@MainActor
@Observable
final class RunReplayModel {
    private(set) var runId: String?
    /// The run's full timeline, sorted by `ts` ascending - see the type doc.
    private(set) var events: [UiEvent] = []
    private(set) var isLoading = false
    private(set) var bannerMessage: String?
    private(set) var loadedAt: Date?

    /// How many of `events`, from the start, are "revealed" - the scrub
    /// position. `0` means nothing revealed yet (playback has not started
    /// / the operator scrubbed to the very beginning); `events.count` means
    /// the whole run is revealed (playback finished, or the operator
    /// scrubbed to the end).
    private(set) var revealedCount = 0
    private(set) var isPlaying = false
    var speed: PlaybackSpeed = .x1

    /// Cap on how many of a (pathologically long) run's events this view
    /// ever loads. Sized up from `Agent360Model.eventsLimit`'s 50: a replay
    /// is meant to show a run's WHOLE story, not just its most recent
    /// slice, but still bounded (`Store::events_for_run`'s own `limit`
    /// param exists for exactly this - "caps a pathologically long run").
    private static let limit: UInt32 = 500

    /// (Re)load one run's timeline, replacing whatever was previously
    /// loaded (including a different run's data - unlike `GraphModel`/
    /// `Agent360Model`'s "last known good over blank" posture on a
    /// transient failure, a stale PREVIOUS run's timeline left on screen
    /// under a NEW run's id would be actively misleading, not just
    /// outdated: there is no "last known good" to preserve across a run
    /// change the way there is across a periodic refresh of the SAME run).
    /// Fail-closed: a thrown `FfiError` clears `events` and sets
    /// `bannerMessage`, never leaves a half-loaded or fabricated timeline.
    func load(runId: String, fleet: FleetModel) async {
        let trimmed = runId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        self.runId = trimmed
        events = []
        revealedCount = 0
        pause()
        isLoading = true
        defer { isLoading = false }
        do {
            let loaded = try await fleet.eventsForRun(trimmed, limit: Self.limit)
            events = loaded.sorted { lhs, rhs in
                (Self.parseTimestamp(lhs.ts) ?? .distantPast) < (Self.parseTimestamp(rhs.ts) ?? .distantPast)
            }
            loadedAt = Date()
            bannerMessage = nil
        } catch {
            bannerMessage = String(describing: error)
        }
    }

    /// Return to the run picker - clears every loaded/playback field so a
    /// stale run's data can never bleed into the next pick.
    func clear() {
        pause()
        runId = nil
        events = []
        revealedCount = 0
        loadedAt = nil
        bannerMessage = nil
    }

    // MARK: - playback clock

    func play() {
        guard !events.isEmpty else { return }
        if revealedCount >= events.count {
            revealedCount = 0  // replay from the start once it has already run off the end
        }
        isPlaying = true
    }

    func pause() {
        isPlaying = false
    }

    func togglePlay() {
        isPlaying ? pause() : play()
    }

    /// Advance the scrub position by one speed-tick's worth of events -
    /// called from `RunReplayView`'s own driving `.task` loop, never a
    /// `Timer`/`Task` owned here (mirrors every other panel's
    /// `.task { while ... }` cadence convention - see
    /// `PostureView`/`MoneyView`/`PolicyView`'s own `.task`s - rather than
    /// this model scheduling its own work).
    func tick() {
        guard isPlaying else { return }
        revealedCount = min(events.count, revealedCount + speed.rawValue)
        if revealedCount >= events.count {
            pause()
        }
    }

    /// Scrub directly to `index` (clamped to the valid range), pausing
    /// playback - a manual drag always wins over the auto-advance clock.
    func scrub(to index: Int) {
        revealedCount = max(0, min(index, events.count))
        pause()
    }

    /// The revealed slice of `events`, chronological (oldest first) - what
    /// the timeline list renders.
    var revealedEvents: [UiEvent] {
        Array(events.prefix(revealedCount))
    }

    /// The wall-clock instant of the most recently revealed event, or the
    /// run's start instant before playback begins - the "sim clock"
    /// reading shown next to the scrubber. `nil` only when `events` itself
    /// is empty.
    var currentTimestamp: String? {
        guard !events.isEmpty else { return nil }
        if revealedCount == 0 { return events.first?.ts }
        return events[min(revealedCount, events.count) - 1].ts
    }

    // A dedicated small ISO8601 parsing helper, matching `PostureModel`'s
    // own precedent of one small parsing helper per file rather than a
    // shared central one (see that file's own doc comment for why).
    private nonisolated(unsafe) static let isoFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private nonisolated(unsafe) static let isoFormatterNoFraction: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    private static func parseTimestamp(_ iso: String) -> Date? {
        isoFormatter.date(from: iso) ?? isoFormatterNoFraction.date(from: iso)
    }
}
