import Foundation
import GenaryxCoreFFI
import Observation

/// Whole-panel connection state for the Remote (Distance) surface - honest,
/// distinct states the view renders directly. UNLIKE every other Phase-4
/// panel, this is not a "did we discover an environment" state: `RemoteHandle`
/// resolves no environment at all (`crates/ffi/src/remote/mod.rs`'s own
/// module doc, "no `taipan up`-style environment to discover") - it only
/// starts a small local async runtime, so `.failed` here can only mean a
/// genuine local resource problem, not a missing Hetzner token / WireGuard
/// binary / SSH target (those are all per-field, form-level concerns the
/// panel content itself renders honestly, never a whole-panel empty state).
enum RemoteConnection: Equatable {
    case connecting
    case failed(reason: String)
    case ready

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// The WireGuard tunnel's status for the Swift side - a 1:1 mirror of
/// `WgStatusRecord`, kept as its own type only so `RemoteModel` has a value
/// to hold before the first real read (`WgStatusRecord` itself has no
/// "not yet asked" case, on purpose - see its own doc). `.failed` here means
/// EXACTLY what docs/PHASE4.md W4's "Privilege reality" note describes: the
/// expected LOCAL outcome without root, never a bug to paper over.
enum TunnelStatus: Equatable {
    case disconnected
    case connected(interface: String, handshakeSecsAgo: UInt64?)
    case failed(reason: String)
}

/// Live Remote (Distance) state for the SwiftUI shell (docs/PHASE4.md W4,
/// decision D11): owns a `RemoteHandle` (constructed once at `connect()`)
/// and drives Hetzner inventory reads, the WireGuard tunnel, and SSH ops
/// through it. Every field below the connection state is operator-entered -
/// there is no shared "environment" this whole panel depends on (see
/// `RemoteConnection`'s own doc), so Hetzner/WireGuard/SSH each fail
/// independently and honestly rather than collapsing into one empty state.
///
/// `RemoteHandle`'s exported methods are synchronous and can block (a real
/// Hetzner network read, or - for `connectTunnel` - up to several seconds of
/// `wireguard-go` bring-up). Every call into the handle below therefore runs
/// inside `Task.detached`, off this model's `@MainActor` isolation, exactly
/// like `CryptoModel`/`DrillsModel`/`EvidenceModel` - see `PolicyModel.swift`'s
/// own doc for the full rationale. `RemoteHandle` is generated as
/// `@unchecked Sendable`, so capturing it into a detached task's closure is
/// safe.
@MainActor
@Observable
final class RemoteModel {
    private(set) var connection: RemoteConnection = .connecting

    // MARK: - Hetzner inventory (read-only)

    /// Never persisted beyond this in-memory field; sent to `RemoteHandle`
    /// PER CALL, never stored Rust-side either (`RemoteHandle::list_hetzner`'s
    /// own doc: "the token is taken PER CALL, never stored on this handle").
    var hetznerToken: String = ""
    var hetznerLabelSelector: String = ""
    private(set) var hetznerServers: [HetznerServerRecord] = []
    private(set) var hetznerLoadedAt: Date?
    private(set) var isLoadingHetzner = false

    // MARK: - Remote environment config: the WireGuard peer + tunnel

    var wireguardGoBin: String = ""
    var interfaceName: String = ""
    /// The client-hosted Cloud's own WG peer public key (hex) - operator
    /// pastes this in from the box admin, mirroring `consolePublicKeyHex`'s
    /// own role in reverse.
    var peerPublicKeyHex: String = ""
    var endpoint: String = ""
    /// Comma-separated CIDRs, parsed on submit - a plain text field is a
    /// better fit than a dynamic list editor for the common one-or-two-CIDR
    /// case, mirroring how `EvidenceModel`'s own text fields stay plain
    /// strings rather than structured pickers wherever a free-text value is
    /// just as clear.
    var allowedIpsText: String = ""
    var persistentKeepaliveText: String = ""
    var localIp: String = ""
    var peerIp: String = ""

    // MARK: - Remote environment config: the SSH target

    var sshHost: String = ""
    var sshPortText: String = ""
    var sshUser: String = ""
    var sshIdentityFile: String = ""
    var sshPinnedHostKey: String = ""

    // MARK: - WireGuard tunnel state

