import Foundation
import GenaryxCoreFFI
import SwiftUI

/// Shared building blocks for the Remote view: the "not ready yet" empty
/// state plus small formatters (tunnel status badge, Hetzner server status
/// color). Mirrors `DrillsComponents.swift`'s own role and distribution:
/// view-specific sections (Hetzner table, the remote-environment config
/// form, the tunnel controls, SSH ops) live in `RemoteView.swift` rather than
/// here.

// MARK: - RemoteEmptyStateView

/// Shared "not ready yet" rendering for the Remote view - see
/// `RemoteConnection`'s own doc for why this only ever fires on a genuine
/// local resource problem, never a missing Hetzner token / WireGuard binary
/// / SSH target (those stay per-field, honest states inside the ready
/// content itself). Mirrors `DrillsEmptyStateView`/`CryptoEmptyStateView`
/// field-for-field.
@MainActor
struct RemoteEmptyStateView: View {
    let connection: RemoteConnection

    var body: some View {
        centered {
            switch connection {
            case .connecting:
                Text("starting the remote transport...")
                    .font(Theme.mono(12))
                    .foregroundStyle(Theme.textTertiary)

            case .failed(let reason):
                card {
                    Text("Could not start the Remote panel")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.coral)
                    Text(reason)
                        .font(Theme.mono(11.5))
                        .foregroundStyle(Theme.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }

            case .ready:
                EmptyView()
            }
        }
    }

    @ViewBuilder
    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8, content: content)
            .padding(20)
            .frame(maxWidth: 460, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                    .fill(Theme.panelElevated)
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Radius.card, style: .continuous)
                    .strokeBorder(Theme.hairline, lineWidth: 1)
            )
    }

    @ViewBuilder
    private func centered<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack {
            Spacer(minLength: 0)
            HStack {
                Spacer(minLength: 0)
                content()
                Spacer(minLength: 0)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(24)
    }
}

// MARK: - tunnel status badge

/// docs/PHASE4.md W4: "an honest status badge (Connecting / CONNECTED
/// (handshake Ns ago) / FAILED: <reason>)". The "Connecting" state is NOT one
/// of `TunnelStatus`'s own cases (see that type's own doc: bring-up is
/// synchronous, so Rust only ever returns a settled verdict) - `RemoteView`
/// renders it separately, from `model.isConnectingTunnel`, overriding
/// whatever this formatter would otherwise show for the LAST settled status
/// while a fresh attempt is in flight.
enum TunnelStatusFormat {
    static func label(_ status: TunnelStatus) -> String {
        switch status {
        case .disconnected:
            return "DISCONNECTED"
        case .connected(_, let handshakeSecsAgo):
            if let handshakeSecsAgo {
                return "CONNECTED (handshake \(handshakeSecsAgo)s ago)"
            }
            return "CONNECTED (no handshake yet)"
        case .failed(let reason):
            return "FAILED: \(reason)"
        }
    }

    static func color(_ status: TunnelStatus) -> Color {
        switch status {
        case .disconnected:
            return Theme.textTertiary
        case .connected(_, let handshakeSecsAgo):
            return handshakeSecsAgo != nil ? Theme.mint : Theme.amber
        case .failed:
            return Theme.coral
        }
    }
}

// MARK: - Hetzner server status color

/// Hetzner's own `server.status` vocabulary (`running` | `off` | `starting`
/// | `stopping` | `initializing` | `migrating` | `rebuilding` | `deleting` |
/// `unknown`) colored for the inventory table - `running` reads as healthy,
/// `off`/`deleting` as inactive, everything else (a transitional state) as
/// in-progress. Never a status this console cannot also see honestly: it
/// only colors whatever string Hetzner itself returned, never substitutes
/// one.
enum HetznerStatusFormat {
    static func color(_ status: String) -> Color {
        switch status.lowercased() {
        case "running":
            return Theme.mint
        case "off", "deleting":
            return Theme.textTertiary
        default:
            return Theme.amber
        }
    }
}

// MARK: - "as of last <fetch>" formatting

/// A `Date` this model stamped at fetch time (Hetzner's own inventory rows
/// carry no "scanned at" field to reuse, unlike e.g. `DrillReportRecord`'s
/// own `generatedAt` - mirrors `MemoryStatsFormat`/`RecallFormat`'s own
/// `Date -> ISO8601 -> MoneyFormat.timestamp` conversion). One shared
/// formatter (rather than a separate enum per call site, unlike
/// `MemoryStatsFormat`/`RecallFormat` staying deliberately distinct for two
/// semantically different actions) because every call site here is the exact
/// same kind of thing: "when did this read last actually happen" - a Hetzner
/// scan, an SSH descriptor read.
enum RemoteAsOfFormat {
    static func label(_ date: Date?, prefix: String, emptyText: String) -> String {
        guard let date else { return emptyText }
        let iso = ISO8601DateFormatter().string(from: date)
        return "\(prefix) \(MoneyFormat.timestamp(iso))"
    }
}
