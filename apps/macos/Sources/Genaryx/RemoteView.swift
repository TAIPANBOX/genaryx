import GenaryxCoreFFI
import SwiftUI

/// The Remote (Distance) panel (docs/PHASE4.md W4, decision D11): Hetzner
/// read-only inventory, the remote-environment config form (the box's own
/// WireGuard peer + the SSH target), the tunnel's Connect/Disconnect control
/// with an honest status badge, the console's own WG public key shown for
/// copying, and SSH ops (check / read descriptor / tail). Fed entirely by
/// `RemoteModel`; like `CryptoView`/`DrillsView`, this panel has no bus-event
/// filter section (none of Hetzner/WireGuard/SSH emit a bus event of their
/// own) and every read/action here is explicit, never auto-run.
///
/// Every one of the four sections below fails independently and honestly -
/// there is no single "environment" gating the whole panel (see
/// `RemoteConnection`'s own doc): an operator can browse Hetzner inventory
/// with zero WireGuard tooling installed, or vice versa.
@MainActor
struct RemoteView: View {
    let model: RemoteModel

    /// Keeps the tunnel status badge's "handshake Ns ago" honestly fresh
    /// without a second `connectTunnel` attempt - mirrors
    /// `MemoryView`/`MenuBarBusView`'s own periodic `.task` refresh loops.
    private static let tunnelPollInterval: Duration = .seconds(5)

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                RemoteEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task(id: model.connection.isReady) {
            guard model.connection.isReady else { return }
            while !Task.isCancelled {
                await model.refreshTunnelStatus()
                try? await Task.sleep(for: Self.tunnelPollInterval)
            }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                privilegeNote

                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                section(title: "Hetzner Inventory") {
                    HetznerInventorySection(model: model)
                }
                section(title: "Remote Environment") {
                    RemoteEnvironmentSection(model: model)
                }
                section(title: "Tunnel") {
                    TunnelSection(model: model)
                }
                section(title: "SSH Ops") {
                    SshOpsSection(model: model)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// docs/PHASE4.md W4: "Privilege reality... Add a short in-panel note."
    /// `wireguard-go` needs root to create a tun device, so a LOCAL Connect
    /// attempt on this box is EXPECTED to fail - this note says so up front,
    /// before the operator ever presses Connect and sees `FAILED` for
    /// themselves.
    private var privilegeNote: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "info.circle")
                .font(.system(size: 11))
                .foregroundStyle(Theme.textTertiary)
            Text(
                "wireguard-go needs root privileges to create a tun device. Run locally and unprivileged, Connect is expected to fail honestly, that is the correct v1 outcome, not a bug. The live tunnel is proven on the Hetzner validation campaign, where the box has the privileges this laptop does not."
            )
            .font(Theme.mono(10.5))
            .foregroundStyle(Theme.textTertiary)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panelElevated)
        )
    }

    @ViewBuilder
    private func section<Content: View>(title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title.uppercased())
                .font(Theme.mono(11, weight: .semibold))
                .tracking(1.4)
                .foregroundStyle(Theme.textTertiary)
            content()
        }
    }
}

// MARK: - shared field style

/// One labeled text field, the same visual weight `DrillsView`/`EvidenceView`
/// already establish - a file-scope helper (rather than a method on any one
/// section struct) since every section below needs it.
@ViewBuilder
private func remoteField(_ placeholder: String, text: Binding<String>) -> some View {
    TextField(placeholder, text: text)
        .textFieldStyle(.plain)
        .font(Theme.mono(11.5))
        .foregroundStyle(Theme.textPrimary)
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panelElevated))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))
}

private func subheading(_ text: String) -> some View {
    Text(text.uppercased())
        .font(Theme.mono(9.5, weight: .semibold))
        .tracking(0.8)
        .foregroundStyle(Theme.textTertiary)
}

// MARK: - HetznerInventorySection

