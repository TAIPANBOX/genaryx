import GenaryxCoreFFI
import SwiftUI

/// The Pocket panel (docs/PHASE5.md W2, itrat-console/13 D12.2a): "Connect
/// TokenFuse Pocket" mints a pairing code at the Cloud, arms the relay's
/// pairing window, and renders the QR the phone scans - a later wave (W3)
/// builds the scanner itself. Three states: idle (Connect button),
/// showing-QR (an armed window, waiting for the phone), and paired (device
/// details + Disconnect). Fed entirely by `PocketModel`, mirroring
/// `RemoteView`'s "no bus-event filter section, every read/action here is
/// explicit" shape.
@MainActor
struct PocketView: View {
    let model: PocketModel

    /// While a QR is armed, poll status fast enough that the panel flips to
    /// Paired within a couple seconds of a real pairing - mirrors
    /// `apps/desktop/src/lib/usePocketStatus.ts`'s identical `WATCH_POLL_MS`.
    private static let watchPollInterval: Duration = .seconds(2)
    /// Redraws the "expires in Ns" countdown - display-only, never drives a
    /// network call.
    private static let countdownTick: Duration = .seconds(1)

    @State private var now = Date()

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                PocketEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task(id: watchKey) {
            guard model.connection.isReady, model.armedQr != nil else { return }
            while !Task.isCancelled {
                try? await Task.sleep(for: Self.watchPollInterval)
                if Task.isCancelled { return }
                await model.refreshStatus()
            }
        }
        .task(id: model.armedQr?.qrContent) {
            guard model.armedQr != nil else { return }
            while !Task.isCancelled {
                try? await Task.sleep(for: Self.countdownTick)
                now = Date()
            }
        }
    }

    /// Re-runs the watch-poll `.task` exactly when entering/leaving the
    /// "an armed QR exists" state (not on every status refresh, which would
    /// otherwise restart the loop's own sleep on every tick).
    private var watchKey: String {
        "\(model.connection.isReady)-\(model.armedQr != nil)"
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                statusBadge

                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                explainer

                switch model.status {
                case .relayUnreachable(let message):
                    relayUnreachableNote(message)
                case .idle:
                    if let armedQr = model.armedQr {
                        qrCard(armedQr)
                    } else {
                        connectCard
                    }
                case .paired(let deviceId, let name, let platform, let pairedAtUnix, let lastSeenUnix):
                    pairedCard(
                        deviceId: deviceId, name: name, platform: platform, pairedAtUnix: pairedAtUnix,
                        lastSeenUnix: lastSeenUnix)
                case .none:
                    EmptyView()
                }
            }
            .padding(20)
            .frame(maxWidth: 520, alignment: .leading)
        }
    }

    private var statusBadge: some View {
        let color = PocketStatusFormat.dotColor(model.status)
        return HStack(spacing: 6) {
            Circle().fill(color).frame(width: 7, height: 7)
            Text(PocketStatusFormat.label(model.status).uppercased())
                .font(Theme.mono(12, weight: .bold))
                .foregroundStyle(color)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Capsule().fill(color.opacity(0.12)))
        .overlay(Capsule().strokeBorder(color.opacity(0.4), lineWidth: 1))
    }

    private var explainer: some View {
        Text(
            "Pair your phone (TokenFuse Pocket) to this box's relay so you can see the exception queue and slide-to-kill a runaway from anywhere. A QR carries the relay's pinned TLS identity plus a one-time code, scanned once, no manual entry."
        )
        .font(Theme.mono(10.5))
        .foregroundStyle(Theme.textTertiary)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func relayUnreachableNote(_ message: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 11))
                .foregroundStyle(Theme.coral)
            Text("relay admin API unreachable - \(message)")
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).fill(Theme.panelElevated))
    }

    // MARK: - idle: Connect button

    private var connectCard: some View {
        let cloudReady: Bool = {
            if case .idle(let ready) = model.status { return ready }
            return false
        }()
        return VStack(alignment: .leading, spacing: 8) {
            Button {
                Task { await model.connectPocket() }
            } label: {
                HStack(spacing: 5) {
                    if model.isConnecting {
                        ProgressView().controlSize(.small)
                    } else {
                        Image(systemName: "qrcode")
                            .font(.system(size: 10, weight: .bold))
                    }
                    Text(model.isConnecting ? "Connecting..." : "Connect TokenFuse Pocket")
                }
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.mint)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Capsule().fill(Theme.mint.opacity(0.14)))
                .overlay(Capsule().strokeBorder(Theme.mint.opacity(0.4), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .disabled(model.isConnecting || !cloudReady)

            if !cloudReady {
                Text("no TokenFuse Cloud environment found (see Overview/Money) - cannot mint a pairing code yet.")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
            }
        }
    }

    // MARK: - showing-QR

    private func qrCard(_ qr: PocketQrRecord) -> some View {
        let remaining = max(0, qr.expiresUnix - Int64(now.timeIntervalSince1970))
        return VStack(alignment: .leading, spacing: 10) {
            if remaining > 0 {
                VStack(spacing: 10) {
                    QrCodeView(content: qr.qrContent, size: 220)
                    Text("expires in \(remaining)s - scan with TokenFuse Pocket")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                    cancelButton
                }
                .frame(maxWidth: .infinity)
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    Text("the pairing window expired unredeemed.")
                        .font(Theme.mono(11, weight: .semibold))
                        .foregroundStyle(Theme.amber)
                    Button {
                        Task { await model.cancelArmedWindow() }
                    } label: {
                        Text("Mint a new code")
                            .font(Theme.mono(11, weight: .semibold))
                            .foregroundStyle(Theme.iris)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(Capsule().fill(Theme.iris.opacity(0.14)))
                            .overlay(Capsule().strokeBorder(Theme.iris.opacity(0.4), lineWidth: 1))
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    private var cancelButton: some View {
        Button {
            Task { await model.cancelArmedWindow() }
        } label: {
            Text("Cancel")
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.textSecondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Capsule().fill(Theme.panelElevated))
                .overlay(Capsule().strokeBorder(Theme.hairlineStrong, lineWidth: 1))
        }
        .buttonStyle(.plain)
    }

    // MARK: - paired

    private func pairedCard(
        deviceId: String, name: String, platform: String, pairedAtUnix: Int64, lastSeenUnix: Int64
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text("\(name.isEmpty ? "(unnamed device)" : name) · \(platform.isEmpty ? "unknown platform" : platform)")
                    .font(Theme.mono(12, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                Text("device_id: \(deviceId)")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .textSelection(.enabled)
                Text("paired \(PocketTimeFormat.label(unixSeconds: pairedAtUnix))")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                Text("last seen \(PocketTimeFormat.label(unixSeconds: lastSeenUnix))")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
            }
            .padding(10)
            .background(RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).fill(Theme.panelElevated))

            Button {
                Task { await model.disconnectPocket() }
            } label: {
                HStack(spacing: 5) {
                    if model.isDisconnecting {
                        ProgressView().controlSize(.small)
                    }
                    Text(model.isDisconnecting ? "Disconnecting..." : "Disconnect")
                }
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.coral)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Capsule().fill(Theme.coral.opacity(0.14)))
                .overlay(Capsule().strokeBorder(Theme.coral.opacity(0.4), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .disabled(model.isDisconnecting)
        }
    }
}

/// Shown while `PocketHandle()` is still constructing, or failed to
/// construct - mirrors `RemoteEmptyStateView`'s own shape
/// (`RemoteComponents.swift`).
@MainActor
private struct PocketEmptyStateView: View {
    let connection: PocketConnection

    var body: some View {
        VStack(spacing: 10) {
            switch connection {
            case .connecting:
                ProgressView()
                Text("resolving the Pocket panel...")
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textTertiary)
            case .failed(let reason):
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 18))
                    .foregroundStyle(Theme.coral)
                Text("Could not start the Pocket panel")
                    .font(Theme.mono(12, weight: .semibold))
                    .foregroundStyle(Theme.textSecondary)
                Text(reason)
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 420)
            case .ready:
                EmptyView()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
