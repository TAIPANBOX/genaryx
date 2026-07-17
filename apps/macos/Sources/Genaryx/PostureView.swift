import GenaryxCoreFFI
import SwiftUI

/// The Posture panel: a read-only list of stack-sanity findings, each a
/// {severity, title, why it matters, how to fix} row (docs/PHASE2.md Wave 3,
/// "Posture-lite"). Fed by `PostureModel.findings(cloud:policy:fleet:)`,
/// itself a pure function over the three models' already-live state - no
/// new read, no new FFI call (PHASE2.md: "computed from already-observable
/// signals"). PHASE2.md asks for the identical 4 v0 zonds in both shells;
/// the Tauri track (`apps/desktop`) builds its own parallel panel from the
/// same spec and is out of scope here (SwiftUI/Tauri are two independent
/// tracks over one shared data contract, not a shared implementation).
///
/// Unlike Overview/Money/Policy, this view never gates its content behind a
/// `connection.isReady` empty state: Posture's whole point is to describe
/// the CURRENT state of the stack, including a not-yet-connected one (the
/// devkey and governance zonds simply do not fire until their respective
/// connection is `.ready` - see `PostureModel`'s own doc comments - while
/// the schema-mix and bus-stale zonds read only `FleetModel`, which is
/// independent of Cloud/Wardryx pairing entirely).
@MainActor
struct PostureView: View {
    let cloudModel: CloudModel
    let policyModel: PolicyModel
    let fleetModel: FleetModel

    /// Cheap re-evaluation cadence so "bus stale" flips live while this tab
    /// sits open with no new events arriving at all - the one zond here
    /// that time alone can change, mirroring `TTLCountdownTile`'s own
    /// `TimelineView` use in `PolicyComponents.swift` for the identical
    /// reason (a value that must keep moving even with no state mutation to
    /// trigger a re-render).
    private static let tick: TimeInterval = 5
    /// Matches every other panel's own periodic-refresh cadence
    /// (`OverviewView`/`MoneyView`/`PolicyView`'s `refreshInterval`), so
    /// `policy.policies` (the governance-fail-open zond's input) stays
    /// current even for an operator who only ever has the Posture tab open
    /// and never visits the Policy panel itself.
    private static let refreshInterval: Duration = .seconds(20)

    var body: some View {
        TimelineView(.periodic(from: .now, by: Self.tick)) { context in
            content(at: context.date)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .task {
            while !Task.isCancelled {
                await policyModel.refresh()
                try? await Task.sleep(for: Self.refreshInterval)
            }
        }
    }

    private func content(at date: Date) -> some View {
        let findings = PostureModel.findings(cloud: cloudModel, policy: policyModel, fleet: fleetModel, now: date)
        return ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header(findingCount: findings.count)

                if findings.isEmpty {
                    Text("no posture findings - the stack looks sane.")
                        .font(Theme.mono(12))
                        .foregroundStyle(Theme.textTertiary)
                        .padding(.vertical, 12)
                } else {
                    VStack(spacing: 10) {
                        ForEach(findings) { finding in
                            PostureFindingRow(finding: finding)
                        }
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func header(findingCount: Int) -> some View {
        HStack(spacing: 8) {
            Text("STACK SANITY")
                .font(Theme.mono(11, weight: .semibold))
                .tracking(1.4)
                .foregroundStyle(Theme.textTertiary)
            Spacer()
            Text("\(findingCount) finding\(findingCount == 1 ? "" : "s")")
                .font(Theme.mono(11, weight: .semibold))
                .foregroundStyle(findingCount > 0 ? Theme.amber : Theme.mint)
        }
    }
}

/// One finding: a severity badge (`SeverityPill`, reused directly from
/// `MoneyComponents.swift` - the exact same atom `MoneyView`'s
/// `IncidentsList` and `PolicyView`'s Decision Stream badge already draw
/// from), title, why-it-matters, and a concrete how-to-fix line.
@MainActor
private struct PostureFindingRow: View {
    let finding: PostureFinding

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                SeverityPill(severity: finding.severity)
                Text(finding.title)
                    .font(Theme.mono(12.5, weight: .semibold))
                    .foregroundStyle(Theme.textPrimary)
                Spacer(minLength: 0)
            }

            Text(finding.whyItMatters)
                .font(Theme.mono(11.5))
                .foregroundStyle(Theme.textSecondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack(alignment: .top, spacing: 6) {
                Text("FIX")
                    .font(Theme.mono(9.5, weight: .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Theme.textTertiary)
                Text(finding.howToFix)
                    .font(Theme.mono(11.5))
                    .foregroundStyle(Theme.amber)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .fill(Theme.panelElevated)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                .strokeBorder(Theme.severityColor(finding.severity).opacity(0.35), lineWidth: 1)
        )
    }
}