/// docs/PHASE4.md W4: "a read-scoped API token + optional label selector ->
/// list boxes: id/name/status/ipv4/type/cores/RAM/price-per-hour; NEVER
/// create/delete". The token field carries no autocomplete/history of its
/// own (a plain `TextField`, not a `SecureField` - PHASE4.md does not ask for
/// masking, and a read-scoped token is the same honesty class this console
/// already pre-fills from `HCLOUD_TOKEN` in the clear).
@MainActor
private struct HetznerInventorySection: View {
    let model: RemoteModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                remoteField("Hetzner API token", text: Binding(get: { model.hetznerToken }, set: { model.hetznerToken = $0 }))
                remoteField(
                    "label selector (optional)",
                    text: Binding(get: { model.hetznerLabelSelector }, set: { model.hetznerLabelSelector = $0 }))
                scanButton
            }
            Text(
                RemoteAsOfFormat.label(model.hetznerLoadedAt, prefix: "as of last scan", emptyText: "no scan yet")
            )
            .font(Theme.mono(10.5))
            .foregroundStyle(Theme.textTertiary)

            if model.hetznerServers.isEmpty {
                Text(model.isLoadingHetzner ? "scanning..." : "scan a token to see its Hetzner inventory.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            } else {
                table
            }
        }
    }

    private var scanButton: some View {
        Button {
            Task { await model.listHetzner() }
        } label: {
            HStack(spacing: 5) {
                if model.isLoadingHetzner {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "server.rack")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(model.isLoadingHetzner ? "Scanning..." : "Scan")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.amber)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.amber.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.amber.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isLoadingHetzner)
    }

    private var table: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(Theme.hairlineStrong)
            ForEach(Array(model.hetznerServers.enumerated()), id: \.element.id) { index, server in
                HetznerServerRow(server: server)
                if index < model.hetznerServers.count - 1 {
                    Divider().overlay(Theme.hairline)
                }
            }
        }
        .background(Theme.panel)
        .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .strokeBorder(Theme.hairline, lineWidth: 1)
        )
    }

    private var header: some View {
        HStack(spacing: 10) {
            columnLabel("NAME").frame(maxWidth: .infinity, alignment: .leading)
            columnLabel("STATUS").frame(width: 90, alignment: .leading)
            columnLabel("IPV4").frame(width: 120, alignment: .leading)
            columnLabel("TYPE").frame(width: 70, alignment: .leading)
            columnLabel("CORES/RAM").frame(width: 90, alignment: .trailing)
            columnLabel("PRICE/HR").frame(width: 90, alignment: .trailing)
            columnLabel("LOCATION").frame(width: 80, alignment: .trailing)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(Theme.panelElevated)
    }

    private func columnLabel(_ text: String) -> some View {
        Text(text)
            .font(Theme.mono(10, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(Theme.textTertiary)
    }

    private struct HetznerServerRow: View {
        let server: HetznerServerRecord

        var body: some View {
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(server.name)
                        .font(Theme.mono(11.5, weight: .medium))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    if !server.labels.isEmpty {
                        Text(server.labels.map { "\($0.key)=\($0.value)" }.joined(separator: ", "))
                            .font(Theme.mono(9.5))
                            .foregroundStyle(Theme.textTertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Text(server.status.uppercased())
                    .font(Theme.mono(10, weight: .semibold))
                    .foregroundStyle(HetznerStatusFormat.color(server.status))
                    .frame(width: 90, alignment: .leading)

                Text(server.ipv4 ?? "no IP")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(server.ipv4 == nil ? Theme.textTertiary : Theme.textSecondary)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .frame(width: 120, alignment: .leading)

                Text(server.serverType)
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: 70, alignment: .leading)

                Text("\(server.cores)c / \(String(format: "%.0f", server.memoryGb))GB")
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: 90, alignment: .trailing)

                Text(server.priceHourlyEur.map { String(format: "\u{20AC}%.4f", $0) } ?? "n/a")
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(server.priceHourlyEur == nil ? Theme.textTertiary : Theme.textSecondary)
                    .frame(width: 90, alignment: .trailing)

                Text(server.location)
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .frame(width: 80, alignment: .trailing)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        }
    }
}

// MARK: - RemoteEnvironmentSection

/// docs/PHASE4.md W4: "remote environment config (WG peer pubkey_hex +
/// endpoint + allowed_ips + tunnel local_ip/peer_ip; SSH host/port/user/
/// identity-file/pinned-host-key; the wireguard-go binary path)". Every field
/// here is a plain, honest, "operator can see/set it, never enforced"
/// pre-fill (`RemoteModel.loadDefaults`) - none of it is validated until the
/// operator actually presses Connect / an SSH op button.
@MainActor
private struct RemoteEnvironmentSection: View {
    let model: RemoteModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 6) {
                subheading("WireGuard peer (the client-hosted Cloud's box)")
                HStack(spacing: 8) {
                    remoteField(
                        "wireguard-go binary", text: Binding(get: { model.wireguardGoBin }, set: { model.wireguardGoBin = $0 }))
                    remoteField("interface", text: Binding(get: { model.interfaceName }, set: { model.interfaceName = $0 }))
                }
                remoteField(
                    "peer public key (hex)", text: Binding(get: { model.peerPublicKeyHex }, set: { model.peerPublicKeyHex = $0 })
                )
                HStack(spacing: 8) {
                    remoteField("endpoint (host:port)", text: Binding(get: { model.endpoint }, set: { model.endpoint = $0 }))
                    remoteField(
                        "keepalive seconds", text: Binding(get: { model.persistentKeepaliveText }, set: { model.persistentKeepaliveText = $0 })
                    )
                }
                remoteField(
                    "allowed IPs (comma-separated CIDRs)",
                    text: Binding(get: { model.allowedIpsText }, set: { model.allowedIpsText = $0 }))
                HStack(spacing: 8) {
                    remoteField("tunnel local IP", text: Binding(get: { model.localIp }, set: { model.localIp = $0 }))
                    remoteField("tunnel peer IP", text: Binding(get: { model.peerIp }, set: { model.peerIp = $0 }))
                }
            }

            Divider().overlay(Theme.hairline)

            VStack(alignment: .leading, spacing: 6) {
                subheading("SSH target")
                HStack(spacing: 8) {
                    remoteField("host", text: Binding(get: { model.sshHost }, set: { model.sshHost = $0 }))
                    remoteField("port", text: Binding(get: { model.sshPortText }, set: { model.sshPortText = $0 }))
                    remoteField("user", text: Binding(get: { model.sshUser }, set: { model.sshUser = $0 }))
                }
                remoteField(
                    "identity file (path)", text: Binding(get: { model.sshIdentityFile }, set: { model.sshIdentityFile = $0 }))
                remoteField(
                    "pinned host key (\"ssh-ed25519 AAAA...\")",
                    text: Binding(get: { model.sshPinnedHostKey }, set: { model.sshPinnedHostKey = $0 }))
            }
        }
    }
}

