import GenaryxCoreFFI
import SwiftUI

/// The Pocket panel (docs/PHASE5.md W2, itrat-console/13 D12.2a): "Connect
/// TokenFuse Pocket" mints a pairing code for the phone and one for the
/// watch at the Cloud, arms both of the relay's pairing windows, and renders
/// the QR (both codes) the phone scans - a later wave (W3) builds the
/// scanner itself. Three states: idle (Connect button), showing-QR (both
/// windows armed, waiting for the phone), and paired (each slot's device
/// details, or "not paired", + Disconnect). Fed entirely by `PocketModel`,
/// mirroring `RemoteView`'s "no bus-event filter section, every read/action
/// here is explicit" shape.
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
                case .idle(_, let phoneWindow, let watchWindow):
                    if let armedQr = model.armedQr {
                        qrCard(armedQr, phoneWindow: phoneWindow, watchWindow: watchWindow)
                    } else {
                        connectCard
                    }
                case .paired(let phone, let watch, let phoneWindow, let watchWindow):
                    pairedCard(phone: phone, watch: watch, phoneWindow: phoneWindow, watchWindow: watchWindow)
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
            "Pair your phone (TokenFuse Pocket) and its paired Watch to this box's relay so you can see the exception queue and slide-to-kill a runaway from anywhere. One QR carries the relay's pinned TLS identity plus a one-time code for each device, scanned once on the phone (which hands the Watch its own code), no manual entry."
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
            if case .idle(let ready, _, _) = model.status { return ready }
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

    private func qrCard(
        _ qr: PocketQrRecord, phoneWindow: PocketWindowRecord?, watchWindow: PocketWindowRecord?
    ) -> some View {
        let remaining = max(0, qr.expiresUnix - Int64(now.timeIntervalSince1970))
        return VStack(alignment: .leading, spacing: 10) {
            if remaining > 0 {
                VStack(spacing: 10) {
                    QrCodeView(content: qr.qrContent, size: 220)
                    Text("expires in \(remaining)s - scan with TokenFuse Pocket")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                    PairingProbeNote(label: "phone", pairingWindow: phoneWindow)
                    PairingProbeNote(label: "watch", pairingWindow: watchWindow)
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

    /// Both slots, independently: `Connect` always arms the phone's and the
    /// watch's pairing windows together (see the type doc), so a partial
    /// state here (one slot `nil`, the other set) means that device was
    /// disconnected on its own, never that it was simply not offered a code
    /// yet - `deviceRow` renders the honest "not paired" placeholder for it
    /// rather than omitting the row. There is no per-slot re-Connect from
    /// this state: Disconnect always frees BOTH slots at once, so resetting
    /// either one to pair again also resets the other.
    private func pairedCard(
        phone: PocketDeviceRecord?, watch: PocketDeviceRecord?, phoneWindow: PocketWindowRecord?,
        watchWindow: PocketWindowRecord?
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 8) {
                deviceRow(label: "Phone", device: phone, pairingWindow: phoneWindow)
                deviceRow(label: "Watch", device: watch, pairingWindow: watchWindow)
            }

            Button {
                Task { await model.disconnectPocket() }
            } label: {
                HStack(spacing: 5) {
                    if model.isDisconnecting {
                        ProgressView().controlSize(.small)
                    }
                    Text(model.isDisconnecting ? "Disconnecting..." : "Disconnect all")
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

    /// One device slot within the paired card - `device == nil` renders the
    /// slot's honest "not paired" placeholder rather than being omitted, so
    /// the operator always sees both the phone and the watch slots at a
    /// glance (mirrors `PocketView.tsx`'s `PocketDeviceRow`). `pairingWindow`
    /// is normally `nil` once `device` is set (a successful redemption
    /// closes that slot's window at the relay) - it matters for the `device
    /// == nil` case: the watch's window commonly outlives the phone's own
    /// pairing while it waits on a WatchConnectivity handoff, so its probe
    /// count needs to stay visible even after the phone row above already
    /// shows paired.
    private func deviceRow(
        label: String, device: PocketDeviceRecord?, pairingWindow: PocketWindowRecord?
    ) -> some View {
        Group {
            if let device {
                VStack(alignment: .leading, spacing: 3) {
                    Text(
                        "\(label): \(device.name.isEmpty ? "(unnamed device)" : device.name) · \(device.platform.isEmpty ? "unknown platform" : device.platform)"
                    )
                    .font(Theme.mono(12, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                    Text("device_id: \(device.deviceId)")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                        .textSelection(.enabled)
                    Text("paired \(PocketTimeFormat.label(unixSeconds: device.pairedAtUnix))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                    Text("last seen \(PocketTimeFormat.label(unixSeconds: device.lastSeenUnix))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).fill(Theme.panelElevated))
            } else {
                VStack(alignment: .leading, spacing: 3) {
                    Text("\(label): not paired")
                        .font(Theme.mono(11, weight: .medium))
                        .foregroundStyle(Theme.textTertiary)
                    PairingProbeNote(label: label, pairingWindow: pairingWindow)
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous).fill(Theme.panelElevated))
            }
        }
    }
}

/// A small warning line for one currently armed window's probing count -
/// renders nothing at all while `failedAttempts` is 0 (the normal, quiet
/// steady state). PURELY OBSERVATIONAL: the relay never closes a window
/// over this (the pairing route is pre-auth, so it can't without letting an
/// unauthenticated caller deny pairing at will), so the copy deliberately
/// never implies blocking, lockout, or that the window will close itself -
/// it is only ever "here is what happened, use Disconnect if you want to
/// act on it" (mirrors `PocketView.tsx`'s `PairingProbeNote`).
@MainActor
private struct PairingProbeNote: View {
    let label: String
    let pairingWindow: PocketWindowRecord?

    var body: some View {
        if let pairingWindow, pairingWindow.failedAttempts > 0 {
            let n = pairingWindow.failedAttempts
            Text("\(label): \(n) invalid code\(n == 1 ? "" : "s") presented since arming")
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.amber)
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