    private(set) var consolePublicKeyB64: String?
    private(set) var consolePublicKeyHex: String?
    private(set) var tunnelStatus: TunnelStatus = .disconnected
    private(set) var isGeneratingKeypair = false
    private(set) var isConnectingTunnel = false

    // MARK: - SSH ops state

    private(set) var sshCheckResult: String?
    var sshDescriptorPath: String = ""
    private(set) var sshDescriptorBytes: Data?
    private(set) var sshDescriptorLoadedAt: Date?
    var sshTailPath: String = ""
    private(set) var sshTailOutput: String = ""
    private(set) var sshTailOffset: UInt64 = 0
    private(set) var isRunningSshOp = false

    private(set) var bannerMessage: String?

    private var handle: RemoteHandle?

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// Build a fresh handle. Called once from `init()`; also reachable from
    /// a "retry" affordance in the empty state. Deliberately resolves no
    /// environment of its own (see `RemoteConnection`'s own doc) - it only
    /// pre-fills every form field's `default*()` suggestion, once, without
    /// clobbering anything the operator already typed.
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        do {
            let newHandle = try await Task.detached { try RemoteHandle() }.value
            handle = newHandle
            connection = .ready
            await loadDefaults()
        } catch {
            handle = nil
            connection = .failed(reason: String(describing: error))
        }
    }

    /// Pre-fill every editable field from `RemoteHandle`'s own `default*()`
    /// getters - mirrors `CryptoModel.connect()`'s own "pre-fill once, never
    /// re-clobber an operator edit" contract: only ever touches a field that
    /// is STILL blank.
    private func loadDefaults() async {
        guard let handle else { return }
        if wireguardGoBin.isEmpty {
            wireguardGoBin = await Task.detached { handle.defaultWireguardGoBin() }.value ?? ""
        }
        if interfaceName.isEmpty {
            interfaceName = await Task.detached { handle.defaultInterface() }.value
        }
        if hetznerLabelSelector.isEmpty {
            hetznerLabelSelector = await Task.detached { handle.defaultHetznerLabelSelector() }.value
        }
        if hetznerToken.isEmpty {
            hetznerToken = await Task.detached { handle.defaultHetznerToken() }.value ?? ""
        }
        if localIp.isEmpty {
            localIp = await Task.detached { handle.defaultTunnelLocalIp() }.value
        }
        if peerIp.isEmpty {
            peerIp = await Task.detached { handle.defaultTunnelPeerIp() }.value
        }
        if persistentKeepaliveText.isEmpty {
            let keepalive = await Task.detached { handle.defaultPersistentKeepalive() }.value
            persistentKeepaliveText = keepalive.map(String.init) ?? ""
        }
        if sshPortText.isEmpty {
            sshPortText = await Task.detached { String(handle.defaultSshPort()) }.value
        }
    }

    // MARK: - Hetzner (read-only inventory)

    /// docs/PHASE4.md W4: "a read-scoped API token + optional label selector
    /// -> list boxes". Never creates/deletes - `RemoteHandle::list_hetzner`
    /// wraps a connector with no mutation method at all.
    func listHetzner() async {
        guard let handle else { return }
        let token = hetznerToken.trimmingCharacters(in: .whitespaces)
        guard !token.isEmpty else {
            bannerMessage = "Enter a Hetzner API token first."
            return
        }
        let selector = hetznerLabelSelector.trimmingCharacters(in: .whitespaces)
        isLoadingHetzner = true
        defer { isLoadingHetzner = false }
        bannerMessage = nil
        do {
            hetznerServers = try await Task.detached {
                try handle.listHetzner(token: token, labelSelector: selector.isEmpty ? nil : selector)
            }.value
            hetznerLoadedAt = Date()
        } catch {
            present(error)
        }
    }

    // MARK: - WireGuard tunnel (D11: the primary console-to-Cloud channel)

    /// Generate the console's session keypair and show its PUBLIC key for
    /// the box admin to paste into their peer config - docs/PHASE4.md W4:
    /// "generate the console WG keypair, show its PUBLIC key for the box
    /// admin". The private half never reaches this model at all (see
    /// `RemoteHandle`'s own module doc).
    func generateKeypair() async {
        guard let handle else { return }
        isGeneratingKeypair = true
        defer { isGeneratingKeypair = false }
        bannerMessage = nil
        do {
            let record = try await Task.detached { try handle.wgGenerateKeypair() }.value
            consolePublicKeyB64 = record.publicB64
            consolePublicKeyHex = record.publicHex
        } catch {
            present(error)
        }
    }

