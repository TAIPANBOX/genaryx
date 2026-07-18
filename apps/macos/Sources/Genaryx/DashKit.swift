import GenaryxCoreFFI
import SwiftUI

/// The native dashboard kit: the SwiftUI counterpart of the Tauri shell's
/// `components/dash.tsx`. Every panel composes its dashboard from these
/// primitives (hero + KPI tiles + fuse bars + sparkline + feeds + composition)
/// so the whole app shares one modern, readable, "fuse" visual language over
/// the same `Theme` tokens. Hand-rolled Canvas/Path charts, no dependency.

// MARK: - Fuse tone

/// The budget-health ramp: mint (healthy) -> amber (warming) -> ember (over) ->
/// iris (the interactive/router accent). Drives the fuse bar gradient + glow.
enum FuseTone {
    case mint, amber, ember, iris

    var gradient: [Color] {
        switch self {
        case .mint: return [Color(hex: 0x2BB98C), Theme.mint]
        case .amber: return [Theme.mint, Theme.amber]
        case .ember: return [Theme.amber, Theme.ember]
        case .iris: return [Color(hex: 0x4C5AE0), Theme.iris]
        }
    }

    var glow: Color? {
        switch self {
        case .amber: return Theme.amber.opacity(0.45)
        case .ember: return Theme.ember.opacity(0.55)
        case .iris: return Theme.iris.opacity(0.4)
        case .mint: return nil
        }
    }

    static func forFraction(_ f: Double) -> FuseTone {
        f >= 1 ? .ember : (f >= 0.8 ? .amber : .mint)
    }
}

// MARK: - Card chrome

extension View {
    /// The soft gradient card every dashboard surface uses (Theme.Radius.card).
    func dashCard() -> some View {
        self
            .background(
                LinearGradient(
                    colors: [Theme.panelElevated, Theme.panel],
                    startPoint: .top, endPoint: .bottom)
            )
            .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                    .strokeBorder(Theme.hairline, lineWidth: 1)
            )
    }
}

// MARK: - FuseBar

/// The fuse heat bar: a track with a gradient fill at `fraction` width, tinted
/// mint/amber/ember/iris and glowing when hot.
@MainActor
struct FuseBar: View {
    let fraction: Double
    var tone: FuseTone?
    var height: CGFloat = 8

    var body: some View {
        let t = tone ?? FuseTone.forFraction(fraction)
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(Theme.background)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .strokeBorder(Theme.hairline, lineWidth: 1))
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(LinearGradient(colors: t.gradient, startPoint: .leading, endPoint: .trailing))
                    .frame(width: max(0, min(1, fraction)) * geo.size.width)
                    .shadow(color: t.glow ?? .clear, radius: t.glow != nil ? 6 : 0)
            }
        }
        .frame(height: height)
    }
}

// MARK: - Sparkline

/// Hand-rolled area+line sparkline (Path), theme-colored, endpoint dot.
@MainActor
struct Sparkline: View {
    let values: [Double]
    var stroke: Color = Theme.amber
    var fill: Color? = nil
    var dot: Color = Theme.ember
    var height: CGFloat = 72

    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width
            let h = geo.size.height
            let pad: CGFloat = 8
            let maxV = max(1, values.max() ?? 1)
            let pts: [CGPoint] = values.enumerated().map { i, v in
                let x = values.count <= 1 ? 0 : CGFloat(i) / CGFloat(values.count - 1) * w
                let y = h - CGFloat(v / maxV) * (h - pad * 2) - pad
                return CGPoint(x: x, y: y)
            }
            ZStack {
                if pts.count >= 2 {
                    Path { p in
                        p.move(to: CGPoint(x: 0, y: h))
                        for pt in pts { p.addLine(to: pt) }
                        p.addLine(to: CGPoint(x: w, y: h))
                        p.closeSubpath()
                    }
                    .fill(fill ?? stroke.opacity(0.18))
                    Path { p in
                        p.move(to: pts[0])
                        for pt in pts.dropFirst() { p.addLine(to: pt) }
                    }
                    .stroke(stroke, style: StrokeStyle(lineWidth: 2.5, lineCap: .round, lineJoin: .round))
                    if let last = pts.last {
                        Circle().fill(dot).frame(width: 7, height: 7).position(last)
                    }
                }
            }
        }
        .frame(height: height)
    }
}

