import GenaryxCoreFFI
import SwiftUI

/// The Memory panel: store stats (auto-refreshed), an on-demand recall query
/// with ranked results, a Provenance card for a selected memory (`why`,
/// branching semantic vs episodic) with a guarded Forget action, and a live
/// Timeline filtered from the shared bus feed. Fed by `MemoryModel` (the
/// Engram reads) plus the app's own `FleetModel` bus events - the Timeline is
/// a FILTER over the same live tail the Bus Explorer renders, never a new
/// read through `MemoryHandle` (docs/PHASE4.md W2: "NOT a new read - mirror
/// how PolicyModel/PolicyView filter bus events"), at parity with the Tauri
/// shell's own Memory panel.
@MainActor
struct MemoryView: View {
    let model: MemoryModel
    /// The app-wide bus feed (`FleetModel.events`), filtered below to
    /// `source == "engram"` for the Timeline - see the type doc.
    let busEvents: [UiEvent]

    private static let refreshInterval: Duration = .seconds(20)

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                MemoryEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task(id: model.connection.isReady) {
            guard model.connection.isReady else { return }
            while !Task.isCancelled {
                await model.refreshStats()
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

                section(title: "Store Stats") {
                    StoreStatsSection(stats: model.stats, loadedAt: model.statsLoadedAt, isLoading: model.isLoadingStats)
                }
                section(title: "Recall") {
                    RecallSection(model: model)
                }
                section(title: "Provenance") {
                    ProvenanceSection(model: model)
                }
                section(title: "Timeline") {
                    MemoryTimelineSection(events: timelineEvents)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// docs/PHASE4.md W2: every `engram.*` event (`memory_written`,
    /// `memory_forgotten`, `reflection_run`, `contradiction_found`), not just
    /// one type - mirrors `PolicyView`'s own source-only Decision Stream
    /// filter, not `QualityView`'s narrower single-type `quality_drift` one.
    private var timelineEvents: [UiEvent] {
        busEvents.filter { $0.source.lowercased() == "engram" }
    }

    @ViewBuilder
    private var environmentChip: some View {
        // Defensive-only: `body` already gates `content` (and therefore this
        // chip) on `model.connection.isReady` - same convention
        // `IdentityView.environmentChip` documents for its own unreachable
        // non-`.ready` branch.
        if case .ready(let source, let engramMcpBin, let dbPath, let agentId) = model.connection {
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    Circle().fill(Theme.sourceColor("engram")).frame(width: 6, height: 6)
                    Text("\(sourceLabel(source)) \u{00B7} \(engramMcpBin) \u{00B7} \(dbPath)")
                        .font(Theme.mono(11, weight: .medium))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Capsule().fill(Theme.panelElevated))
                .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))

                if let agentId {
                    Text("scope \u{00B7} \(agentId)")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Text(MemoryStatsFormat.label(model.statsLoadedAt))
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)

                Spacer(minLength: 0)
            }
        }
    }

    private func sourceLabel(_ source: MemoryEnvSource) -> String {
        switch source {
        case .taipan:
            "taipan \u{00B7} well-known"
        case .pathEnv:
            "$PATH"
        case .explicit:
            "explicit path"
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

// MARK: - StoreStatsSection

/// docs/PHASE4.md W2: episodic/semantic/procedural counts (`procedural`
/// labeled "not implemented in this Engram version", never a real 0) + active/
/// superseded facts, entities, reflections, vector-index size, db path/size.
@MainActor
private struct StoreStatsSection: View {
    let stats: EngramStatsRecord?
    let loadedAt: Date?
    let isLoading: Bool

    var body: some View {
        if let stats {
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 12) {
                    StatTile(label: "episodic", value: String(stats.counts.episodic))
                    StatTile(label: "semantic", value: String(stats.counts.semantic))
                    proceduralTile(stats.counts.procedural)
                    StatTile(label: "entities", value: String(stats.entities))
                    StatTile(label: "reflections", value: String(stats.reflections))
                }
                HStack(spacing: 12) {
                    StatTile(label: "facts total", value: String(stats.factsTotal))
                    StatTile(label: "facts active", value: String(stats.factsActive), tone: Theme.mint)
                    StatTile(label: "facts superseded", value: String(stats.factsSuperseded), tone: Theme.textTertiary)
                    StatTile(label: "vector index", value: String(stats.vectorIndexSize))
                }
                Text("\(stats.dbPath) \u{00B7} \(dbSizeText(stats.dbSizeBytes))")
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
            }
        } else {
            Text(isLoading ? "loading store stats..." : "no store stats yet.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 4)
        }
    }

    /// docs/PHASE4.md W2 guard: `procedural` is ALWAYS 0 in this Engram
    /// version (the store implements only episodic+semantic -
    /// `EngramCountsRecord.procedural`'s own doc) - labeled honestly, never
    /// shown as a plain live zero the operator could mistake for "nothing
    /// procedural happened yet".
    private func proceduralTile(_ value: Int64) -> some View {
        StatTile(label: "procedural", value: String(value), sub: "not implemented in this Engram version", tone: Theme.textTertiary)
    }

    /// `None` renders "in-memory / n/a" (docs/PHASE4.md W2 guard), never a
    /// fabricated 0 or a raw byte count with no units at small sizes.
    private func dbSizeText(_ bytes: Int64?) -> String {
        guard let bytes, bytes > 0 else { return "in-memory / n/a" }
        if bytes < 1_000_000 {
            return String(format: "%.1f KB", Double(bytes) / 1000)
        }
        return String(format: "%.1f MB", Double(bytes) / 1_000_000)
    }
}

// MARK: - RecallSection

/// docs/PHASE4.md W2: a recall query box (+ mode + limit) running `recall` on
/// demand, ranked memories most-relevant-first, "as of last query".
@MainActor
private struct RecallSection: View {
    let model: MemoryModel

    private static let displayLimit = 100

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            controls
            Text(RecallFormat.label(model.lastRecallAt))
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textTertiary)

            if model.recallResults.isEmpty {
                Text(emptyText)
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            } else {
                let shown = Array(model.recallResults.prefix(Self.displayLimit))
                VStack(spacing: 8) {
                    ForEach(shown, id: \.id) { memory in
                        RecallResultRow(
                            memory: memory, isSelected: memory.id == model.selectedMemoryId,
                            onSelect: { Task { await model.why(memory.id) } })
                    }
                }
                if model.recallResults.count > Self.displayLimit {
                    Text("+\(model.recallResults.count - Self.displayLimit) more (showing top \(Self.displayLimit))")
                        .font(Theme.mono(10.5))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
        }
    }

    private var emptyText: String {
        if model.isRecalling {
            return model.hasRecalledOnce
                ? "searching..." : "loading the embedding model - the first recall can take a few seconds..."
        }
        return model.lastRecallAt == nil ? "enter a query and press Recall." : "no memories matched this query."
    }

    private var controls: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                TextField("recall query", text: Binding(get: { model.recallQuery }, set: { model.recallQuery = $0 }))
                    .textFieldStyle(.plain)
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.textPrimary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panelElevated))
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))
                    .onSubmit { Task { await model.recall() } }

                modePicker
                limitField
                recallButton
            }
        }
    }

    private var modePicker: some View {
        Picker("mode", selection: Binding(get: { model.recallMode }, set: { model.recallMode = $0 })) {
            ForEach(RecallMode.allCases) { mode in
                Text(mode.rawValue).tag(mode)
            }
        }
        .labelsHidden()
        .pickerStyle(.menu)
        .font(Theme.mono(11))
        .frame(width: 120)
    }

    private var limitField: some View {
        TextField(
            "limit",
            value: Binding(get: { model.recallLimit }, set: { model.recallLimit = $0 }), format: .number
        )
        .textFieldStyle(.plain)
        .font(Theme.mono(11.5))
        .monospacedDigit()
        .foregroundStyle(Theme.textPrimary)
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .frame(width: 52)
        .background(RoundedRectangle(cornerRadius: 6).fill(Theme.panelElevated))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.hairlineStrong, lineWidth: 1))
    }

    private var recallButton: some View {
        Button {
            Task { await model.recall() }
        } label: {
            HStack(spacing: 5) {
                if model.isRecalling {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 10, weight: .bold))
                }
                Text(model.isRecalling ? "Recalling..." : "Recall")
            }
            .font(Theme.mono(11, weight: .semibold))
            .foregroundStyle(Theme.iris)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.iris.opacity(0.14)))
            .overlay(Capsule().strokeBorder(Theme.iris.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(model.isRecalling)
    }

    /// One ranked hit: score/importance, content, actors/tags, timestamp.
    /// Tapping selects it for the Provenance section below (docs/PHASE4.md
    /// W2: "`why` provenance on selecting a memory id").
    private struct RecallResultRow: View {
        let memory: EngramMemoryRecord
        let isSelected: Bool
        let onSelect: () -> Void

        var body: some View {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .top, spacing: 10) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(memory.content)
                            .font(Theme.mono(12, weight: .medium))
                            .foregroundStyle(Theme.textPrimary)
                            .lineLimit(2)
                            .truncationMode(.tail)
                        Text(memory.id)
                            .font(Theme.mono(10))
                            .foregroundStyle(Theme.textTertiary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Spacer(minLength: 8)
                    scoreColumn
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(Theme.textTertiary)
                }
                if !memory.actors.isEmpty || !memory.tags.isEmpty {
                    HStack(spacing: 6) {
                        if !memory.actors.isEmpty {
                            tagPill(memory.actors.joined(separator: ", "), tone: Theme.steel)
                        }
                        ForEach(memory.tags, id: \.self) { tag in
                            tagPill(tag, tone: Theme.iris)
                        }
                    }
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(isSelected ? Theme.iris.opacity(0.10) : Theme.panelElevated)
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .strokeBorder(isSelected ? Theme.iris.opacity(0.6) : Color.clear, lineWidth: 1.5)
            )
            .contentShape(Rectangle())
            .onTapGesture(perform: onSelect)
            .help("See \(memory.id)'s provenance")
        }

        private var scoreColumn: some View {
            VStack(alignment: .trailing, spacing: 2) {
                Text(String(format: "score %.3f", memory.score))
                    .font(Theme.mono(10.5))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textSecondary)
                Text(String(format: "importance %.2f", memory.importance))
                    .font(Theme.mono(10))
                    .monospacedDigit()
                    .foregroundStyle(Theme.textTertiary)
            }
        }

        private func tagPill(_ text: String, tone: Color) -> some View {
            Text(text)
                .font(Theme.mono(9.5, weight: .semibold))
                .foregroundStyle(tone)
                .padding(.horizontal, 7)
                .padding(.vertical, 2)
                .background(Capsule().fill(tone.opacity(0.14)))
        }
    }
}

