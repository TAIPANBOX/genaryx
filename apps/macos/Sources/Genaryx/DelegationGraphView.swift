import GenaryxCoreFFI
import SwiftUI

/// The Graph tab: the whole delegation graph, drawn from core's own laid-out
/// positions (PHASE3.md position 3 - "layout in core, dumb renderers in the
/// shells... BOTH shells draw Canvas2D", never WebGL/Metal). Fed by
/// `GraphModel` (the read) and `FleetModel` (the handle `GraphModel` calls
/// through - see `FleetModel.swift`'s own "delegation graph + Agent 360"
/// section). A tap on a node both highlights it (and its direct edges) and
/// opens its Agent 360 card via `onOpenAgent` - the deep-link entry point
/// PHASE3.md's parity checklist names ("a click on an agent from any
/// panel... opens its 360 card"); the other entry point is the Identity
/// panel's own row tap (`IdentityView.swift`).
@MainActor
struct DelegationGraphView: View {
    let fleetModel: FleetModel
    let model: GraphModel
    let onOpenAgent: (String) -> Void

    private static let refreshInterval: Duration = .seconds(20)

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if let bannerMessage = model.bannerMessage {
                ErrorBannerView(message: bannerMessage)
            }
            Group {
                if let layout = model.layout, !layout.nodes.isEmpty {
                    GraphCanvas(
                        layout: layout,
                        selectedNodeId: model.selectedNodeId,
                        onTapNode: { nodeId in
                            model.select(nodeId)
                            if let nodeId {
                                onOpenAgent(nodeId)
                            }
                        }
                    )
                } else {
                    emptyState
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .background(Theme.background)
        .task(id: fleetModel.unavailableMessage == nil) {
            guard fleetModel.unavailableMessage == nil else { return }
            await model.refresh(fleet: fleetModel)
            while !Task.isCancelled {
                try? await Task.sleep(for: Self.refreshInterval)
                await model.refresh(fleet: fleetModel)
            }
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 14) {
            Text("DELEGATION GRAPH")
                .font(Theme.mono(11, weight: .semibold))
                .tracking(1.4)
                .foregroundStyle(Theme.textTertiary)

            if let layout = model.layout {
                Text("\(layout.nodes.count) nodes \u{00B7} \(layout.edges.count) edges")
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
            }

            Spacer(minLength: 8)

            legend

            if let loadedAt = model.loadedAt {
                Text(MoneyFormat.timestamp(ISO8601DateFormatter().string(from: loadedAt)))
                    .font(Theme.mono(10))
                    .foregroundStyle(Theme.textTertiary)
            }
        }
    }

    private var legend: some View {
        HStack(spacing: 10) {
            legendSwatch("user", color: Theme.amber)
            legendSwatch("agent", color: Theme.iris)
            legendSwatch("other", color: Theme.steel)
        }
    }

    private func legendSwatch(_ label: String, color: Color) -> some View {
        HStack(spacing: 5) {
            Circle().fill(color).frame(width: 7, height: 7)
            Text(label)
                .font(Theme.mono(10))
                .foregroundStyle(Theme.textTertiary)
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        VStack {
            Spacer(minLength: 0)
            HStack {
                Spacer(minLength: 0)
                content
                Spacer(minLength: 0)
            }
            Spacer(minLength: 0)
        }
    }

    @ViewBuilder
    private var content: some View {
        if let unavailableMessage = fleetModel.unavailableMessage {
            Text("Core unavailable: \(unavailableMessage)")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.coral)
        } else if model.isLoading && model.layout == nil {
            Text("loading the delegation graph...")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
        } else {
            Text("no delegation activity on the bus yet.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
        }
    }
}

// MARK: - GraphCanvas

/// The dumb Canvas2D renderer over one `LayoutViewRecord`: nodes as circles
/// (color by `NodeKind`, radius by `eventCount`), directed edges as
/// arrow-terminated lines, short-form URI labels. Pan (drag) + zoom
/// (pinch/magnification), tap-to-select. All drawing and hit-testing use the
/// SAME explicit screen<->logical transform (`point(for:)` /
/// `handleTap(at:)`'s inverse) rather than `GraphicsContext`'s own CTM
/// stack, so the two can never drift apart from a transform-order mistake.
@MainActor
private struct GraphCanvas: View {
    let layout: LayoutViewRecord
    let selectedNodeId: String?
    /// `nil` when the tap hit no node (background tap) - deselects.
    let onTapNode: (String?) -> Void

    @State private var scale: CGFloat = 1
    @State private var liveScale: CGFloat = 1
    @State private var offset: CGSize = .zero
    @State private var liveOffset: CGSize = .zero
    @State private var hasFitToView = false

    private static let minScale: CGFloat = 0.15
    private static let maxScale: CGFloat = 6
    /// Drag distance (points) below which a `DragGesture(minimumDistance:
    /// 0)` release is treated as a tap rather than a pan.
    private static let tapSlack: CGFloat = 6
    private static let baseRadius: CGFloat = 7
    /// Below this zoom level labels are skipped so a large graph does not
    /// turn into overlapping text - a deliberate declutter simplification,
    /// not a bug (see this file's own report note on residual gaps).
    private static let labelMinScale: CGFloat = 0.35

    var body: some View {
        GeometryReader { geo in
            Canvas { context, _ in
                draw(context: &context)
            }
            .frame(width: geo.size.width, height: geo.size.height)
            .contentShape(Rectangle())
            .gesture(panAndTapGesture)
            .simultaneousGesture(zoomGesture)
            .onAppear { fitToView(size: geo.size) }
        }
        .background(Theme.panel)
        .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .strokeBorder(Theme.hairline, lineWidth: 1)
        )
    }

    // MARK: - drawing (screen space; see `point(for:)`)

    private func draw(context: inout GraphicsContext) {
        let nodesById = Dictionary(uniqueKeysWithValues: layout.nodes.map { ($0.id, $0) })

        for edge in layout.edges {
            guard let from = nodesById[edge.from], let to = nodesById[edge.to] else { continue }
            let highlighted = selectedNodeId != nil && (edge.from == selectedNodeId || edge.to == selectedNodeId)
            drawEdge(
                context: &context, from: point(for: from), to: point(for: to), toRadius: screenRadius(for: to),
                highlighted: highlighted)
        }
        for node in layout.nodes {
            drawNode(context: &context, node: node, isSelected: node.id == selectedNodeId)
        }
    }

    private func drawNode(context: inout GraphicsContext, node: PositionedNodeRecord, isSelected: Bool) {
        let center = point(for: node)
        let r = screenRadius(for: node)
        let rect = CGRect(x: center.x - r, y: center.y - r, width: r * 2, height: r * 2)
        let base = color(for: node.kind)
        context.fill(Path(ellipseIn: rect), with: .color(base.opacity(isSelected ? 0.85 : 0.5)))
        context.stroke(
            Path(ellipseIn: rect), with: .color(isSelected ? Theme.amber : base.opacity(0.75)),
            lineWidth: isSelected ? 2.5 : 1)

        guard liveScale >= Self.labelMinScale else { return }
        context.draw(
            Text(Self.shortLabel(node.id))
                .font(Theme.mono(isSelected ? 10.5 : 9, weight: isSelected ? .semibold : .regular))
                .foregroundStyle(isSelected ? Theme.textPrimary : Theme.textSecondary),
            at: CGPoint(x: center.x, y: center.y + r + 3),
            anchor: .top
        )
    }

    private func drawEdge(
        context: inout GraphicsContext, from: CGPoint, to: CGPoint, toRadius: CGFloat, highlighted: Bool
    ) {
        let dx = to.x - from.x
        let dy = to.y - from.y
        let dist = max(1, (dx * dx + dy * dy).squareRoot())
        let ux = dx / dist
        let uy = dy / dist
        let stop = CGPoint(x: to.x - ux * toRadius, y: to.y - uy * toRadius)

        var line = Path()
        line.move(to: from)
        line.addLine(to: stop)
        let tint = highlighted ? Theme.amber : Theme.textTertiary
        context.stroke(line, with: .color(tint.opacity(highlighted ? 0.9 : 0.3)), lineWidth: highlighted ? 2 : 1)

        // Arrowhead: a small filled triangle at `stop`, pointing along (ux,uy).
        let arrowLength: CGFloat = 7
        let arrowWidth: CGFloat = 3.5
        let backX = stop.x - ux * arrowLength
        let backY = stop.y - uy * arrowLength
        let px = -uy
        let py = ux
        var arrow = Path()
        arrow.move(to: stop)
        arrow.addLine(to: CGPoint(x: backX + px * arrowWidth, y: backY + py * arrowWidth))
        arrow.addLine(to: CGPoint(x: backX - px * arrowWidth, y: backY - py * arrowWidth))
        arrow.closeSubpath()
        context.fill(arrow, with: .color(tint.opacity(highlighted ? 0.95 : 0.4)))
    }

    private func color(for kind: NodeKind) -> Color {
        switch kind {
        case .user: return Theme.amber
        case .agent: return Theme.iris
        case .other: return Theme.steel
        }
    }

    /// The last, non-empty path component after the `scheme://` - e.g.
    /// `agent://taipanbox.dev/demo/tier1-bot` -> `tier1-bot`,
    /// `user://taipanbox.dev/j.doe` -> `j.doe`. Falls back to the full id
    /// when it does not parse as `scheme://...`, never a crash.
    private static func shortLabel(_ id: String) -> String {
        guard let schemeRange = id.range(of: "://") else { return id }
        let afterScheme = id[schemeRange.upperBound...]
        guard let last = afterScheme.split(separator: "/").last, !last.isEmpty else { return id }
        return String(last)
    }

    // MARK: - screen <-> logical transform

    private func point(for node: PositionedNodeRecord) -> CGPoint {
        CGPoint(
            x: node.x * Double(liveScale) + Double(liveOffset.width),
            y: node.y * Double(liveScale) + Double(liveOffset.height)
        )
    }

    private func logicalRadius(for node: PositionedNodeRecord) -> CGFloat {
        Self.baseRadius + CGFloat(min(20.0, Double(node.eventCount).squareRoot() * 2.2))
    }

    private func screenRadius(for node: PositionedNodeRecord) -> CGFloat {
        logicalRadius(for: node) * liveScale
    }

    private func fitToView(size: CGSize) {
        guard !hasFitToView, size.width > 0, size.height > 0, layout.width > 0, layout.height > 0 else { return }
        hasFitToView = true
        let fit = min(size.width / CGFloat(layout.width), size.height / CGFloat(layout.height)) * 0.92
        scale = clampScale(fit)
        liveScale = scale
        let contentWidth = CGFloat(layout.width) * scale
        let contentHeight = CGFloat(layout.height) * scale
        offset = CGSize(width: (size.width - contentWidth) / 2, height: (size.height - contentHeight) / 2)
        liveOffset = offset
    }

    // MARK: - gestures

    private var panAndTapGesture: some Gesture {
        DragGesture(minimumDistance: 0)
            .onChanged { value in
                liveOffset = CGSize(
                    width: offset.width + value.translation.width,
                    height: offset.height + value.translation.height
                )
            }
            .onEnded { value in
                let moved = max(abs(value.translation.width), abs(value.translation.height))
                if moved < Self.tapSlack {
                    liveOffset = offset  // revert the tiny jitter; this was a tap, not a pan
                    handleTap(at: value.startLocation)
                } else {
                    offset = liveOffset
                }
            }
    }

    private var zoomGesture: some Gesture {
        MagnificationGesture()
            .onChanged { value in
                liveScale = clampScale(scale * value)
            }
            .onEnded { value in
                scale = clampScale(scale * value)
                liveScale = scale
            }
    }

    private func clampScale(_ value: CGFloat) -> CGFloat {
        min(max(value, Self.minScale), Self.maxScale)
    }

    /// Nearest-node hit test in logical space (the inverse of `point(for:)`).
    /// A miss (background tap, or outside every node's padded radius) reports
    /// `nil` so the caller can clear the current selection.
    private func handleTap(at screenPoint: CGPoint) {
        let safeScale = max(liveScale, 0.0001)
        let logical = CGPoint(
            x: (screenPoint.x - liveOffset.width) / safeScale,
            y: (screenPoint.y - liveOffset.height) / safeScale
        )

        var bestNode: PositionedNodeRecord?
        var bestDist = Double.greatestFiniteMagnitude
        for node in layout.nodes {
            let dx = node.x - Double(logical.x)
            let dy = node.y - Double(logical.y)
            let d = (dx * dx + dy * dy).squareRoot()
            if d < bestDist {
                bestDist = d
                bestNode = node
            }
        }

        if let bestNode {
            let tapSlackLogical = Double(Self.tapSlack) / Double(safeScale)
            if bestDist <= Double(logicalRadius(for: bestNode)) + tapSlackLogical {
                onTapNode(bestNode.id)
                return
            }
        }
        onTapNode(nil)
    }
}
