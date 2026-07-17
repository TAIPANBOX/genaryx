import GenaryxCoreFFI
import SwiftUI

/// The Overview panel: an environment chip, four headline tiles (total
/// spent, active runs, open incidents, total saved), and the shared
/// error/upsell banners. Fed by `CloudModel`, at parity with the Tauri
/// shell's `OverviewView.tsx`.
@MainActor
struct OverviewView: View {
    let model: CloudModel

    /// Feels-alive refresh cadence, matching the Tauri shell's
    /// `REFRESH_INTERVAL_MS` - not a live SSE push (out of scope for this
    /// wave), just a periodic re-fetch.
    private static let refreshInterval: Duration = .seconds(20)

    var body: some View {
        Group {
            if model.connection.isReady {
                content
            } else {
                MoneyEmptyStateView(connection: model.connection)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task(id: model.connection.isReady) {
            guard model.connection.isReady else { return }
            while !Task.isCancelled {
                await model.refreshOverview()
                try? await Task.sleep(for: Self.refreshInterval)
            }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                environmentChip

                if let planRequired = model.planRequired {
                    UpsellBannerView(notice: planRequired)
                }
                if let bannerMessage = model.bannerMessage {
                    ErrorBannerView(message: bannerMessage)
                }

                if let overview = model.overview {
                    tiles(for: overview)
                } else {
                    Text("loading overview...")
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textTertiary)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private var environmentChip: some View {
        // Defensive-only: `body` already gates `content` (and therefore
        // this chip) on `model.connection.isReady`, so the non-`.ready`
        // branch is never actually hit in practice - same convention the
        // Tauri shell's `MoneyEmptyState.tsx` documents for its own
        // unreachable `"ready"` branch.
        if case .ready(let source, let cloudUrl, _) = model.connection {
            HStack(spacing: 6) {
                Circle().fill(Theme.mint).frame(width: 6, height: 6)
                Text("\(sourceLabel(source)) \u{00B7} \(cloudUrl)")
                    .font(Theme.mono(11, weight: .medium))
                    .foregroundStyle(Theme.textSecondary)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Theme.panelElevated))
            .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
        }
    }

    private func sourceLabel(_ source: EnvSource) -> String {
        switch source {
        case .taipan(let name):
            "taipan up \u{00B7} \(name)"
        case .envFallback:
            "env fallback"
        }
    }

    private func tiles(for overview: Overview) -> some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 4), spacing: 12) {
            StatTile(
                label: "total spent",
                value: MoneyFormat.usd(overview.totalSpentUsd),
                sub: "\(overview.totalCalls) calls"
            )
            StatTile(
                label: "active runs",
                value: String(overview.activeRuns),
                sub: "\(overview.killedRuns) killed of \(overview.totalRuns)"
            )
            StatTile(
                label: "open incidents",
                value: String(overview.openIncidents),
                sub: "\(overview.totalIncidents) total",
                tone: overview.openIncidents > 0 ? Theme.coral : nil
            )
            StatTile(
                label: "total saved",
                value: MoneyFormat.usd(overview.totalSavedUsd),
                tone: Theme.mint
            )
        }
    }
}
