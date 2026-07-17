import GenaryxCoreFFI
import SwiftUI

/// The Identity panel: a load-once Identities list (type-filtered), the
/// 21-detector Alerts stream (severity + detector filtered, with a Rescan
/// button), and an Attestation surface derived from the relevant alerts.
/// Fed entirely by `IdentityModel` (the Idryx reads) - unlike `PolicyView`,
/// this panel has no bus-event filter section and no privileged mutation:
/// Identity is read-only this wave (docs/PHASE3.md W2), at parity with the
/// Tauri shell's own Identity panel (`apps/desktop/src-tauri/src/identity/*`
/// + its React panel).
///
/// PHASE3 W3 adds `onOpenAgent`: tapping an identity row is one of this
/// app's two Agent 360 deep-link entry points (the other is a delegation-
/// graph node tap, `DelegationGraphView.swift`) - `GenaryxApp` wires this to
/// its own `agentFocus` sheet state.
@MainActor
struct IdentityView: View {
    let model: IdentityModel
    let onOpenAgent: (String) -> Void

    private static let refreshInterval: Duration = .seconds(20)

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                IdentityEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task(id: model.connection.isReady) {
            guard model.connection.isReady else { return }
            while !Task.isCancelled {
                await model.refresh()
                try? await Task.sleep(for: Self.refreshInterval)
            }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                environmentChip

                if let notice = model.mutationNotice {
                    noticeBar(notice)
                }
                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                if model.isRefreshing && model.identities.isEmpty && model.alerts.isEmpty {
                    Text("loading the identity snapshot...")
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textTertiary)
                }

                section(title: "Identities") {
                    IdentitiesListSection(identities: model.identities, onOpenAgent: onOpenAgent)
                }
                section(title: "Alerts") {
                    AlertsSection(
                        alerts: model.alerts,
                        isRescanning: model.isRescanning,
                        rescanUnavailableReason: model.rescanUnavailableReason,
                        onRescan: { await model.rescan() }
                    )
                }
                section(title: "Attestation") {
                    AttestationSection(alerts: model.alerts)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private var environmentChip: some View {
        // Defensive-only: `body` already gates `content` (and therefore this
        // chip) on `model.connection.isReady` - same convention
        // `PolicyView.environmentChip` documents for its own unreachable
        // non-`.ready` branch.
        if case .ready(let source, let idryxUrl) = model.connection {
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    Circle().fill(Theme.steel).frame(width: 6, height: 6)
                    Text("\(sourceLabel(source)) \u{00B7} \(idryxUrl)")
                        .font(Theme.mono(11, weight: .medium))
                        .foregroundStyle(Theme.textSecondary)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Capsule().fill(Theme.panelElevated))
                .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))