// MARK: - KpiTile

/// A big-number KPI tile: uppercase label, large tabular value, optional sub.
@MainActor
struct KpiTile: View {
    let label: String
    let value: String
    var sub: String?
    var tone: Color?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label.uppercased())
                .font(Theme.mono(10, weight: .semibold))
                .tracking(1.1)
                .foregroundStyle(Theme.textTertiary)
            Text(value)
                .font(Theme.display(30, weight: .bold))
                .monospacedDigit()
                .foregroundStyle(tone ?? Theme.textPrimary)
                .lineLimit(1)
                .minimumScaleFactor(0.55)
            if let sub {
                Text(sub)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 18)
        .padding(.vertical, 16)
        .dashCard()
    }
}

// MARK: - FreshBadge

/// The freshness-grammar pill every dashboard section header can wear: a
/// small mono-uppercase capsule naming exactly where a card's data comes
/// from and how stale it can be, so no card ever "looks live" by accident.
/// Mirrors the Tauri shell's own `.d-badge` pill (see
/// `/Users/factory/Development/itrat-console/11-design-spec-chesna-svizhist.md`
/// section 3, "Граматика свіжості"). Declared as an enum so the case list
/// itself is the whole freshness vocabulary - a view can never construct a
/// badge that isn't one of these six honest states.
@MainActor
enum FreshBadge: View {
    /// Push-stream data (the ~2s bus feed). The only state whose dot pulses.
    case live
    /// A scheduled REST poll, firing every `period` (e.g. "20s").
    case auto(period: String)
    /// Data frozen since the last explicit load; `at` is that load's clock
    /// time (e.g. "14:32"). Click target for a Rescan/Refresh in the spec;
    /// this view is display-only, callers wire the action separately
    /// (`DashSection`'s own `onRefresh`).
    case snapshot(at: String)
    /// Data fetched only when the operator explicitly asks for it (Scan,
    /// Run, Build, a query). `last` is that action's clock time, or `nil`
    /// before it has ever run this session.
    case onDemand(last: String?)
    /// An accumulated window/aggregate rather than a single fresh read, e.g.
    /// "history" or "24h".
    case window(label: String)
    /// A live stream currently paused by the operator, with `buffered`
    /// events queued to flow in once unpaused.
    case paused(buffered: Int)

    private var dotColor: Color {
        switch self {
        case .live: Theme.mint
        case .auto: Theme.iris
        case .snapshot: Theme.textSecondary
        case .onDemand: Theme.textSecondary
        case .window: Theme.amber
        case .paused: Theme.amber
        }
    }

    private var text: String {
        switch self {
        case .live: "LIVE"
        case .auto(let period): "AUTO \u{00B7} \(period)"
        case .snapshot(let at): "SNAPSHOT \u{00B7} \(at)"
        case .onDemand(let last): last.map { "ON-DEMAND \u{00B7} \($0)" } ?? "ON-DEMAND"
        case .window(let label): "WINDOW \u{00B7} \(label)"
        case .paused(let buffered): "PAUSED \u{00B7} \(buffered)"
        }
    }

    /// Only `.live` pulses - every other state is a static dot, matching the
    /// spec's own table (section 3: only the LIVE row carries "пульсуюча").
    private var pulses: Bool {
        if case .live = self { return true }
        return false
    }

    var body: some View {
        FreshBadgeDot(color: dotColor, text: text, pulses: pulses)
    }
}

/// The actual pill chrome for `FreshBadge`, split out only so the pulse
/// animation has somewhere to own its own `@State`/`@Environment` (an enum
/// can't carry stored properties).
@MainActor
private struct FreshBadgeDot: View {
    let color: Color
    let text: String
    let pulses: Bool

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pulseDim = false

