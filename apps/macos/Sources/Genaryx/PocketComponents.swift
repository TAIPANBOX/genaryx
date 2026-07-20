import CoreImage
import CoreImage.CIFilterBuiltins
import GenaryxCoreFFI
import SwiftUI

/// Renders `content` as a scannable QR code using CoreImage's built-in
/// `CIQRCodeGenerator` filter (docs/PHASE5.md W2: "Use a QR generator
/// (CoreImage CIQRCodeGenerator is in the SDK, no dependency)") - no
/// third-party QR library, mirroring the Tauri shell's own "dependency-free"
/// requirement met there with a vendored encoder instead
/// (`apps/desktop/src/lib/vendor/qrcodegen.ts`).
///
/// `CIQRCodeGenerator`'s native output is tiny (about one point per module -
/// a ~45-module code renders as a ~45x45pt `CIImage`), so this scales the
/// `CIImage` itself with a `CGAffineTransform` BEFORE rasterizing, then
/// disables SwiftUI's own resize interpolation on top (`.interpolation(.none)`):
/// smooth interpolation would blur a QR code's hard module edges into
/// something a camera can misread, exactly the failure mode the Tauri
/// shell's own `QrCode.tsx` avoids by rendering one hard-edged SVG `<path>`
/// per module rather than a blurred raster.
///
/// Colors are hardcoded white background / black modules, NEVER
/// `Theme`'s dynamic light/dark tokens - see `QrCode.tsx`'s identical
/// reasoning: a QR code must keep strong light/dark contrast to scan at
/// all, independent of the system appearance.
struct QrCodeView: View {
    let content: String
    var size: CGFloat = 220

    var body: some View {
        Group {
            if let nsImage = Self.render(content) {
                Image(nsImage: nsImage)
                    .resizable()
                    .interpolation(.none)
                    .frame(width: size, height: size)
            } else {
                Text("could not render QR")
                    .font(Theme.mono(10))
                    .foregroundStyle(.black)
                    .frame(width: size, height: size)
            }
        }
        .background(Color.white)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    /// `nil` only if CoreImage's own filter fails on `content` (empty
    /// input, or - per Apple's documented ~2953-byte QR ceiling - a message
    /// too long to encode at any version); the Pocket panel's QR content is
    /// a bounded ~150-character URL, far under that limit, so this is a
    /// defensive fallback, not an expected path.
    private static func render(_ content: String) -> NSImage? {
        guard let data = content.data(using: .utf8) else { return nil }
        let filter = CIFilter.qrCodeGenerator()
        filter.message = data
        // "M" (medium, ~15% tolerance) - the same error-correction level
        // the Tauri shell's `renderQr` picks, for parity between the two
        // shells' QR robustness.
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }

        let scale: CGFloat = 10
        let scaled = output.transformed(by: CGAffineTransform(scaleX: scale, y: scale))

        let context = CIContext()
        guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else { return nil }
        return NSImage(cgImage: cgImage, size: NSSize(width: scaled.extent.width, height: scaled.extent.height))
    }
}

// MARK: - status formatting

/// Dot color + label for `PocketStatusRecord` - mirrors
/// `TunnelStatusFormat`'s own shape (`RemoteComponents.swift`).
enum PocketStatusFormat {
    static func dotColor(_ status: PocketStatusRecord?) -> Color {
        switch status {
        case .paired:
            return Theme.mint
        case .relayUnreachable:
            return Theme.coral
        case .idle, .none:
            return Theme.textTertiary
        }
    }

    /// `.paired` now carries BOTH slots independently (`phone`/`watch`, each
    /// `nil` for an empty slot) - mirrors
    /// `apps/desktop/src/components/PocketView.tsx`'s `statusLabel` exactly:
    /// list whichever of the two are actually paired, since at least one
    /// always is in this case (that is what makes it `.paired` rather than
    /// `.idle`).
    static func label(_ status: PocketStatusRecord?) -> String {
        switch status {
        case .idle(let cloudReady, _, _):
            return cloudReady ? "no devices paired" : "no devices paired · Cloud not resolvable"
        case .paired(let phone, let watch, _, _):
            var parts: [String] = []
            if let phone { parts.append("phone: \(phone.name.isEmpty ? phone.deviceId : phone.name)") }
            if let watch { parts.append("watch: \(watch.name.isEmpty ? watch.deviceId : watch.name)") }
            return "paired · \(parts.joined(separator: ", "))"
        case .relayUnreachable:
            return "relay unreachable"
        case .none:
            return "resolving..."
        }
    }
}

/// A UNIX-seconds field (`pairedAtUnix`/`lastSeenUnix`) -> the same
/// "Jul 16 14:32:05" clock `MoneyFormat.timestamp` renders for an ISO
/// string, via the same `Date -> ISO8601 -> MoneyFormat.timestamp` bridge
/// `RemoteAsOfFormat.label` already uses for a `Date?` (`RemoteComponents.swift`).
enum PocketTimeFormat {
    static func label(unixSeconds: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(unixSeconds))
        let iso = ISO8601DateFormatter().string(from: date)
        return MoneyFormat.timestamp(iso)
    }
}