    /// Bring the tunnel up against the current peer-config fields. ALWAYS
    /// settles into a real `TunnelStatus`, including `.failed` - the
    /// expected LOCAL outcome without root privileges to create a tun
    /// device (docs/PHASE4.md W4's own "Privilege reality" note) - never a
    /// thrown error the banner would otherwise have to render instead of
    /// the status badge.
    func connectTunnel() async {
        guard let handle else { return }
        guard consolePublicKeyB64 != nil else {
            bannerMessage = "Generate the console keypair first."
            return
        }
        let bin = wireguardGoBin.trimmingCharacters(in: .whitespaces)
        let iface = interfaceName.trimmingCharacters(in: .whitespaces)
        let peerKey = peerPublicKeyHex.trimmingCharacters(in: .whitespaces)
        let ep = endpoint.trimmingCharacters(in: .whitespaces)
        let allowed = allowedIpsText
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        let lIp = localIp.trimmingCharacters(in: .whitespaces)
        let pIp = peerIp.trimmingCharacters(in: .whitespaces)
        guard !bin.isEmpty, !iface.isEmpty, !peerKey.isEmpty, !ep.isEmpty, !allowed.isEmpty, !lIp.isEmpty, !pIp.isEmpty
        else {
            bannerMessage =
                "Fill in the wireguard-go binary, interface, peer key, endpoint, allowed IPs, and tunnel addresses first."
            return
        }
        let keepalive = UInt16(persistentKeepaliveText.trimmingCharacters(in: .whitespaces))
        let inputs = ConnectTunnelInputs(
            wireguardGoBin: bin, interface: iface, peerPublicKeyHex: peerKey, endpoint: ep, allowedIps: allowed,
            persistentKeepalive: keepalive, listenPort: nil, localIp: lIp, peerIp: pIp)

        isConnectingTunnel = true
        defer { isConnectingTunnel = false }
        bannerMessage = nil
        tunnelStatus = await Task.detached { handle.connectTunnel(inputs: inputs) }.value.asModel
    }

    /// Re-read the live tunnel status - `RemoteView` polls this on a timer
    /// while connected so the "handshake Ns ago" badge stays honestly fresh
    /// without another `connectTunnel` attempt.
    func refreshTunnelStatus() async {
        guard let handle, connection.isReady else { return }
        tunnelStatus = await Task.detached { handle.tunnelStatus() }.value.asModel
    }

    func disconnectTunnel() async {
        guard let handle else { return }
        await Task.detached { handle.disconnectTunnel() }.value
        tunnelStatus = .disconnected
    }

    // MARK: - SSH ops (secondary to WireGuard - D11)

    private func currentSshTarget() -> SshTargetRecord? {
        let host = sshHost.trimmingCharacters(in: .whitespaces)
        let user = sshUser.trimmingCharacters(in: .whitespaces)
        let identity = sshIdentityFile.trimmingCharacters(in: .whitespaces)
        let pinned = sshPinnedHostKey.trimmingCharacters(in: .whitespaces)
        guard !host.isEmpty, !user.isEmpty, !identity.isEmpty, !pinned.isEmpty else { return nil }
        let port = UInt16(sshPortText.trimmingCharacters(in: .whitespaces)) ?? 22
        return SshTargetRecord(host: host, port: port, user: user, identityFile: identity, pinnedHostKey: pinned)
    }

    private static let sshTargetIncompleteMessage = "Fill in host, user, identity file, and pinned host key first."

    func sshCheck() async {
        guard let handle else { return }
        guard let target = currentSshTarget() else {
            bannerMessage = Self.sshTargetIncompleteMessage
            return
        }
        isRunningSshOp = true
        defer { isRunningSshOp = false }
        bannerMessage = nil
        sshCheckResult = nil
        do {
            try await Task.detached { try handle.sshCheck(target: target) }.value
            sshCheckResult = "reachable, host key verified"
        } catch {
            present(error)
        }
    }

