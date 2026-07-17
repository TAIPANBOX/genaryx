import AppKit
import Foundation
import GenaryxCoreFFI
import Observation
import UniformTypeIdentifiers

/// Live Evidence Center state for the SwiftUI shell (docs/PHASE4.md W3).
/// Unlike every other Phase-4 panel, this one owns NO handle of its own: the
/// build lives on `CloudHandle` (`crates/ffi/src/cloud/mod.rs`), the SAME
/// paired device/signing key Money/Overview already use, so this model
/// borrows `CloudModel.cloudHandle` at call time rather than pairing (or
/// resolving) anything independently - `build(cloudModel:)`/
/// `loadDefaults(cloudModel:)` both take a `CloudModel` argument for exactly
/// this reason, mirroring how `PostureModel` is handed the models it reads
/// rather than owning any of its own.
///
/// Source toggles (Cloud, Qryx, Agent-BOM, FOCUS) are independent and
/// honest: a source with no resolvable binary is disabled with an inline
/// reason, never silently dropped from the manifest without explanation -
/// the returned pack's own `manifest.missing` names exactly what a source
/// that WAS requested still failed to produce, and what got left out never
/// even reaches that list. Every resolved default is pre-filled but
/// editable ("operator can see/set it, never enforced" - the same idiom
/// `CryptoModel.scanTarget`/`DrillsModel.scenarioDir` already establish for
/// their own panels).
///
/// `CloudHandle`'s exported methods are synchronous and can block (a real
/// qryx/idryx/tokenfuse subprocess run, or Cloud network I/O). Every call
/// into the handle below therefore runs inside `Task.detached`, off this
/// model's `@MainActor` isolation, exactly like `CryptoModel`/`DrillsModel` -
/// see `PolicyModel.swift`'s own doc for the full rationale. `CloudHandle`
/// is generated as `@unchecked Sendable`, so capturing it into a detached
/// task's closure is safe.
@MainActor
@Observable
final class EvidenceModel {
    // MARK: - source toggles + editable fields

    /// Always available once this panel is reachable at all (the whole
    /// Evidence Center gates on a paired `CloudHandle` - see
    /// `EvidenceView`'s own doc), so this toggle needs no disabled reason.
    var includeCloud = true

    var qryxEnabled = false
    var qryxBin = ""
    var qryxTarget = ""
    /// Never pre-filled - there is no honest default for a private signing
    /// key path (mirrors `CryptoModel.verifyFilePath`'s own "no honest
    /// default" precedent). Optional even when Qryx itself is enabled: an
    /// unsigned Qryx evidence bundle inside the pack is a normal, valid
    /// choice.
    var qryxSignKeyPath = ""

    var idryxEnabled = false
    var idryxBin = ""
    /// Read-only: the stack-bus `--load source:path` pairs
    /// `evidenceEnvDefaults()` resolved (the same ones `IdryxHandle.rescan`
    /// uses) - not operator-editable, there is no natural per-pair UI for
    /// this and it is deliberately internal plumbing, mirroring how
    /// `IdryxHandle.rescan` itself never exposes its own `--load` pairs to
    /// the Identity panel either.
    private(set) var idryxLoads: [EvidenceLoadEntry] = []

    var tokenfuseEnabled = false
    var tokenfuseBin = ""
    var tokenfuseTracesDir = ""
    var tokenfuseFrom = ""
    var tokenfuseTo = ""

    // MARK: - build state

    private(set) var lastPack: EvidencePackRecord?
    private(set) var bannerMessage: String?
    private(set) var isBuilding = false
    /// The path the operator last actually saved a pack to (`NSSavePanel`
    /// confirmed, not cancelled) - shown as a small confirmation line, never
    /// load-bearing state.
    private(set) var lastSavedPath: String?
    private(set) var defaultsLoaded = false

    // MARK: - defaults (best-effort; cheap, local-only, no subprocess)

    /// Pre-fill every editable field from `CloudHandle.evidenceEnvDefaults()`
    /// - called once (guarded by `defaultsLoaded`) when the panel first has a
    /// live `cloudModel.cloudHandle` to ask (`EvidenceView`'s own
    /// `.task(id:)`). Mirrors `CryptoModel.connect()`'s own "pre-fill once,
    /// never re-clobber an operator edit" contract: only ever touches a
    /// field that is STILL blank.
    func loadDefaults(cloudModel: CloudModel) async {
        guard !defaultsLoaded, let handle = cloudModel.cloudHandle else { return }
        defaultsLoaded = true

        let defaults = await Task.detached { handle.evidenceEnvDefaults() }.value
        if qryxBin.isEmpty { qryxBin = defaults.qryxBin ?? "" }
        if qryxTarget.isEmpty { qryxTarget = defaults.qryxScanTarget }
        if idryxBin.isEmpty { idryxBin = defaults.idryxBin ?? "" }
        idryxLoads = defaults.idryxLoads
        if tokenfuseBin.isEmpty { tokenfuseBin = defaults.tokenfuseBin ?? "" }
        if tokenfuseTracesDir.isEmpty { tokenfuseTracesDir = defaults.tokenfuseTracesDir ?? "" }

        // Enable a source by default exactly when it resolved a binary -
        // "operator can see/set it, never enforced": the operator can still
        // flip any of these off (or on, if a binary shows up later and they
        // retype the path) before pressing Build.
        qryxEnabled = !qryxBin.trimmingCharacters(in: .whitespaces).isEmpty
        idryxEnabled = !idryxBin.trimmingCharacters(in: .whitespaces).isEmpty
        tokenfuseEnabled = !tokenfuseBin.trimmingCharacters(in: .whitespaces).isEmpty
    }