                Text(LoadedAtFormat.label(model.loadedAt))
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)

                Spacer(minLength: 0)
            }
        }
    }

    private func sourceLabel(_ source: IdryxEnvSource) -> String {
        switch source {
        case .taipan(let name):
            "taipan up \u{00B7} \(name)"
        case .envFallback:
            "env fallback"
        }
    }

    private func noticeBar(_ text: String) -> some View {
        Text(text)
            .font(Theme.mono(11.5))
            .foregroundStyle(Theme.mint)
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

// MARK: - IdentitiesListSection

/// PHASE3.md row shape: id, type, source, owner, privileged, permission
/// count, `on_behalf_of` chain (root-first), events/alerts COUNTS. Type
/// filter: human / service_account / key / agent / mcp_server - an empty
/// selection shows every type (never "shows nothing"), toggled via
/// `FilterChip`.
@MainActor
private struct IdentitiesListSection: View {
    let identities: [IdentityRecord]
    let onOpenAgent: (String) -> Void

    @State private var selectedTypes: Set<String> = []

    private static let knownTypes = ["human", "service_account", "key", "agent", "mcp_server"]
    private static let displayLimit = 100

    private var filtered: [IdentityRecord] {
        guard !selectedTypes.isEmpty else { return identities }
        return identities.filter { selectedTypes.contains($0.identityType) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            typeFilterRow

            if identities.isEmpty {
                Text("no identities in this snapshot.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            } else if filtered.isEmpty {
                Text("no identities match the current type filter.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            } else {
                VStack(spacing: 8) {
                    ForEach(Array(filtered.prefix(Self.displayLimit)), id: \.id) { identity in
                        IdentityRow(identity: identity, onOpenAgent: onOpenAgent)
                    }
                }
                if filtered.count > Self.displayLimit {
                    Text("+\(filtered.count - Self.displayLimit) more (showing \(Self.displayLimit))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
        }
    }

    private var typeFilterRow: some View {
        HStack(spacing: 6) {
            ForEach(Self.knownTypes, id: \.self) { type in
                FilterChip(
                    label: type,
                    isSelected: selectedTypes.contains(type),
                    tone: Theme.iris,
                    onToggle: { toggleType(type) }
                )
            }
            Spacer(minLength: 0)
            Text("\(filtered.count) of \(identities.count)")
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textTertiary)
        }
    }

    private func toggleType(_ type: String) {
        if selectedTypes.contains(type) {
            selectedTypes.remove(type)
        } else {
            selectedTypes.insert(type)
        }
    }

    /// One identity: id / type / source / owner, privileged + admin-perm +
    /// remediation/rotation tags, the delegation chain, and the
    /// permission/events/alerts counts - explicitly labeled as counts
    /// (PHASE3.md: "label them as counts, not objects"). PHASE3 W3: the
    /// whole row is one of this app's two Agent 360 deep-link entry points -
    /// tapping it opens `identity.id`'s 360 card via `onOpenAgent`.
    private struct IdentityRow: View {
        let identity: IdentityRecord
        let onOpenAgent: (String) -> Void

        var body: some View {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top, spacing: 10) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(identity.id)
                            .font(Theme.mono(12, weight: .medium))
                            .foregroundStyle(Theme.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Text(subtitle)
                            .font(Theme.mono(10.5))
                            .foregroundStyle(Theme.textTertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                    Spacer(minLength: 8)
                    countsColumn
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(Theme.textTertiary)
                }

                if !tags.isEmpty {
                    HStack(spacing: 6) {
                        ForEach(Array(tags.enumerated()), id: \.offset) { _, tag in
                            tagPill(tag.0, tone: tag.1)
                        }
                    }
                }

                if !identity.onBehalfOf.isEmpty {
                    Text("on behalf of: \(identity.onBehalfOf.joined(separator: " \u{2192} "))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(Theme.panelElevated)
            )
            .contentShape(Rectangle())
            .onTapGesture { onOpenAgent(identity.id) }
            .help("Open \(identity.id)'s Agent 360 card")
        }

        private var subtitle: String {
            "\(identity.identityType) \u{00B7} \(identity.source) \u{00B7} \(identity.owner.isEmpty ? "no owner on record" : identity.owner)"
        }

        private var countsColumn: some View {
            VStack(alignment: .trailing, spacing: 2) {
                Text("\(identity.permissionCount) permission\(identity.permissionCount == 1 ? "" : "s")")
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                Text("\(identity.events) events \u{00B7} \(identity.alerts) alerts")
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
            }
        }

        private var tags: [(String, Color)] {
            var tags: [(String, Color)] = []
            if identity.privileged {
                tags.append(("privileged", Theme.amber))
            }
            if identity.adminPermissionCount > 0 {
                tags.append((
                    "\(identity.adminPermissionCount) admin perm\(identity.adminPermissionCount == 1 ? "" : "s")",
                    Theme.coral
                ))
            }
            if identity.remediation != nil {
                tags.append(("right-size suggested", Theme.mint))
            }
            if identity.rotation != nil {
                tags.append(("rotation suggested", Theme.violet))
            }
            return tags
        }

        private func tagPill(_ text: String, tone: Color) -> some View {
            Text(text)
                .font(Theme.mono(10, weight: .semibold))
                .foregroundStyle(tone)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(Capsule().fill(tone.opacity(0.14)))
                .overlay(Capsule().strokeBorder(tone.opacity(0.4), lineWidth: 1))
        }
    }
}

// MARK: - AlertsSection

/// PHASE3.md: the 21-detector alert stream, severity filters
/// (critical/high/medium/low) + a detector filter, plus the Rescan button
/// (`detect --format json`, disabled with an honest note when the binary is
/// unavailable). Row: detector, severity, identity, time, summary.
@MainActor
private struct AlertsSection: View {
    let alerts: [AlertRecord]
    let isRescanning: Bool
    let rescanUnavailableReason: String?
    let onRescan: () async -> Void

    @State private var selectedSeverities: Set<String> = []
    @State private var selectedDetector: String = AlertsSection.allDetectors

    private static let allDetectors = "all detectors"
    /// PHASE3.md's own named filter set - deliberately NOT the full
    /// `critical|high|medium|low|info|none` wire range idryx can emit (the
    /// two rarely-used tail values stay visible in the stream, just without
    /// a dedicated chip).
    private static let severities = ["critical", "high", "medium", "low"]
    private static let displayLimit = 80

    /// Built from whatever detectors are actually present in `alerts`,
    /// rather than a hard-coded 21-item list: a future 22nd detector still
    /// gets a working filter entry with zero maintenance here.
    private var detectorOptions: [String] {
        [Self.allDetectors] + Set(alerts.map(\.detector)).sorted()
    }

    private var filtered: [AlertRecord] {
        alerts.filter { alert in
            let severityOk = selectedSeverities.isEmpty || selectedSeverities.contains(alert.severity.lowercased())
            let detectorOk = selectedDetector == Self.allDetectors || alert.detector == selectedDetector
            return severityOk && detectorOk
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            toolbar

            if let rescanUnavailableReason {
                Text("Rescan unavailable: \(rescanUnavailableReason)")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if filtered.isEmpty {
                Text(alerts.isEmpty ? "no alerts in this snapshot." : "no alerts match the current filters.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            } else {
                let shown = Array(filtered.prefix(Self.displayLimit))
                VStack(spacing: 0) {
                    ForEach(Array(shown.enumerated()), id: \.offset) { index, alert in
                        AlertRow(alert: alert)
                        if index < shown.count - 1 {
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

                if filtered.count > Self.displayLimit {
                    Text("+\(filtered.count - Self.displayLimit) more (showing most recent \(Self.displayLimit))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
        }
    }

    private var toolbar: some View {
        HStack(spacing: 10) {
            HStack(spacing: 6) {
                ForEach(Self.severities, id: \.self) { severity in
                    FilterChip(
                        label: severity,
                        isSelected: selectedSeverities.contains(severity),
                        tone: Theme.severityColor(severity),
                        onToggle: { toggleSeverity(severity) }
                    )
                }
            }

            Picker("", selection: $selectedDetector) {
                ForEach(detectorOptions, id: \.self) { detector in
                    Text(detector).tag(detector)
                }
            }
            .labelsHidden()
            .pickerStyle(.menu)
            .font(Theme.mono(11))
            .frame(width: 200)

            Spacer(minLength: 8)

            rescanButton
        }
    }

    private func toggleSeverity(_ severity: String) {
        if selectedSeverities.contains(severity) {
            selectedSeverities.remove(severity)
        } else {
            selectedSeverities.insert(severity)
        }
    }

    private var rescanButton: some View {
        Button {
            Task { await onRescan() }
        } label: {
            HStack(spacing: 5) {
                if isRescanning {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(isRescanning ? "Rescanning..." : "Rescan")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(rescanDisabled ? Theme.textTertiary : Theme.mint)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.mint.opacity(rescanDisabled ? 0.05 : 0.14)))
            .overlay(Capsule().strokeBorder(Theme.mint.opacity(rescanDisabled ? 0.15 : 0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(rescanDisabled)
        .help(rescanUnavailableReason ?? "Recompute the 21 detectors over the current stack bus files")
    }

    private var rescanDisabled: Bool {
        isRescanning || rescanUnavailableReason != nil
    }

    private struct AlertRow: View {
        let alert: AlertRecord

        var body: some View {
            HStack(spacing: 12) {
                severityBadge

                Text(alert.detector)
                    .font(Theme.mono(11.5, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(width: 160, alignment: .leading)

                Text(alert.identity)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.head)
                    .frame(width: 190, alignment: .leading)

                Text(alert.summary)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Text(MoneyFormat.timestamp(alert.time))
                    .font(Theme.mono(11))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
                    .frame(width: 118, alignment: .trailing)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        }

        private var severityBadge: some View {
            let color = Theme.severityColor(alert.severity)
            return HStack(spacing: 6) {
                Circle()
                    .fill(color)
                    .frame(width: 7, height: 7)
                    .shadow(color: color.opacity(0.6), radius: 3)
                Text(alert.severity.uppercased())
                    .font(Theme.mono(10, weight: .semibold))
                    .tracking(0.6)
            }
            .foregroundStyle(Theme.textSecondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(color.opacity(0.14)))
            .overlay(Capsule().strokeBorder(color.opacity(0.4), lineWidth: 1))
            .frame(width: 92, alignment: .leading)
        }
    }
}

// MARK: - AttestationSection

/// PHASE3.md: "Attestation status surfaces via `attestation_missing`/
/// `bom_incomplete` alerts (honest: not a clean field)". Never invents a
/// structured attestation value - see `AlertRecord.attestationValue`'s own
/// doc comment (`IdentityComponents.swift`) for the best-effort parse, and
/// its fallback to the raw `summary` when that parse comes up empty.
@MainActor
private struct AttestationSection: View {
    let alerts: [AlertRecord]

    private static let detectors: Set<String> = ["attestation_missing", "bom_incomplete"]

    private var relevant: [AlertRecord] {
        alerts.filter { Self.detectors.contains($0.detector) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(
                "Idryx does not carry attestation as a structured identity field; this list is derived from the attestation_missing and bom_incomplete detector alerts instead."
            )
            .font(Theme.mono(10.5))
            .foregroundStyle(Theme.textTertiary)
            .fixedSize(horizontal: false, vertical: true)

            if relevant.isEmpty {
                Text("no attestation or BOM findings in this snapshot.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(relevant.enumerated()), id: \.offset) { index, alert in
                        AttestationRow(alert: alert)
                        if index < relevant.count - 1 {
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
    }

    private struct AttestationRow: View {
        let alert: AlertRecord

        var body: some View {
            HStack(spacing: 12) {
                Text(alert.detector)
                    .font(Theme.mono(10, weight: .semibold))
                    .foregroundStyle(Theme.violet)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(Theme.violet.opacity(0.14)))
                    .overlay(Capsule().strokeBorder(Theme.violet.opacity(0.4), lineWidth: 1))
                    .frame(width: 150, alignment: .leading)

                Text(alert.identity)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.head)
                    .frame(width: 190, alignment: .leading)

                Text(attestationText)
                    .font(Theme.mono(11))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        }

        private var attestationText: String {
            if let value = alert.attestationValue {
                return "attestation: \(value)"
            }
            return alert.summary
        }
    }
}
