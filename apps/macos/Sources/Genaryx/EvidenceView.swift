import GenaryxCoreFFI
import SwiftUI

/// The Evidence Center panel (docs/PHASE4.md W3): source toggles (Cloud,
/// Qryx, Agent-BOM, FOCUS), a "Build evidence pack" action that saves the
/// result via `NSSavePanel`, and a manifest/contents view (a signed/UNSIGNED
/// badge, an artifacts table, and a separate "Not included" list). Fed by
/// `EvidenceModel` (its own toggles/fields/build state) PLUS `cloudModel`
/// directly (`EvidenceModel.swift`'s own doc: the build itself runs through
/// `cloudModel.cloudHandle`, the SAME paired device Money/Overview use) -
/// mirrors how `PostureView` is handed `cloudModel` alongside its own model
/// rather than owning a handle of its own.
///
/// Unlike every other Phase-4 panel, this one gates its WHOLE content on
/// `cloudModel.connection.isReady`, not a connection state of its own: every
/// source here (even a Qryx-only or idryx-only pack) is still gathered and
/// signed through the paired `CloudHandle`, so an unpaired Cloud makes the
/// entire Evidence Center unavailable (`EvidenceEmptyStateView`), while an
/// individual unresolved TOOL (qryx/idryx/tokenfuse) only disables that one
/// source's own toggle within an otherwise-usable panel - the same
/// distinction `CryptoView`/`DrillsView` draw between "no crypto/drills
/// plane at all" and "this one scan/run attempt failed".
@MainActor
struct EvidenceView: View {
    let model: EvidenceModel
    let cloudModel: CloudModel

    var body: some View {
        Group {
            if cloudModel.connection.isReady {
                content
            } else {
                EvidenceEmptyStateView(connection: cloudModel.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task(id: cloudModel.connection.isReady) {
            await model.loadDefaults(cloudModel: cloudModel)
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                section(title: "Sources") {
                    sourceTogglesSection
                }
                buildRow

                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }
                if let lastSavedPath = model.lastSavedPath {
                    savedNotice(lastSavedPath)
                }

                if let pack = model.lastPack {
                    section(title: "Manifest") {
                        ManifestHeaderView(pack: pack)
                    }
                    section(title: "Artifacts") {
                        ArtifactsTableSection(artifacts: pack.manifest.artifacts)
                    }
                    section(title: "Not Included") {
                        MissingSourcesSection(missing: pack.manifest.missing)
                    }
                } else {
                    section(title: "Manifest") {
                        Text(model.isBuilding ? "building..." : "build a pack to see its manifest.")
                            .font(Theme.mono(12))
                            .foregroundStyle(Theme.textTertiary)
                            .padding(.vertical, 4)
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
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

    private func savedNotice(_ path: String) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(Theme.mint)
                .font(.system(size: 11))
            Text("saved to \(path)")
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }

    // MARK: - source toggles

    /// docs/PHASE4.md W3: "source toggles (Cloud, Qryx, Agent-BOM, FOCUS)
    /// disabled when unavailable".
    private var sourceTogglesSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            EvidenceSourceRow(
                title: "Cloud", subtitle: "compliance evidence + audit-chain verdict",
                isOn: Binding(get: { model.includeCloud }, set: { model.includeCloud = $0 }),
                disabledReason: nil
            ) {
                EmptyView()
            }

            EvidenceSourceRow(
                title: "Qryx", subtitle: "CNSA crypto evidence + CBOM",
                isOn: Binding(get: { model.qryxEnabled }, set: { model.qryxEnabled = $0 }),
                disabledReason: qryxDisabledReason
            ) {
                fieldRow("qryx binary", text: Binding(get: { model.qryxBin }, set: { model.qryxBin = $0 }))
                fieldRow("scan target", text: Binding(get: { model.qryxTarget }, set: { model.qryxTarget = $0 }))
                fieldRow(
                    "sign key (optional, PEM path)",
                    text: Binding(get: { model.qryxSignKeyPath }, set: { model.qryxSignKeyPath = $0 }))
            }

            EvidenceSourceRow(
                title: "Agent-BOM", subtitle: "idryx CycloneDX agent bill of materials",
                isOn: Binding(get: { model.idryxEnabled }, set: { model.idryxEnabled = $0 }),
                disabledReason: idryxDisabledReason
            ) {
                fieldRow("idryx binary", text: Binding(get: { model.idryxBin }, set: { model.idryxBin = $0 }))
            }

            EvidenceSourceRow(
                title: "FOCUS", subtitle: "TokenFuse FinOps cost export",
                isOn: Binding(get: { model.tokenfuseEnabled }, set: { model.tokenfuseEnabled = $0 }),
                disabledReason: tokenfuseDisabledReason
            ) {
                fieldRow(
                    "tokenfuse binary", text: Binding(get: { model.tokenfuseBin }, set: { model.tokenfuseBin = $0 }))
                fieldRow(
                    "traces directory",
                    text: Binding(get: { model.tokenfuseTracesDir }, set: { model.tokenfuseTracesDir = $0 }))
                HStack(spacing: 8) {
                    fieldRow(
                        "from (optional, RFC 3339)",
                        text: Binding(get: { model.tokenfuseFrom }, set: { model.tokenfuseFrom = $0 }))
                    fieldRow(
                        "to (optional, RFC 3339)",
                        text: Binding(get: { model.tokenfuseTo }, set: { model.tokenfuseTo = $0 }))
                }
            }
        }
    }

    private var qryxDisabledReason: String? {
        model.qryxBin.trimmingCharacters(in: .whitespaces).isEmpty
            ? "no qryx binary found - set one below to enable" : nil
    }

    private var idryxDisabledReason: String? {
        model.idryxBin.trimmingCharacters(in: .whitespaces).isEmpty
            ? "no idryx binary found - set one below to enable" : nil
    }

    private var tokenfuseDisabledReason: String? {
        model.tokenfuseBin.trimmingCharacters(in: .whitespaces).isEmpty
            ? "no tokenfuse binary found - set one below to enable" : nil
    }

    private func fieldRow(_ placeholder: String, text: Binding<String>) -> some View {
        TextField(placeholder, text: text)
            .textFieldStyle(.plain)
            .font(Theme.mono(11))
            .foregroundStyle(Theme.textPrimary)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panel))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))
    }

    // MARK: - build + save

    private var buildRow: some View {
        HStack(spacing: 10) {
            buildButton
            if model.lastPack != nil {
                saveButton
            }
            Spacer(minLength: 0)
        }
    }

    private var buildButton: some View {
        Button {
            Task {
                if await model.build(cloudModel: cloudModel) {
                    model.save()
                }
            }
        } label: {
            HStack(spacing: 5) {
                if model.isBuilding {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "shippingbox.fill")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(model.isBuilding ? "Building..." : "Build evidence pack")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.mint)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.mint.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.mint.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isBuilding)
    }

    private var saveButton: some View {
        Button {
            model.save()
        } label: {
            Text("Save to disk\u{2026}")
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(Theme.textSecondary)
        }
        .buttonStyle(.plain)
        .disabled(model.isBuilding)
    }
}