    func sshReadDescriptor() async {
        guard let handle else { return }
        guard let target = currentSshTarget() else {
            bannerMessage = Self.sshTargetIncompleteMessage
            return
        }
        let path = sshDescriptorPath.trimmingCharacters(in: .whitespaces)
        guard !path.isEmpty else {
            bannerMessage = "Enter a remote path to read."
            return
        }
        isRunningSshOp = true
        defer { isRunningSshOp = false }
        bannerMessage = nil
        do {
            sshDescriptorBytes = try await Task.detached { try handle.sshReadDescriptor(target: target, path: path) }
                .value
            sshDescriptorLoadedAt = Date()
        } catch {
            sshDescriptorBytes = nil
            present(error)
        }
    }

    /// One bounded poll of `sshTailPath`'s tail, appended to the running
    /// `sshTailOutput` - see `RemoteHandle.sshTailOnce`'s own doc for why
    /// this is a manual/timed poll rather than a live stream. Call
    /// repeatedly (`RemoteView` wires a "Poll" button plus its own short
    /// auto-poll loop while the section is visible) to approximate tailing.
    func sshTailPoll() async {
        guard let handle else { return }
        guard let target = currentSshTarget() else {
            bannerMessage = Self.sshTargetIncompleteMessage
            return
        }
        let path = sshTailPath.trimmingCharacters(in: .whitespaces)
        guard !path.isEmpty else {
            bannerMessage = "Enter a remote path to tail."
            return
        }
        isRunningSshOp = true
        defer { isRunningSshOp = false }
        bannerMessage = nil
        let offset = sshTailOffset
        do {
            let chunk = try await Task.detached {
                try handle.sshTailOnce(target: target, path: path, fromOffset: offset)
            }.value
            if !chunk.isEmpty {
                sshTailOutput += String(decoding: chunk, as: UTF8.self)
                sshTailOffset += UInt64(chunk.count)
            }
        } catch {
            present(error)
        }
    }

    /// Clears the accumulated tail buffer and restarts from offset 0 - for
    /// switching to a different remote path, or just starting fresh.
    func resetTail() {
        sshTailOutput = ""
        sshTailOffset = 0
    }

    // MARK: - error presentation

    /// Fold any thrown error into the plain banner. Mirrors
    /// `CryptoModel.present`/`DrillsModel.present`/`EvidenceModel.present`.
    private func present(_ error: Error) {
        guard let remoteError = error as? RemoteError else {
            bannerMessage = String(describing: error)
            return
        }
        bannerMessage = Self.describe(remoteError)
    }

    private static func describe(_ error: RemoteError) -> String {
        switch error {
        case .Runtime(let reason):
            return "Could not start the local runtime: \(reason)"
        case .HetznerBuild(let reason):
            return "Could not build the Hetzner client: \(reason)"
        case .HetznerApi(let status, let body):
            return "Hetzner returned HTTP \(status): \(body)"
        case .HetznerTransport(let reason):
            return "Could not reach Hetzner: \(reason)"
        case .HetznerJson(let reason):
            return "Unexpected response from Hetzner: \(reason)"
        case .WgKeyGen(let reason):
            return "Could not generate a WireGuard keypair: \(reason)"
        case .InvalidTarget(let reason):
            return "Invalid SSH target: \(reason)"
        case .SshPin(let reason):
            return "Could not pin the SSH host key: \(reason)"
        case .SshSpawn(let reason):
            return "Could not run ssh: \(reason)"
        case .SshRemote(let code, let stderr):
            return "ssh exited \(code): \(stderr)"
        }
    }
}

/// `WgStatusRecord` (the FFI verdict) -> `TunnelStatus` (the Swift-side
/// mirror) - a one-to-one relabeling, kept as its own conversion rather than
/// reusing `WgStatusRecord` directly in view code so `RemoteModel`'s public
/// surface never leaks a raw FFI type any more than every sibling model's
/// does.
extension WgStatusRecord {
    fileprivate var asModel: TunnelStatus {
        switch self {
        case .disconnected:
            return .disconnected
        case .connected(let interface, let handshakeSecsAgo):
            return .connected(interface: interface, handshakeSecsAgo: handshakeSecsAgo)
        case .failed(let reason):
            return .failed(reason: reason)
        }
    }
}