// MARK: - ProvenanceSection

/// docs/PHASE4.md W2: `why` provenance, branching on `kind` (semantic triple
/// + extraction chain vs episodic content + access metadata), an unknown id
/// shown as the honest Tool error, plus a guarded `forget` action.
@MainActor
private struct ProvenanceSection: View {
    let model: MemoryModel

    @State private var confirmingForget = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let memoryId = model.selectedMemoryId {
                if model.isLoadingProvenance {
                    Text("loading provenance for \(memoryId)...")
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textTertiary)
                } else if let provenance = model.provenance {
                    ProvenanceCard(provenance: provenance)
                    forgetRow(memoryId)
                } else if let provenanceError = model.provenanceError {
                    ErrorBannerView(message: provenanceError)
                    forgetRow(memoryId)
                }
            } else {
                Text("select a memory from Recall above to see its provenance.")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)
                    .padding(.vertical, 4)
            }
        }
    }

    /// docs/PHASE4.md W2: "optional `forget` admin action, guarded as
    /// irreversible" - a plain two-step confirm (no hardware challenge is
    /// specified for this action, unlike Money/Policy's break-glass
    /// mutations), mirroring `ConfirmButton`'s own generic "privileged
    /// action, always confirm first" role rather than its Touch-ID-labeled
    /// `PolicyView` call sites specifically.
    private func forgetRow(_ memoryId: String) -> some View {
        HStack {
            Spacer(minLength: 0)
            ConfirmButton(
                label: "Forget this memory", confirmLabel: "Confirm forget - this cannot be undone", tone: Theme.ember,
                onConfirm: { await model.forget(memoryId) }
            )
        }
    }
}

