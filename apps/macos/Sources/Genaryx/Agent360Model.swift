import Foundation
import GenaryxCoreFFI
import Observation

/// The two Agent 360 planes with no existing client-side cache to filter:
/// the delegation neighborhood (`FleetHandle.agentSlice`) and this agent's
/// own event history (`FleetHandle.eventsForAgent`) - PHASE3 W3. Every other
/// plane the card shows - Identity, Money, Policy - is a plain filter over
/// state `IdentityModel`/`CloudModel`/`PolicyModel`/`FleetModel` already
/// hold live, computed directly in `Agent360View`'s body (mirrors
/// `PostureModel`'s own "no new read, just a pure function of already-live
/// state" precedent - see `PostureModel.swift`'s type doc); only these two
/// need a fresh FFI call of their own, so this model carries only them.
@MainActor
@Observable
final class Agent360Model {
    let agentId: String

    private(set) var slice: AgentSliceRecord?
    private(set) var events: [UiEvent] = []
    private(set) var isLoading = false
    private(set) var bannerMessage: String?
    private(set) var loadedAt: Date?

    private static let eventsLimit: UInt32 = 50

    init(agentId: String) {
        self.agentId = agentId
    }

    /// Loads both planes in parallel through `fleet` (the same `FleetHandle`
    /// `FleetModel` already owns - see that file's own doc). Fail-closed: a
    /// thrown `FfiError` becomes `bannerMessage`; `slice`/`events` keep
    /// their last-known-good values rather than flashing to empty on a
    /// transient failure.
    func refresh(fleet: FleetModel) async {
        isLoading = true
        defer { isLoading = false }
        do {
            async let sliceLoad = fleet.agentSlice(agentId)
            async let eventsLoad = fleet.eventsForAgent(agentId, limit: Self.eventsLimit)
            let (loadedSlice, loadedEvents) = try await (sliceLoad, eventsLoad)
            slice = loadedSlice
            events = loadedEvents
            loadedAt = Date()
            bannerMessage = nil
        } catch {
            bannerMessage = String(describing: error)
        }
    }
}