// MARK: - EvidenceSourceRow

/// One source toggle row: a checkbox with a title/subtitle (or an inline
/// disabled reason in place of the subtitle), plus its own editable fields
/// revealed only while the toggle is on and available. Shared by all four
/// rows in `EvidenceView.sourceTogglesSection`.
@MainActor
private struct EvidenceSourceRow<Fields: View>: View {
    let title: String
    let subtitle: String
    @Binding var isOn: Bool
    /// Non-`nil` disables the toggle and shows this text in place of
    /// `subtitle` - an honest, specific reason (docs/PHASE4.md W3: "source
    /// toggles... disabled when unavailable"), never a bare greyed-out
    /// control with no explanation.
    let disabledReason: String?
    @ViewBuilder var fields: () -> Fields

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Toggle(isOn: $isOn) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(title)
                        .font(Theme.mono(12, weight: .semibold))
                        .foregroundStyle(Theme.textPrimary)
                    Text(disabledReason ?? subtitle)
                        .font(Theme.mono(10))
                        .foregroundStyle(disabledReason != nil ? Theme.coral : Theme.textSecondary)
                }
            }
            .toggleStyle(.checkbox)
            .disabled(disabledReason != nil)

            if isOn && disabledReason == nil {
                VStack(alignment: .leading, spacing: 6) {
                    fields()
                }
                .padding(.leading, 22)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panelElevated)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .strokeBorder(Theme.hairline, lineWidth: 1)
        )
    }
}

// MARK: - ManifestHeaderView

/// docs/PHASE4.md W3: "a manifest/contents view (pack header + a
/// signed/UNSIGNED badge)". `signed: false` is rendered exactly as
/// prominently as `true` - an honest UNSIGNED badge, never soft-pedaled or
/// hidden (06 §0.5).
@MainActor
private struct ManifestHeaderView: View {
    let pack: EvidencePackRecord