    // MARK: - build

    /// Build a pack from the current toggles/fields through
    /// `cloudModel.cloudHandle`, then immediately offer an `NSSavePanel` on
    /// success (docs/PHASE4.md W3: "a 'Build evidence pack' action -> calls
    /// the handle -> writes the returned zip_bytes to disk via an
    /// NSSavePanel"). A source whose toggle is off is never sent, regardless
    /// of what its text fields still hold - flipping a toggle off is an
    /// honest "do not use this", not just a UI hide.
    @discardableResult
    func build(cloudModel: CloudModel) async -> Bool {
        guard let handle = cloudModel.cloudHandle else {
            bannerMessage = "Connect to Cloud (Money or Overview) before building an evidence pack."
            return false
        }
        guard !isBuilding else { return false }

        let trimmedQryxTarget = qryxTarget.trimmingCharacters(in: .whitespaces)
        if qryxEnabled && trimmedQryxTarget.isEmpty {
            bannerMessage = "Enter a Qryx scan target, or turn off Qryx."
            return false
        }
        let trimmedTracesDir = tokenfuseTracesDir.trimmingCharacters(in: .whitespaces)
        if tokenfuseEnabled && trimmedTracesDir.isEmpty {
            bannerMessage = "Enter a FOCUS traces directory, or turn off FOCUS."
            return false
        }
        if !includeCloud && !qryxEnabled && !idryxEnabled && !tokenfuseEnabled {
            bannerMessage = "Enable at least one source before building."
            return false
        }

        isBuilding = true
        defer { isBuilding = false }
        bannerMessage = nil

        let stamp = Self.isoFormatter.string(from: Date())
        let inputs = EvidenceBuildInputs(
            operatorName: nil, // CloudHandle falls back to its own paired console_operator()
            org: nil, // CloudHandle falls back to its own paired org_domain()
            generatedAt: stamp,
            includeCloud: includeCloud,
            qryxBin: nonBlank(qryxEnabled ? qryxBin : nil),
            qryxTarget: nonBlank(qryxEnabled ? qryxTarget : nil),
            qryxSignKey: nonBlank(qryxEnabled ? qryxSignKeyPath : nil),
            idryxBin: nonBlank(idryxEnabled ? idryxBin : nil),
            idryxLoads: idryxEnabled ? idryxLoads : [],
            tokenfuseBin: nonBlank(tokenfuseEnabled ? tokenfuseBin : nil),
            tokenfuseTracesDir: nonBlank(tokenfuseEnabled ? tokenfuseTracesDir : nil),
            tokenfuseFrom: nonBlank(tokenfuseEnabled ? tokenfuseFrom : nil),
            tokenfuseTo: nonBlank(tokenfuseEnabled ? tokenfuseTo : nil))

        do {
            let pack = try await Task.detached { try handle.buildEvidencePack(inputs: inputs) }.value
            lastPack = pack
            return true
        } catch {
            present(error)
            return false
        }
    }

    /// `nil` for a missing/blank string, otherwise the original value -
    /// mirrors `DrillsModel.run`'s own `apiKey.isEmpty ? nil : apiKeyValue`
    /// empty-to-nil convention at this exact FFI boundary shape
    /// (`Option<String>` parameters).
    private func nonBlank(_ s: String?) -> String? {
        guard let s, !s.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return s
    }

    // MARK: - save

    /// Writes `lastPack.zipBytes` to disk via `NSSavePanel` - the operator's
    /// own save-dialog confirm click is the only permission this needs
    /// (never writes anywhere without it; a cancelled panel is a silent
    /// no-op, not an error). Default name `genaryx-evidence-<stamp>.zip`
    /// (docs/PHASE4.md W3).
    func save() {
        guard let pack = lastPack else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.zip]
        panel.nameFieldStringValue = Self.defaultFilename()
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try pack.zipBytes.write(to: url, options: .atomic)
            lastSavedPath = url.path
        } catch {
            bannerMessage = "Could not save the evidence pack: \(error.localizedDescription)"
        }
    }

    private static func defaultFilename() -> String {
        "genaryx-evidence-\(filenameStampFormatter.string(from: Date())).zip"
    }

    // `nonisolated(unsafe)`: each formatter is configured exactly once at
    // first access and only ever read afterward, the same reasoning
    // `MoneyFormat`'s own formatters document.
    private nonisolated(unsafe) static let isoFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private static let filenameStampFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "yyyyMMdd-HHmmss"
        f.locale = Locale(identifier: "en_US_POSIX")
        return f
    }()

    // MARK: - error presentation

    /// Fold any thrown error into the plain banner. Mirrors
    /// `CryptoModel.present`/`DrillsModel.present`.
    private func present(_ error: Error) {
        guard let evidenceError = error as? EvidenceError else {
            bannerMessage = String(describing: error)
            return
        }
        switch evidenceError {
        case .NoArtifacts:
            bannerMessage = "No evidence sources were available - enable at least one source with a valid path."
        case .Sign(let reason):
            bannerMessage = "Evidence manifest signing failed: \(reason)"
        case .Assemble(let reason):
            bannerMessage = "Evidence assembly failed: \(reason)"
        }
    }
}