// MARK: - TunnelSection

/// docs/PHASE4.md W4: "a Connect/Disconnect control with an honest status
/// badge... the console WG public key shown for copying".
@MainActor
private struct TunnelSection: View {
    let model: RemoteModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            statusBadge
            controls
            if let publicKeyB64 = model.consolePublicKeyB64, let publicKeyHex = model.consolePublicKeyHex {
                consoleKeyCard(b64: publicKeyB64, hex: publicKeyHex)
            } else {
                Text("generate the console keypair to see its public key here.")
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textTertiary)
            }
        }
    }

    private var statusBadge: some View {
        let label = model.isConnectingTunnel ? "CONNECTING..." : TunnelStatusFormat.label(model.tunnelStatus)
        let color = model.isConnectingTunnel ? Theme.amber : TunnelStatusFormat.color(model.tunnelStatus)
        return HStack(spacing: 6) {
            if model.isConnectingTunnel {
                ProgressView().controlSize(.small)
            } else {
                Circle().fill(color).frame(width: 7, height: 7)
            }
            Text(label)
                .font(Theme.mono(12, weight: .bold))
                .foregroundStyle(color)
                .lineLimit(2)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Capsule().fill(color.opacity(0.12)))
        .overlay(Capsule().strokeBorder(color.opacity(0.4), lineWidth: 1))
    }

    private var controls: some View {
        HStack(spacing: 10) {
            generateKeypairButton
            connectButton
            if case .connected = model.tunnelStatus {
                disconnectButton
            }
            Spacer(minLength: 0)
        }
    }

    private var generateKeypairButton: some View {
        Button {
            Task { await model.generateKeypair() }
        } label: {
            HStack(spacing: 5) {
                if model.isGeneratingKeypair {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "key.fill")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(model.consolePublicKeyB64 == nil ? "Generate console keypair" : "Regenerate console keypair")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.iris)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.iris.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.iris.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isGeneratingKeypair)
    }

    private var connectButton: some View {
        Button {
            Task { await model.connectTunnel() }
        } label: {
            HStack(spacing: 5) {
                if model.isConnectingTunnel {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "bolt.horizontal.circle.fill")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(model.isConnectingTunnel ? "Connecting..." : "Connect")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.mint)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.mint.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.mint.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isConnectingTunnel)
    }

    private var disconnectButton: some View {
        Button {
            Task { await model.disconnectTunnel() }
        } label: {
            Text("Disconnect")
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.coral)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Capsule().fill(Theme.coral.opacity(0.14)))
                .overlay(Capsule().strokeBorder(Theme.coral.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
    }

    /// docs/PHASE4.md W4: "the console WG public key shown for copying" - a
    /// plain, text-selectable field (mirrors `ArtifactRow`'s own sha256
    /// display), never a bespoke clipboard button this codebase does not
    /// otherwise use.
    private func consoleKeyCard(b64: String, hex: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            subheading("console public key (hand this to the box admin)")
            Text(b64)
                .font(Theme.mono(11.5, weight: .medium))
                .foregroundStyle(Theme.textPrimary)
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.middle)
            Text(hex)
                .font(Theme.mono(10))
                .foregroundStyle(Theme.textTertiary)
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panelElevated)
        )
    }
}

