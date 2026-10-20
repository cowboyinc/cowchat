import AppKit
import CryptoKit
import Foundation
import UserNotifications

struct SocialNotificationSnapshot {
    let session: SocialSession
    let requests: [SocialFriendRequest]
    let invitations: [SocialRoomInvitation]

    var accountScope: String {
        SHA256.hash(data: Data("\(session.endpoint.absoluteString)\n\(session.account.id)".utf8))
            .map { String(format: "%02x", $0) }.joined()
    }

    var deliveryScope: String { accountScope + "." + session.scopeId }
}

struct SocialNotificationEvent: Equatable {
    enum Kind: String, CaseIterable {
        case friendRequest
        case friendAccepted
        case roomInvitation

        var destination: SocialSection { self == .roomInvitation ? .invitations : .friends }
        var title: String {
            switch self {
            case .friendRequest: return "New friend request"
            case .friendAccepted: return "Friend request accepted"
            case .roomInvitation: return "Room invitation"
            }
        }
    }

    let id: String
    let kind: Kind
    let body: String
}

struct SocialNotificationLedger: Codable {
    private var seen: Set<String> = []
    private var initialized = false

    mutating func consume(_ snapshot: SocialNotificationSnapshot) -> [SocialNotificationEvent] {
        let accountID = snapshot.session.account.id
        var candidates: [SocialNotificationEvent] = snapshot.requests.compactMap { request in
            if request.to.id == accountID, request.from.id != accountID, request.status == "pending" {
                return SocialNotificationEvent(id: "request.\(request.id)", kind: .friendRequest,
                    body: "@\(request.from.handle.prefix(80)) wants to be your friend.")
            }
            if request.from.id == accountID, request.to.id != accountID, request.status == "accepted" {
                return SocialNotificationEvent(id: "accepted.\(request.id)", kind: .friendAccepted,
                    body: "@\(request.to.handle.prefix(80)) accepted your friend request.")
            }
            return nil
        }
        candidates += snapshot.invitations.compactMap { invite in
            guard invite.to.id == accountID, invite.from.id != accountID, invite.status == "pending" else { return nil }
            return SocialNotificationEvent(id: "invitation.\(invite.id)", kind: .roomInvitation,
                body: "@\(invite.from.handle.prefix(80)) invited you to \(invite.roomName.prefix(120)).")
        }
        var events: [SocialNotificationEvent] = []
        for event in candidates where seen.insert(event.id).inserted {
            if initialized { events.append(event) }
        }
        initialized = true
        return events
    }
}