    /// `.live` pulses unless Reduce Motion is on, in which case the dot
    /// stays a plain static mint dot (spec section 4: "Reduced motion:
    /// already in CSS, extend to the badge pulse - swap for a static dot").
    private var animates: Bool { pulses && !reduceMotion }

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 7, height: 7)
                .opacity(animates && pulseDim ? 0.35 : 1)
                .onAppear {
                    guard animates else { return }
                    withAnimation(.easeInOut(duration: 1.1).repeatForever(autoreverses: true)) {
                        pulseDim = true
                    }
                }
            Text(text)
                .font(Theme.mono(10, weight: .semibold))
                .tracking(0.8)
        }
        .foregroundStyle(color)
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(Capsule().fill(color.opacity(0.12)))
        .overlay(Capsule().strokeBorder(color.opacity(0.4), lineWidth: 1))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(text.replacingOccurrences(of: "\u{00B7}", with: "-"))
    }
}

// MARK: - DashSection

/// A titled gradient card with a header (title, an optional right-aligned
/// note, a freshness `badge`, and an optional `onRefresh` action) and
/// arbitrary content beneath.
@MainActor
struct DashSection<Content: View>: View {
    let title: String
    var right: String?
    /// The freshness-grammar pill for this card's data - see `FreshBadge`.
    /// `nil` renders no badge, for any section the freshness wave hasn't
    /// reached yet.
    var badge: FreshBadge?
    /// An explicit "pull now" action rendered as a small button right of the
    /// badge - for `.snapshot`/`.window` sections whose data does not
    /// refresh itself on a timer (Identity, Quality history; see
    /// `/Users/factory/Development/itrat-console/11-design-spec-chesna-svizhist.md`
    /// section 7 item 4).
    var onRefresh: (() -> Void)?
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(title)
                    .font(Theme.display(14.5, weight: .semibold))
                    .foregroundStyle(Theme.textPrimary)
                Spacer(minLength: 12)
                if let right {
                    Text(right)
                        .font(Theme.mono(11))
                        .foregroundStyle(Theme.textSecondary)
                }
                if let badge {
                    badge
                }
                if let onRefresh {
                    Button(action: onRefresh) {
                        HStack(spacing: 4) {
                            Image(systemName: "arrow.clockwise").font(.system(size: 9, weight: .bold))
                            Text("Refresh")
                        }
                        .font(Theme.mono(10.5, weight: .semibold))
                        .foregroundStyle(Theme.textSecondary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Refresh \(title)")
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 16)
            .padding(.bottom, 12)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .dashCard()
    }
}

// MARK: - HeroCard

/// The plane's headline: caption, a very large value, an optional right-aligned
/// secondary figure, a sparkline, a fuse bar, and a two-part note.
@MainActor
struct HeroCard: View {
    let cap: String
    let value: String
    var sub: Text?
    var series: [Double]?
    var fuseFraction: Double?
    var fuseTone: FuseTone = .iris
    var note: (left: Text, right: Text)?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(cap.uppercased())
                .font(Theme.mono(10.5, weight: .semibold))
                .tracking(1.6)
                .foregroundStyle(Theme.textTertiary)
            HStack(alignment: .lastTextBaseline, spacing: 14) {
                Text(value)
                    .font(Theme.display(52, weight: .heavy))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.5)
                Spacer(minLength: 8)
                if let sub {
                    sub.font(Theme.mono(12.5)).foregroundStyle(Theme.textSecondary)
                }
            }
            .padding(.top, 8)
            if let series {
                Sparkline(values: series).padding(.top, 14).padding(.bottom, 6)
            }
            if let fuseFraction {
                FuseBar(fraction: fuseFraction, tone: fuseTone)
            }
            if let note {
                HStack {
                    note.left
                    Spacer(minLength: 12)
                    note.right
                }
                .font(Theme.mono(11.5))
                .foregroundStyle(Theme.textSecondary)
                .padding(.top, 12)
            }
        }
        .padding(.horizontal, 24)
        .padding(.top, 22)
        .padding(.bottom, 18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .dashCard()
    }
}

