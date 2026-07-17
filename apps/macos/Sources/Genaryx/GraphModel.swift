import Foundation
import GenaryxCoreFFI
import Observation

/// Live state for the Graph tab: the delegation graph's current layout
/// (PHASE3 W3 position 3 - layout is computed once in core,
/// `FleetHandle.agentGraph`/`FleetModel.agentGraph`, this model just holds
/// the latest result) plus the tap-to-highlight selection the Canvas2D
/// renderer reports. No connection enum unlike `CloudModel`/`PolicyModel`/
/// `IdentityModel`: this reads through the SAME `FleetHandle` `FleetModel`
/// already constructed at launch (there is no separate pairing/discovery
/// step of its own - see `FleetModel.swift`'s own "delegation graph + Agent
/// 360" section), so the only failure mode is a per-call `FfiError`,
/// rendered as a plain banner exactly like every other panel's
/// `bannerMessage`.
@MainActor
@Observable
final class GraphModel {
    private(set) var layout: LayoutViewRecord?
    private(set) var isLoading = false
    private(set) var bannerMessage: String?
    private(set) var loadedAt: Date?

    /// Tap-to-highlight (PHASE3 W3): the node id the operator last tapped in
    /// `DelegationGraphView`'s Canvas2D renderer - a pure rendering hint,
    /// never a fetch of its own. Cleared automatically when a refresh's
    /// fresh node set no longer contains it (the agent may have aged out of
    /// the graph), kept otherwise so a highlight survives a routine
    /// background refresh rather than resetting every ~20s.
    private(set) var selectedNodeId: String?

    func select(_ nodeId: String?) {
        selectedNodeId = nodeId
    }

    /// (Re)load the graph. Fail-closed: a thrown `FfiError` becomes
    /// `bannerMessage`, never a crash; `layout` keeps its last-known-good
    /// value on a transient failure rather than flashing to empty (mirrors
    /// `FleetModel`'s own "last known good over blank" posture the menu-bar
    /// burn readout already documents).
    func refresh(fleet: FleetModel) async {
        isLoading = true
        defer { isLoading = false }
        do {
            let loaded = try await fleet.agentGraph()
            if let selectedNodeId, !loaded.nodes.contains(where: { $0.id == selectedNodeId }) {
                self.selectedNodeId = nil
            }
            layout = loaded
            loadedAt = Date()
            bannerMessage = nil
        } catch {
            bannerMessage = String(describing: error)
        }
    }
}
