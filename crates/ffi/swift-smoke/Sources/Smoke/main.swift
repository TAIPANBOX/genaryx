// Phase-0 spike 1 smoke: Swift calls the real genaryx-core through the
// UniFFI binding. Proves, in order: (1) constructing a FleetHandle seeds and
// ingests the demo campaign; (2) recentEvents() returns real stored rows;
// (3) at least one live event arrives via the EventListener callback pushed
// from the Rust ingest thread; (4) the store keeps growing while we watch
// (the reader connection observes the writer's WAL commits).
//
// Run via ../build-smoke.sh. Exit code 0 = every check passed.

import Foundation
import GenaryxCoreFFI

/// Collects pushed events. Called on the Rust ingest thread, hence the lock;
/// the generated EventListener protocol requires Sendable.
final class CountingListener: EventListener, @unchecked Sendable {
    private let lock = NSLock()
    private var events: [UiEvent] = []

    func onEvent(event: UiEvent) {
        lock.lock()
        defer { lock.unlock() }
        events.append(event)
    }

    var snapshot: [UiEvent] {
        lock.lock()
        defer { lock.unlock() }
        return events
    }
}

func line(_ e: UiEvent) -> String {
    "id=\(e.id) [\(e.severity ?? "-")] \(e.source)/\(e.eventType) "
        + "\(e.agentId) run=\(e.runId ?? "-") ts=\(e.ts)"
}

do {
    print("smoke: constructing FleetHandle (demo world + ingest + feeder)")
    let handle = try FleetHandle()

    let primedCount = try handle.eventCount()
    let recent = try handle.recentEvents(limit: 5)
    print("smoke: eventCount() = \(primedCount) stored events after priming")
    print("smoke: recentEvents(limit: 5) -> \(recent.count) events, newest first:")
    for e in recent {
        print("  \(line(e))")
    }

    let listener = CountingListener()
    handle.subscribe(listener: listener)
    print("smoke: subscribed EventListener; feeder appends ~1 conforming line/s")

    // Bounded wait: stop early once three live pushes arrived, give up at 8s.
    let deadline = Date().addingTimeInterval(8)
    while Date() < deadline && listener.snapshot.count < 3 {
        Thread.sleep(forTimeInterval: 0.2)
    }

    let live = listener.snapshot
    print("smoke: \(live.count) live events pushed via callback:")
    for e in live.prefix(5) {
        print("  \(line(e))")
    }

    let finalCount = try handle.eventCount()
    print("smoke: eventCount() = \(finalCount) after the live window")

    let ok = recent.count == 5 && !live.isEmpty && finalCount > primedCount
    if ok {
        print("smoke: PASS (history + live push + growing store, all through FFI)")
        exit(0)
    } else {
        print(
            "smoke: FAIL (recent=\(recent.count)/5, live=\(live.count), "
                + "count \(primedCount) -> \(finalCount))")
        exit(1)
    }
} catch {
    print("smoke: FAIL with thrown error: \(error)")
    exit(1)
}