// MARK: - HeroBand

/// The top band: a wide hero card beside a 2x2 KPI tile grid.
@MainActor
struct HeroBand<Hero: View, Tiles: View>: View {
    @ViewBuilder let hero: () -> Hero
    @ViewBuilder let tiles: () -> Tiles

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            hero().frame(maxWidth: .infinity)
            tiles().frame(width: 380)
        }
    }
}

// MARK: - DashMain

/// The dashboard body: a wide primary column beside a fixed rail.
@MainActor
struct DashMain<Primary: View, Rail: View>: View {
    @ViewBuilder let primary: () -> Primary
    @ViewBuilder let rail: () -> Rail

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            VStack(spacing: 16) { primary() }.frame(maxWidth: .infinity)
            VStack(spacing: 16) { rail() }.frame(width: 372)
        }
    }
}

// MARK: - Bars

struct DashBarItem: Identifiable {
    let id: String
    let label: String
    var sub: String?
    let fraction: Double
    var tone: FuseTone = .amber
    let value: String
    var onTap: (() -> Void)?
}

/// Ranked horizontal fuse-bars (spend by agent, findings by kind, ...).
@MainActor
struct DashBars: View {
    let items: [DashBarItem]
    var empty: String = "no data"

    var body: some View {
        if items.isEmpty {
            Text(empty)
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
        } else {
            VStack(spacing: 14) {
                ForEach(items) { it in
                    if let onTap = it.onTap {
                        Button(action: onTap) { barRow(it) }
                            .buttonStyle(.plain)
                    } else {
                        barRow(it)
                    }
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 6)
            .padding(.bottom, 16)
        }
    }

    private func barRow(_ it: DashBarItem) -> some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 1) {
                Text(it.label)
                    .font(.system(size: 12.5))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let sub = it.sub {
                    Text(sub.uppercased())
                        .font(Theme.mono(9.5, weight: .semibold))
                        .tracking(0.9)
                        .foregroundStyle(Theme.textTertiary)
                }
            }
            .frame(width: 168, alignment: .leading)
            FuseBar(fraction: it.fraction, tone: it.tone)
            Text(it.value)
                .font(Theme.mono(12.5))
                .monospacedDigit()
                .foregroundStyle(Theme.textPrimary)
                .frame(width: 80, alignment: .trailing)
        }
        .contentShape(Rectangle())
    }
}

// MARK: - Feed

struct DashFeedItem: Identifiable {
    let id: String
    let color: Color
    let title: String
    var sub: String?
    var value: String?
    var valueColor: Color?
    var onTap: (() -> Void)?
}

/// A vertical feed of dot + title/sub + right value rows (incidents, alerts).
@MainActor
struct DashFeed: View {
    let items: [DashFeedItem]
    var empty: String = "nothing here"

