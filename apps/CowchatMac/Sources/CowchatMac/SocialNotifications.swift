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

/// Where clicking a social notification opens: the Friends page for a friend request or an accepted one, and the Inbox,
/// which holds the Join sheet, for a room invitation. The raw values are the ones notifications already delivered carry.
enum SocialDestination: String {
    case friends = "Friends"
    case inbox = "Invitations"

    /// The page a notification's click shows: Friends for a friend request, the Inbox for a room invitation.
    var page: WorkspaceStore.Page {
        switch self {
        case .friends: return .friends
        case .inbox: return .inbox
        }
    }
}

struct SocialNotificationEvent: Equatable {
    enum Kind: String, CaseIterable {
        case friendRequest
        case friendAccepted
        case roomInvitation

        var destination: SocialDestination { self == .roomInvitation ? .inbox : .friends }
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

/// The Inbox's messages already seen on this Mac for an account, so each new one notifies once. The first poll only
/// records what is there, as `SocialNotificationLedger` does.
struct InboxNotificationLedger: Codable {
    private var seen: Set<String> = []
    private var initialized = false

    /// The IDs not seen before, none on the first poll.
    mutating func consume(_ ids: [String]) -> [String] {
        let fresh = ids.filter { seen.insert($0).inserted }
        defer { initialized = true }
        return initialized ? fresh : []
    }
}

@MainActor
final class SocialNotifications: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    @Published private(set) var authorizationStatus: UNAuthorizationStatus = .notDetermined
    @Published private(set) var destination: SocialDestination?
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
    private var pendingOpen: (String, SocialDestination)?
    /// Opens an Inbox message's room, for a click on its notification, by the item's ID.
    var onOpenInboxItem: ((String) -> Void)?
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

    /// A notification for each Inbox message that arrived since the last poll. Invitations notify as room invitations
    /// already do, so they are left out here. Clicking one opens the message's room.
    func receiveInbox(_ items: [InboxItem], session: SocialSession, title: (InboxItem) -> String) {
        let snapshot = SocialNotificationSnapshot(session: session, requests: [], invitations: [])
        guard activeScope == snapshot.deliveryScope else { return }
        let messages = items.filter { !$0.isInvitation }
        let key = "CowchatMac.notifications.seen.inbox.v1." + snapshot.accountScope
        var ledger = defaults.data(forKey: key).flatMap { try? JSONDecoder().decode(InboxNotificationLedger.self, from: $0) }
            ?? InboxNotificationLedger()
        let fresh = Set(ledger.consume(messages.map(\.id)))
        if let data = try? JSONEncoder().encode(ledger) { defaults.set(data, forKey: key) }
        let arrived = messages.filter { fresh.contains($0.id) }
        guard enabled, let center, !arrived.isEmpty else { return }
        let scope = snapshot.deliveryScope
        let contents = arrived.map { item in
            ("\(scope).inbox.\(item.id)", Self.inboxContent(for: item, title: title(item), scope: scope))
        }
        Task {
            let status = await center.notificationSettings().authorizationStatus
            authorizationStatus = status
            guard enabled, activeScope == scope, status == .authorized || status == .provisional else { return }
            for (id, content) in contents {
                guard enabled, activeScope == scope else { return }
                do { try await center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil)) }
                catch { errorMessage = "The notification could not be delivered. \(error.localizedDescription)" }
            }
        }
    }

    nonisolated static let inboxItemKey = "inboxItem"

    /// An Inbox message's notification: who sent it, the room, the message, and the item's ID, which a click opens.
    static func inboxContent(for item: InboxItem, title: String, scope: String) -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = title
        content.subtitle = item.roomName
        content.body = item.message
        content.sound = .default
        content.threadIdentifier = "cowchat-inbox.\(scope)"
        content.userInfo = ["scope": scope, inboxItemKey: item.id]
        return content
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

    func consumeDestination(_ section: SocialDestination) -> Bool {
        guard destination == section else { return false }
        destination = nil
        return true
    }

    private func clearNotifications() {
        center?.removeAllPendingNotificationRequests()
        center?.removeAllDeliveredNotifications()
    }

    private func open(scope: String, section: SocialDestination) {
        guard route(scope: scope, section: section) else { return }
        NSApplication.shared.activate(ignoringOtherApps: true)
        NSApplication.shared.windows.first(where: { $0.canBecomeMain })?.makeKeyAndOrderFront(nil)
    }

    @discardableResult
    func route(scope: String, section: SocialDestination) -> Bool {
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
        let section = (info["section"] as? String).flatMap(SocialDestination.init(rawValue:))
        let inboxItemID = info[Self.inboxItemKey] as? String
        Task { @MainActor in
            if response.actionIdentifier == UNNotificationDefaultActionIdentifier, let scope {
                if let section {
                    open(scope: scope, section: section)
                } else if let inboxItemID, scope == activeScope {
                    onOpenInboxItem?(inboxItemID)
                    NSApplication.shared.activate(ignoringOtherApps: true)
                    NSApplication.shared.windows.first(where: { $0.canBecomeMain })?.makeKeyAndOrderFront(nil)
                }
            }
            completionHandler()
        }
    }
}