/// Renders either provenance shape - exhaustive, so a future third case
/// fails to compile here rather than rendering nothing.
@MainActor
private struct ProvenanceCard: View {
    let provenance: EngramProvenanceRecord

    var body: some View {
        switch provenance {
        case .semantic(
            let id, let subject, let predicate, let object, let confidence, let validFrom, let validTo, let recordedAt,
            let extractedFrom, let extractedByReflectionRun, let extractionModel):
            card(kind: "semantic fact", tone: Theme.violet) {
                Text("\(subject) \u{2192} \(predicate) \u{2192} \(object)")
                    .font(Theme.mono(13, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                fieldRow("confidence", String(format: "%.2f", confidence))
                fieldRow("valid from", MoneyFormat.timestamp(validFrom))
                fieldRow("valid to", validTo.map(MoneyFormat.timestamp) ?? "still valid")
                fieldRow("recorded at", MoneyFormat.timestamp(recordedAt))
                Divider().overlay(Theme.hairline)
                Text("EXTRACTION CHAIN")
                    .font(Theme.mono(9.5, weight: .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Theme.textTertiary)
                fieldRow("extracted from", extractedFrom ?? "n/a")
                fieldRow("reflection run", extractedByReflectionRun ?? "n/a")
                fieldRow("model", extractionModel ?? "n/a")
                fieldRow("id", id)
            }

        case .episodic(
            let id, let content, let timestamp, let actors, let tags, let salience, let emotionalValence,
            let importanceScore, let summaryOf, let agentId, let accessCount, let lastAccessed, let note):
            card(kind: "episodic observation", tone: Theme.iris) {
                Text(content)
                    .font(Theme.mono(13, weight: .medium))
                    .foregroundStyle(Theme.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                if !actors.isEmpty {
                    fieldRow("actors", actors.joined(separator: ", "))
                }
                if !tags.isEmpty {
                    fieldRow("tags", tags.joined(separator: ", "))
                }
                fieldRow("timestamp", MoneyFormat.timestamp(timestamp))
                Divider().overlay(Theme.hairline)
                Text("ENCODING + ACCESS")
                    .font(Theme.mono(9.5, weight: .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Theme.textTertiary)
                fieldRow("salience", salience.map { String(format: "%.2f", $0) } ?? "n/a")
                fieldRow("emotional valence", emotionalValence.map { String(format: "%.2f", $0) } ?? "n/a")
                fieldRow("importance score", importanceScore.map { String(format: "%.2f", $0) } ?? "n/a")
                fieldRow("summarizes", summaryOf.isEmpty ? "n/a" : summaryOf.joined(separator: ", "))
                fieldRow("agent", agentId ?? "n/a")
                fieldRow("access count", String(accessCount))
                fieldRow("last accessed", lastAccessed.map(MoneyFormat.timestamp) ?? "never")
                fieldRow("id", id)
                Text(note)
                    .font(Theme.mono(10.5))
                    .foregroundStyle(Theme.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    @ViewBuilder
    private func card<Content: View>(kind: String, tone: Color, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(kind.uppercased())
                .font(Theme.mono(9.5, weight: .semibold))
                .tracking(0.8)
                .foregroundStyle(tone)
            content()
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .fill(Theme.panelElevated)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                .strokeBorder(tone.opacity(0.3), lineWidth: 1)
        )
    }

    private func fieldRow(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Text(label)
                .font(Theme.mono(10.5))
                .foregroundStyle(Theme.textTertiary)
                .frame(width: 120, alignment: .leading)
            Text(value)
                .font(Theme.mono(11))
                .foregroundStyle(Theme.textSecondary)
                .textSelection(.enabled)
                .lineLimit(2)
                .truncationMode(.middle)
        }
    }
}

// MARK: - MemoryTimelineSection

/// docs/PHASE4.md W2: a timeline from the live `engram.*` bus events - see
/// `MemoryView`'s own doc for why this is a filter, not a new read.
@MainActor
private struct MemoryTimelineSection: View {
    let events: [UiEvent]

    private static let displayLimit = 60

    var body: some View {
        if events.isEmpty {
            Text("no engram bus activity yet.")
                .font(Theme.mono(12))
                .foregroundStyle(Theme.textTertiary)
                .padding(.vertical, 12)
        } else {
            let shown = Array(events.prefix(Self.displayLimit))
            VStack(spacing: 0) {
                ForEach(Array(shown.enumerated()), id: \.element.rowKey) { index, event in
                    TimelineRow(event: event)
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
        }
    }

    private struct TimelineRow: View {
        let event: UiEvent

        @State private var expanded = false

        var body: some View {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 12) {
                    SeverityPill(severity: event.severity ?? "info")

                    Text(event.eventType)
                        .font(Theme.mono(11.5, weight: .medium))
                        .foregroundStyle(Theme.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .frame(width: 170, alignment: .leading)

                    Text(event.agentId)
                        .font(Theme.mono(11))
                        .foregroundStyle(Theme.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                        .frame(maxWidth: .infinity, alignment: .leading)

                    Text(MoneyFormat.timestamp(event.ts))
                        .font(Theme.mono(11))
                        .monospacedDigit()
                        .foregroundStyle(Theme.textTertiary)
                        .frame(width: 118, alignment: .trailing)

                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(Theme.textTertiary)
                }
                if expanded {
                    TimelineRawJsonView(raw: event.raw)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .contentShape(Rectangle())
            .onTapGesture {
                withAnimation(.easeInOut(duration: 0.15)) { expanded.toggle() }
            }
        }
    }
}
