import Foundation
import GenaryxCoreFFI
import Observation
import UserNotifications

/// One deep-link target: the notification response handler (a Review /
/// Approve / Deny tap, or a plain tap on the banner) hands this back to
/// `GenaryxApp`, which switches to the Policy tab and focuses this one
/// approval in the Approvals Inbox - docs/PHASE2.md Wave 3: "DEEP-LINKS
/// into the Approvals Inbox focused on that `approval_id`, where the
/// operator completes the existing Touch-ID / confirm-gated grant/deny".
/// Carries ONLY a routing target, never a verdict: there is no field here a
/// notification response could use to drive a decision directly, so the
/// hardware gate in `PolicyModel.decide` cannot be bypassed by this path
/// even by a future bug, only by construction.
struct ApprovalFocusRequest: Equatable {
    let approvalId: String
}

/// One mute key - PHASE2.md Wave 3: "Mute: per agent / per run / per
/// environment (an in-memory mute set is fine for v0)".
private struct ApprovalMuteKey: Hashable {
    let agentId: String
    let runId: String
    let environment: String
}

/// Watches the live bus for `approval_requested` events (the SAME feed
/// `PolicyView`'s Decision Stream already filters to `source == "wardryx"`
/// - see `FleetModel.onNewEvent`, wired once from `GenaryxApp`) and raises a
/// native, de-duped, mutable macOS notification for each one - PHASE2.md
/// Wave 3 "actionable notifications".
///
/// Deliberately holds NO reference to `PolicyModel` (or anything that could
/// reach `WardryxHandle.decideApproval`): the only thing a notification
/// response can do here is set `focusRequest`, a plain routing value
/// `GenaryxApp` reads to switch tabs and hand an id down to the Approvals
/// Inbox. The actual grant/deny call only ever happens through
/// `PolicyModel.decide`, which always challenges `LocalAuthentication`
/// first (see `PolicyModel.swift`) - PHASE2.md: "an Approve/Deny action
/// must NOT silently execute the privileged mutation... never bypassed".
/// This class has no way to call that method even by accident.
@MainActor
@Observable
final class ApprovalNotificationModel: NSObject {
    /// Set by a notification response (any of Review/Approve/Deny, or a
    /// plain tap - see `userNotificationCenter(_:didReceive:...)`'s own doc
    /// for why all four route identically); cleared by `GenaryxApp` once it
    /// has switched tabs and handed the id down.
    private(set) var focusRequest: ApprovalFocusRequest?

    /// `data.approval_id`s already raised - PHASE2.md: "at most one
    /// notification per approval_id (never re-raised on a list refresh)".
    private var notifiedApprovalIds: Set<String> = []

    /// PHASE2.md: "a muted key raises nothing".
    private var mutedKeys: Set<ApprovalMuteKey> = []

    private static let categoryId = "APPROVAL_REQUEST"
    private static let reviewActionId = "REVIEW_APPROVAL"
    private static let approveActionId = "APPROVE_APPROVAL"
    private static let denyActionId = "DENY_APPROVAL"

    override init() {
        super.init()
        configure()
    }

    // MARK: - launch-time setup

    /// `UNUserNotificationCenter.current()` unconditionally raises an
    /// uncaught `NSInternalInconsistencyException`
    /// ("bundleProxyForCurrentProcess is nil") on any process without a
    /// real, `Info.plist`-backed bundle identifier - confirmed directly
    /// against this toolchain: a minimal SwiftPM `executableTarget`, built
    /// and run exactly like this one (no Xcode `.app` wrapper), crashes the
    /// instant `.current()` is touched. That is exactly what `swift
    /// build`/`swift run` produce for `Genaryx` today (see `Package.swift`'s
    /// own doc comment; no Info.plist generation step exists anywhere in
    /// this repo yet). Every notification call below funnels through
    /// `center`, so the whole feature safely no-ops on today's bare
    /// executable and activates automatically once Genaryx ships inside a
    /// signed `.app` bundle (`CFBundleIdentifier` present) - never a crash
    /// either way, the same fail-closed rule this shell already holds
    /// itself to elsewhere (`FleetModel`/`CloudModel`/`PolicyModel`: "still
    /// launches and renders... instead of crashing").
    private static var isBundled: Bool {
        Bundle.main.bundleIdentifier != nil
    }

    private var center: UNUserNotificationCenter? {
        guard Self.isBundled else { return nil }
        return UNUserNotificationCenter.current()
    }