    var body: some View {
        if items.isEmpty {
            Text(empty)
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 28)
        } else {
            VStack(spacing: 0) {
                ForEach(Array(items.enumerated()), id: \.element.id) { i, it in
                    if i > 0 { Divider().overlay(Theme.hairline) }
                    if let onTap = it.onTap {
                        Button(action: onTap) { feedRow(it) }
                            .buttonStyle(.plain)
                    } else {
                        feedRow(it)
                    }
                }
            }
        }
    }

    private func feedRow(_ it: DashFeedItem) -> some View {
        HStack(spacing: 12) {
            Circle()
                .fill(it.color)
                .frame(width: 9, height: 9)
                .shadow(color: it.color.opacity(0.7), radius: 4)
            VStack(alignment: .leading, spacing: 2) {
                Text(it.title)
                    .font(.system(size: 12.5))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let sub = it.sub {
                    Text(sub)
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            Spacer(minLength: 8)
            if let value = it.value {
                Text(value)
                    .font(Theme.display(16, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(it.valueColor ?? Theme.textPrimary)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .contentShape(Rectangle())
    }
}

// MARK: - Composition

struct DashCompItem: Identifiable {
    let id: String
    let label: String
    let value: Double
    let total: Double
    let tone: FuseTone
    let valueText: String
}

/// Stacked share-of-total rows: label, amount + percent, and a fuse bar.
@MainActor
struct DashComposition: View {
    let items: [DashCompItem]

    var body: some View {
        VStack(spacing: 14) {
            ForEach(items) { it in
                let frac = it.total > 0 ? it.value / it.total : 0
                VStack(spacing: 7) {
                    HStack {
                        Text(it.label)
                            .font(.system(size: 12.5))
                            .foregroundStyle(Theme.textSecondary)
                        Spacer(minLength: 10)
                        Text("\(it.valueText) \u{00B7} \(Int((frac * 100).rounded()))%")
                            .font(Theme.mono(12.5))
                            .monospacedDigit()
                            .foregroundStyle(Theme.textPrimary)
                    }
                    FuseBar(fraction: frac, tone: it.tone)
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 6)
        .padding(.bottom, 18)
    }
}

// MARK: - Shared derivations

enum Dash {
    /// Thousands-separated dollars, no cents (hero headline).
    static func usd0(_ v: Double) -> String {
        let f = NumberFormatter()
        f.numberStyle = .decimal
        f.maximumFractionDigits = 0
        return "$" + (f.string(from: NSNumber(value: v.rounded())) ?? String(Int(v.rounded())))
    }

    /// Plain integer with thousands separators.
    static func int(_ v: Int) -> String {
        let f = NumberFormatter()
        f.numberStyle = .decimal
        return f.string(from: NSNumber(value: v)) ?? String(v)
    }

    static func agentShort(_ id: String) -> String {
        id.split(separator: "/").last.map(String.init) ?? id
    }

    static func agentTeam(_ id: String) -> String {
        let parts = id.split(separator: "/")
        return parts.count >= 2 ? String(parts[parts.count - 2]) : ""
    }

    static func sevRank(_ s: String) -> Int {
        ["info", "low", "medium", "high", "critical"].firstIndex(of: s.lowercased()) ?? 0
    }

    struct AgentSpend: Identifiable {
        let agent: String
        let name: String
        let team: String
        var spent: Double
        var calls: UInt64
        var id: String { agent }
    }

    static func spendByAgent(_ runs: [Run]) -> [AgentSpend] {
        var m: [String: AgentSpend] = [:]
        for r in runs {
            var e = m[r.agentId] ?? AgentSpend(agent: r.agentId, name: agentShort(r.agentId), team: agentTeam(r.agentId), spent: 0, calls: 0)
            e.spent += r.spentUsd
            e.calls += r.calls
            m[r.agentId] = e
        }
        return m.values.sorted { $0.spent > $1.spent }
    }

    private nonisolated(unsafe) static let isoFrac: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    private nonisolated(unsafe) static let isoPlain: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    /// Bucket per-run spend by `lastSeen` into a spend-over-window curve.
    static func spendSeries(_ runs: [Run], buckets: Int = 32) -> [Double] {
        let stamped: [(Double, Double)] = runs.compactMap { r in
            guard let d = isoFrac.date(from: r.lastSeen) ?? isoPlain.date(from: r.lastSeen) else { return nil }
            return (d.timeIntervalSince1970, r.spentUsd)
        }
        guard stamped.count >= 2 else { return [] }
        let times = stamped.map { $0.0 }
        let mn = times.min()!
        let mx = times.max()!
        let span = max(1, mx - mn)
        var out = [Double](repeating: 0, count: buckets)
        for (t, s) in stamped {
            let idx = min(buckets - 1, Int((t - mn) / span * Double(buckets)))
            out[idx] += s
        }
        return out
    }
}