// MARK: - SshOpsSection

/// docs/PHASE4.md W4: "SSH ops (check / read descriptor / tail) - all
/// host-key-pinned by the connector". Every op reuses the SAME SSH target
/// fields from `RemoteEnvironmentSection` above (`RemoteModel` holds one
/// target, not three).
@MainActor
private struct SshOpsSection: View {
    let model: RemoteModel

    @State private var autoTail = false
    private static let autoTailInterval: Duration = .seconds(3)

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            checkRow
            Divider().overlay(Theme.hairline)
            readDescriptorRow
            Divider().overlay(Theme.hairline)
            tailRow
        }
    }

    // MARK: check

    private var checkRow: some View {
        VStack(alignment: .leading, spacing: 6) {
            subheading("check reachability")
            HStack(spacing: 10) {
                opButton(label: "Check", icon: "checkmark.shield") {
                    await model.sshCheck()
                }
                if let result = model.sshCheckResult {
                    HStack(spacing: 5) {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundStyle(Theme.mint)
                            .font(.system(size: 11))
                        Text(result)
                            .font(Theme.mono(11))
                            .foregroundStyle(Theme.textSecondary)
                    }
                }
                Spacer(minLength: 0)
            }
        }
    }

    // MARK: read descriptor

    private var readDescriptorRow: some View {
        VStack(alignment: .leading, spacing: 6) {
            subheading("read a remote file")
            HStack(spacing: 8) {
                remoteField(
                    "remote path (e.g. ~/.taipan/environments/<name>.json)",
                    text: Binding(get: { model.sshDescriptorPath }, set: { model.sshDescriptorPath = $0 }))
                opButton(label: "Read", icon: "doc.text") {
                    await model.sshReadDescriptor()
                }
            }
            Text(
                RemoteAsOfFormat.label(model.sshDescriptorLoadedAt, prefix: "as of last read", emptyText: "no read yet")
            )
            .font(Theme.mono(10.5))
            .foregroundStyle(Theme.textTertiary)
            if let bytes = model.sshDescriptorBytes {
                descriptorPreview(bytes)
            }
        }
    }

    private func descriptorPreview(_ bytes: Data) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("\(EvidenceSizeFormat.label(UInt64(bytes.count))) read")
                .font(Theme.mono(10))
                .foregroundStyle(Theme.textTertiary)
            ScrollView {
                Text(String(decoding: bytes, as: UTF8.self))
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textSecondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
            }
            .frame(maxHeight: 180)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
        }
    }

    // MARK: tail

    private var tailRow: some View {
        VStack(alignment: .leading, spacing: 6) {
            subheading("tail a remote file")
            HStack(spacing: 8) {
                remoteField(
                    "remote path (e.g. a taipan bus ndjson file)",
                    text: Binding(get: { model.sshTailPath }, set: { model.sshTailPath = $0 }))
                opButton(label: "Poll", icon: "arrow.clockwise") {
                    await model.sshTailPoll()
                }
                autoTailToggle
                clearButton
            }
            tailOutput
        }
        .task(id: autoTail) {
            guard autoTail else { return }
            while !Task.isCancelled && autoTail {
                await model.sshTailPoll()
                try? await Task.sleep(for: Self.autoTailInterval)
            }
        }
    }

    private var autoTailToggle: some View {
        Toggle("auto", isOn: $autoTail)
            .toggleStyle(.checkbox)
            .font(Theme.mono(11, weight: .medium))
            .foregroundStyle(Theme.textSecondary)
    }

    private var clearButton: some View {
        Button {
            model.resetTail()
        } label: {
            Text("Clear")
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.textSecondary)
        }
        .buttonStyle(.plain)
    }

    private var tailOutput: some View {
        ScrollView {
            Text(model.sshTailOutput.isEmpty ? "nothing polled yet." : model.sshTailOutput)
                .font(Theme.mono(10.5))
                .foregroundStyle(model.sshTailOutput.isEmpty ? Theme.textTertiary : Theme.textSecondary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(10)
        }
        .frame(maxHeight: 220)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panelElevated)
        )
    }

    // MARK: shared op button

    private func opButton(label: String, icon: String, action: @escaping () async -> Void) -> some View {
        Button {
            Task { await action() }
        } label: {
            HStack(spacing: 5) {
                if model.isRunningSshOp {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: icon)
                        .font(.system(size: 10, weight: .bold))
                }
                Text(label)
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.teal)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.teal.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.teal.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isRunningSshOp)
    }
}