@MainActor
final class SocialNotifications: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    @Published private(set) var authorizationStatus: UNAuthorizationStatus = .notDetermined
    @Published private(set) var destination: SocialSection?
    @Published private(set) var errorMessage: String?
    @Published var enabled: Bool {
        didSet {
            defaults.set(enabled, forKey: Self.enabledKey)
            if enabled { Task { await requestPermission() } }
            else { clearNotifications() }
        }
    }
    @Published var friendRequests: Bool { didSet { defaults.set(friendRequests, forKey: Self.requestsKey) } }
    @Published var acceptedRequests: Bool { didSet { defaults.set(acceptedRequests, forKey: Self.acceptedKey) } }
    @Published var roomInvitations: Bool { didSet { defaults.set(roomInvitations, forKey: Self.invitationsKey) } }
    private let defaults: UserDefaults
    private let center: UNUserNotificationCenter?
    private var activeScope: String?
    private var sessionConfigured = false
    private var pendingOpen: (String, SocialSection)?
    private static let enabledKey = "CowchatMac.notifications.enabled"
    private static let requestsKey = "CowchatMac.notifications.friendRequests"
    private static let acceptedKey = "CowchatMac.notifications.acceptedRequests"
    private static let invitationsKey = "CowchatMac.notifications.roomInvitations"

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        enabled = defaults.object(forKey: Self.enabledKey) as? Bool ?? true
        friendRequests = defaults.object(forKey: Self.requestsKey) as? Bool ?? true
        acceptedRequests = defaults.object(forKey: Self.acceptedKey) as? Bool ?? true
        roomInvitations = defaults.object(forKey: Self.invitationsKey) as? Bool ?? true
        center = Bundle.main.bundleURL.pathExtension == "app" ? UNUserNotificationCenter.current() : nil
        super.init()
        center?.delegate = self
    }

    func setSession(_ session: SocialSession?) {
        let scope = session.map { SocialNotificationSnapshot(session: $0, requests: [], invitations: []).deliveryScope }
        if sessionConfigured, activeScope != scope {
            destination = nil
            pendingOpen = nil
        }
        if scope == nil || (activeScope != nil && activeScope != scope) { clearNotifications() }
        activeScope = scope
        sessionConfigured = true
        if let (scope, section) = pendingOpen {
            pendingOpen = nil
            open(scope: scope, section: section)
        }
        if session != nil, enabled { Task { await requestPermission() } }
    }

    func receive(_ snapshot: SocialNotificationSnapshot) {
        guard activeScope == snapshot.deliveryScope else { return }
        let key = "CowchatMac.notifications.seen.v1." + snapshot.accountScope
        var ledger = defaults.data(forKey: key).flatMap { try? JSONDecoder().decode(SocialNotificationLedger.self, from: $0) } ?? SocialNotificationLedger()
        let events = ledger.consume(snapshot)
        if let data = try? JSONEncoder().encode(ledger) { defaults.set(data, forKey: key) }
        guard enabled, let center else { return }
        let selected = events.filter { event in
            switch event.kind {
            case .friendRequest: return friendRequests
            case .friendAccepted: return acceptedRequests
            case .roomInvitation: return roomInvitations
            }
        }
        let scope = snapshot.deliveryScope
        Task {
            let status = await center.notificationSettings().authorizationStatus
            authorizationStatus = status
            guard enabled, activeScope == scope, status == .authorized || status == .provisional else { return }
            for event in selected {
                guard enabled, activeScope == scope else { return }
                let content = UNMutableNotificationContent()
                content.title = event.kind.title
                content.body = event.body
                content.sound = .default
                content.threadIdentifier = "cowchat-social.\(scope)"
                content.userInfo = ["scope": scope, "section": event.kind.destination.rawValue]
                let id = "\(scope).\(event.id)"
                do {
                    try await center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
                    if !enabled || activeScope != scope {
                        center.removePendingNotificationRequests(withIdentifiers: [id])
                        center.removeDeliveredNotifications(withIdentifiers: [id])
                    }
                } catch { errorMessage = "The notification could not be delivered. \(error.localizedDescription)" }
            }
        }
    }

    func refreshAuthorization() async {
        guard let center else { return }
        authorizationStatus = await center.notificationSettings().authorizationStatus
    }

    func requestPermission() async {
        guard let center, enabled else { return }
        errorMessage = nil
        let status = await center.notificationSettings().authorizationStatus
        if status == .notDetermined {
            do { _ = try await center.requestAuthorization(options: [.alert, .sound, .badge]) }
            catch { errorMessage = "Notification permission could not be requested. \(error.localizedDescription)" }
        }
        await refreshAuthorization()
    }

    func sendTestNotification() async {
        guard enabled, let center else { return }
        await requestPermission()
        guard enabled, authorizationStatus == .authorized || authorizationStatus == .provisional else { return }
        let content = UNMutableNotificationContent()
        content.title = "Cowchat notifications"
        content.body = "Friend requests and room invitations can notify you here."
        content.sound = .default
        content.userInfo = ["test": true]
        do { try await center.add(UNNotificationRequest(identifier: "cowchat-notification-test", content: content, trigger: nil)) }
        catch { errorMessage = "The test notification could not be delivered. \(error.localizedDescription)" }
    }

    func consumeDestination(_ section: SocialSection) -> Bool {
        guard destination == section else { return false }
        destination = nil
        return true
    }

    private func clearNotifications() {
        center?.removeAllPendingNotificationRequests()
        center?.removeAllDeliveredNotifications()
    }

    private func open(scope: String, section: SocialSection) {
        guard route(scope: scope, section: section) else { return }
        NSApplication.shared.activate(ignoringOtherApps: true)
        NSApplication.shared.windows.first(where: { $0.canBecomeMain })?.makeKeyAndOrderFront(nil)
    }

    @discardableResult
    func route(scope: String, section: SocialSection) -> Bool {
        guard sessionConfigured else { pendingOpen = (scope, section); return false }
        guard scope == activeScope else { return false }
        destination = section
        return true
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        Task { @MainActor in
            let scope = notification.request.content.userInfo["scope"] as? String
            let isTest = notification.request.content.userInfo["test"] as? Bool == true
            completionHandler(enabled && (isTest || (scope != nil && scope == activeScope)) ? [.banner, .list, .sound] : [])
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void) {
        let info = response.notification.request.content.userInfo
        let scope = info["scope"] as? String
        let section = (info["section"] as? String).flatMap(SocialSection.init(rawValue:))
        Task { @MainActor in
            if response.actionIdentifier == UNNotificationDefaultActionIdentifier, let scope, let section {
                open(scope: scope, section: section)
            }
            completionHandler()
        }
    }
}