    private var manifest: EvidenceManifestRecord { pack.manifest }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                signedBadge
                if !pack.journaled {
                    journalNotRecordedBadge
                }
                Spacer(minLength: 0)
            }
            Text(
                "generated \(MoneyFormat.timestamp(manifest.generatedAt)) \u{00B7} operator \(manifest.operatorName) \u{00B7} org \(manifest.org)"
            )
            .font(Theme.mono(10.5))
            .foregroundStyle(Theme.textTertiary)
            .lineLimit(1)
            .truncationMode(.middle)
            Text(
                "\(manifest.artifacts.count) artifact\(manifest.artifacts.count == 1 ? "" : "s") \u{00B7} \(manifest.missing.count) not included \u{00B7} pack version \(manifest.packVersion)"
            )
            .font(Theme.mono(10.5))
            .foregroundStyle(Theme.textSecondary)
        }
    }

    private var signedBadge: some View {
        let color = pack.signed ? Theme.mint : Theme.coral
        return Text(pack.signed ? "SIGNED" : "UNSIGNED")
            .font(Theme.mono(11, weight: .bold))
            .tracking(0.8)
            .foregroundStyle(color)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(Capsule().fill(color.opacity(0.16)))
            .overlay(Capsule().strokeBorder(color.opacity(0.45), lineWidth: 1))
    }

    private var journalNotRecordedBadge: some View {
        Text("JOURNAL NOT RECORDED")
            .font(Theme.mono(10, weight: .semibold))
            .foregroundStyle(Theme.amber)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(Theme.amber.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.amber.opacity(0.4), lineWidth: 1))
    }
}

// MARK: - ArtifactsTableSection

/// docs/PHASE4.md W3: "artifacts table with name/source/short-sha256/size/
/// verify_status".
@MainActor
private struct ArtifactsTableSection: View {
    let artifacts: [ManifestArtifactRecord]

    var body: some View {
        if artifacts.isEmpty {
            Text("no artifacts in this pack.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 4)
        } else {
            VStack(spacing: 0) {
                header
                Divider().overlay(Theme.hairlineStrong)
                ForEach(Array(artifacts.enumerated()), id: \.offset) { index, artifact in
                    ArtifactRow(artifact: artifact)
                    if index < artifacts.count - 1 {
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
    }

    private var header: some View {
        HStack(spacing: 10) {
            columnLabel("NAME").frame(maxWidth: .infinity, alignment: .leading)
            columnLabel("SOURCE").frame(width: 170, alignment: .leading)
            columnLabel("SHA-256").frame(width: 110, alignment: .leading)
            columnLabel("SIZE").frame(width: 70, alignment: .trailing)
            columnLabel("VERIFY").frame(width: 150, alignment: .trailing)
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

    private struct ArtifactRow: View {
        let artifact: ManifestArtifactRecord

        var body: some View {
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(artifact.name)
                        .font(Theme.mono(11.5, weight: .medium))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Text(artifact.filename)
                        .font(Theme.mono(9.5))
                        .foregroundStyle(Theme.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Text(artifact.source)
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(2)
                    .truncationMode(.tail)
                    .frame(width: 170, alignment: .leading)

                Text(EvidenceHashFormat.short(artifact.sha256))
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textSecondary)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .frame(width: 110, alignment: .leading)

                Text(EvidenceSizeFormat.label(artifact.sizeBytes))
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .frame(width: 70, alignment: .trailing)

                Text(artifact.verifyStatus ?? "\u{2014}")
                    .font(Theme.mono(9.5))
                    .foregroundStyle(artifact.verifyStatus == nil ? Theme.textTertiary : Theme.mint)
                    .lineLimit(2)
                    .truncationMode(.tail)
                    .multilineTextAlignment(.trailing)
                    .frame(width: 150, alignment: .trailing)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        }
    }
}

// MARK: - MissingSourcesSection

/// docs/PHASE4.md W3: "a separate 'Not included' list from missing" - every
/// requested source that could not be gathered, with its own reason,
/// verbatim. Never dropped, never folded into the artifacts table.
@MainActor
private struct MissingSourcesSection: View {
    let missing: [MissingSourceRecord]

    var body: some View {
        if missing.isEmpty {
            Text("nothing left out - every enabled source made it into the pack.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 4)
        } else {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(missing.enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: "minus.circle")
                            .font(.system(size: 11))
                            .foregroundStyle(Theme.textTertiary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.name)
                                .font(Theme.mono(11.5, weight: .medium))
                                .foregroundStyle(Theme.textPrimary)
                            Text(item.reason)
                                .font(Theme.mono(10.5))
                                .foregroundStyle(Theme.textSecondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer(minLength: 0)
                    }
                }
            }
            .padding(12)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
        }
    }
}