    /// Requests notification authorization once and registers the
    /// Review/Approve/Deny category - PHASE2.md: "request authorization
    /// once on launch... actions Review / Approve / Deny". Called once from
    /// `init()`, matching `FleetModel`/`CloudModel`/`PolicyModel`'s own
    /// "do setup in init" convention rather than a separate app-level
    /// `.task`.
    private func configure() {
        guard let center else { return }
        center.delegate = self

        let review = UNNotificationAction(identifier: Self.reviewActionId, title: "Review", options: [.foreground])
        let approve = UNNotificationAction(
            identifier: Self.approveActionId, title: "Approve", options: [.foreground])
        let deny = UNNotificationAction(
            identifier: Self.denyActionId, title: "Deny", options: [.foreground, .destructive])
        let category = UNNotificationCategory(
            identifier: Self.categoryId,
            actions: [review, approve, deny],
            intentIdentifiers: [],
            options: []
        )
        center.setNotificationCategories([category])

        center.requestAuthorization(options: [.alert, .sound]) { _, _ in
            // Fail-closed, never fatal: a denied/unavailable authorization
            // just means `raise` below silently posts nothing later - this
            // class never blocks on or retries the result.
        }
    }

    // MARK: - mute (PHASE2.md: "per agent / per run / per environment")

    func isMuted(agentId: String, runId: String, environment: String) -> Bool {
        mutedKeys.contains(ApprovalMuteKey(agentId: agentId, runId: runId, environment: environment))
    }

    func mute(agentId: String, runId: String, environment: String) {
        mutedKeys.insert(ApprovalMuteKey(agentId: agentId, runId: runId, environment: environment))
    }

    func unmute(agentId: String, runId: String, environment: String) {
        mutedKeys.remove(ApprovalMuteKey(agentId: agentId, runId: runId, environment: environment))
    }

    /// The resolved Wardryx endpoint, standing in for "environment" in the
    /// mute key above - the same value `PolicyView`'s own environment chip
    /// shows the operator (`WardryxConnection.ready`'s `wardryxUrl`).
    static func environmentLabel(for connection: WardryxConnection) -> String {
        if case .ready(_, let wardryxUrl, _) = connection {
            return wardryxUrl
        }
        return "disconnected"
    }

    // MARK: - bus watch

    /// Called once per newly-live bus event (see `FleetModel.onNewEvent`,
    /// wired from `GenaryxApp`) - the exact same feed `PolicyView`'s
    /// Decision Stream filters to `source == "wardryx"`, never a separate
    /// read of its own.
    func handle(event: UiEvent, environment: String) {
        guard event.source.lowercased() == "wardryx", event.eventType == "approval_requested" else { return }

        let fields = event.wardryxFields
        guard let approvalId = fields.approvalId, !approvalId.isEmpty else { return }
        guard !notifiedApprovalIds.contains(approvalId) else { return }
        notifiedApprovalIds.insert(approvalId)

        guard !isMuted(agentId: event.agentId, runId: event.runId ?? "", environment: environment) else { return }
        raise(approvalId: approvalId, agentId: event.agentId, reason: fields.reason)
    }

    private func raise(approvalId: String, agentId: String, reason: String?) {
        guard let center else { return }
        // PHASE2.md's own illustrated shape: "Approval needed -
        // `<agent_id>` (`data.reason`)" - title carries the fixed lead,
        // body carries the two variable parts.
        let content = UNMutableNotificationContent()
        content.title = "Approval needed"
        content.body = reason.map { "\(agentId) (\($0))" } ?? agentId
        content.categoryIdentifier = Self.categoryId
        content.sound = .default

        // The approval id doubles as the notification identifier: besides
        // being all `didReceive` needs to build the focus request below,
        // re-`add`ing the same identifier replaces rather than duplicates
        // any still-pending banner for that id - a second, OS-level layer
        // under `notifiedApprovalIds`'s own de-dupe above.
        let request = UNNotificationRequest(identifier: approvalId, content: content, trigger: nil)
        center.add(request)
    }

    // MARK: - focus request

    func clearFocusRequest() {
        focusRequest = nil
    }
}

extension ApprovalNotificationModel: UNUserNotificationCenterDelegate {
    /// Show the banner even while Genaryx is the foreground app - the
    /// system default suppresses foreground banners, which would defeat
    /// "actionable notifications" for the common case of the operator
    /// already having the console open. Fires off the main actor (the
    /// delegate callback is never guaranteed to land there), touches no
    /// model state, so no actor hop is needed here at all.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound, .list])
    }

    /// Fires for a Review/Approve/Deny tap AND a plain tap on the banner
    /// itself (`UNNotificationDefaultActionIdentifier`) - every path routes
    /// identically: focus this approval in the Approvals Inbox, never a
    /// direct decide call (see the class doc's "no reference to
    /// PolicyModel"). PHASE2.md's own security rule is symmetric across all
    /// three actions ("an Approve/Deny action must NOT silently execute"),
    /// so there is deliberately no `switch response.actionIdentifier`
    /// picking a different, more-privileged path for any one of them.
    /// Fires off the main actor, so the state mutation hops explicitly -
    /// the same pattern `FleetModel.swift`'s `FleetEventListener.onEvent`
    /// already uses for its own off-main-actor callback.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let approvalId = response.notification.request.identifier
        Task { @MainActor [weak self] in
            self?.focusRequest = ApprovalFocusRequest(approvalId: approvalId)
        }
        completionHandler()
    }
}
