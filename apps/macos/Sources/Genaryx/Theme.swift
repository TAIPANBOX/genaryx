import AppKit
import SwiftUI

/// Color and type tokens derived from the it-rat2 design language
/// (`/Users/factory/Development/it-rat2/assets/site.css`, a read-only
/// reference outside this repo). Every view pulls its colors and fonts from
/// here, so the palette lives in exactly one place.
///
/// it-rat2 itself ships dark-only. `Dark` below is a direct translation of
/// its CSS custom properties; `Light` is a derived analog (the same hue
/// identities and surface/text hierarchy, just inverted lightness) so the
/// native macOS shell still honors the system's light-mode preference,
/// consistent with native macOS patterns taking priority over pixel parity
/// with the web shell.
enum Theme {

    // MARK: - Dark palette (matches site.css `:root` exactly)

    private enum Dark {
        static let bg = Color(hex: 0x0A0E13)
        static let panel = Color(hex: 0x10161F)
        static let panel2 = Color(hex: 0x141C27)
        static let line = Color.white.opacity(0.07)
        static let line2 = Color.white.opacity(0.13)
        static let fg = Color(hex: 0xE9EFF6)
        static let dim = Color(hex: 0x8A97A6)
        static let faint = Color(hex: 0x5A6675)
    }

    // MARK: - Light palette (derived analog; it-rat2 has no light variant)

    private enum Light {
        static let bg = Color(hex: 0xF4F6F9)
        static let panel = Color(hex: 0xFFFFFF)
        static let panel2 = Color(hex: 0xF7F9FB)
        static let line = Color.black.opacity(0.08)
        static let line2 = Color.black.opacity(0.14)
        static let fg = Color(hex: 0x12181F)
        static let dim = Color(hex: 0x55626F)
        static let faint = Color(hex: 0x808C97)
    }

    // MARK: - Semantic surface and text tokens (adapt to the system appearance)

    static let background = Color.dynamic(light: Light.bg, dark: Dark.bg)
    static let panel = Color.dynamic(light: Light.panel, dark: Dark.panel)
    static let panelElevated = Color.dynamic(light: Light.panel2, dark: Dark.panel2)
    static let hairline = Color.dynamic(light: Light.line, dark: Dark.line)
    static let hairlineStrong = Color.dynamic(light: Light.line2, dark: Dark.line2)
    static let textPrimary = Color.dynamic(light: Light.fg, dark: Dark.fg)
    static let textSecondary = Color.dynamic(light: Light.dim, dark: Dark.dim)
    static let textTertiary = Color.dynamic(light: Light.faint, dark: Dark.faint)

    // MARK: - Brand hues (`--mint`, `--amber`, etc. in site.css). Identical in
    // both themes; used only as accents on dots and badge tints/borders,
    // never as body text color, so contrast stays fine in either appearance.

    static let mint = Color(hex: 0x34D399)
    static let amber = Color(hex: 0xF4B23E)
    static let ember = Color(hex: 0xFF574B)
    static let iris = Color(hex: 0x6C7BFF)
    static let teal = Color(hex: 0x2DD4BF)
    static let violet = Color(hex: 0xB48CFF)
    static let rose = Color(hex: 0xFF7AA2)
    static let coral = Color(hex: 0xFF8A5B)
    static let steel = Color(hex: 0x93A8C4)

    // MARK: - Corner radii (`--rad`, `--rad-s`, and the pill radius from `.pill`)

    enum Radius {
        static let card: CGFloat = 18
        static let row: CGFloat = 12
        static let pill: CGFloat = 999
    }

    // MARK: - Fonts (`--font-d` / `--font-m`). SF Pro Display/Text and SF Mono
    // are the system default and system monospaced designs on macOS, so no
    // bundled font files are needed to match them.

    static func display(_ size: CGFloat, weight: Font.Weight = .bold) -> Font {
        .system(size: size, weight: weight, design: .default)
    }

    static func mono(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }

    // MARK: - Per-source accent, matching each service's `--accent` override
    // in it-rat2's `services/*.html`. idryx/pocket/sphere/platform are not
    // emitting bus sources (per `crates/core/src/demo.rs`), so they are
    // omitted here; a future non-bus source falls back to `steel`.

    static func sourceColor(_ source: String) -> Color {
        switch source.lowercased() {
        case "tokenfuse": return amber
        case "wardryx": return teal
        case "engram": return iris
        case "verdryx": return rose
        case "mockryx": return coral
        case "qryx": return violet
        default: return steel
        }
    }

    // MARK: - Severity ladder. Reuses five hues not already claimed by a
    // source above, ramping calm to hot: steel (info) -> mint (low) -> amber
    // (medium) -> coral (high) -> ember (critical). A `nil` severity (the
    // envelope field is optional) is kept visually distinct from an explicit
    // "info" rather than defaulting to it.

    static func severityColor(_ severity: String?) -> Color {
        switch severity?.lowercased() {
        case "critical": return ember
        case "high": return coral
        case "medium": return amber
        case "low": return mint
        case "info": return steel
        default: return Color.secondary
        }
    }

    static func severityLabel(_ severity: String?) -> String {
        severity?.lowercased() ?? "n/a"
    }
}

extension Color {
    /// Builds a `Color` from a packed `0xRRGGBB` literal, the shorthand every
    /// token above uses to stay a direct, greppable match for its it-rat2 hex
    /// source.
    init(hex: UInt32) {
        let r = Double((hex >> 16) & 0xFF) / 255
        let g = Double((hex >> 8) & 0xFF) / 255
        let b = Double(hex & 0xFF) / 255
        self.init(red: r, green: g, blue: b)
    }

    /// A `Color` that resolves to `dark` under the dark appearance and
    /// `light` otherwise, so every call site above reads as one constant
    /// rather than an `@Environment(\.colorScheme)` check per view.
    static func dynamic(light: Color, dark: Color) -> Color {
        Color(
            nsColor: NSColor(
                name: nil,
                dynamicProvider: { appearance in
                    let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
                    return NSColor(isDark ? dark : light)
                }
            )
        )
    }
}
