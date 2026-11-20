import Foundation
import CowchatKit

/// A page of `GET /social/v1/inbox`: open invitations, messages that mention the person, and messages agents marked
/// for people in a session's room.
struct InboxReply: Decodable {
    let items: [InboxWireItem]
    let nextCursor: String?
}

struct InboxWireItem: Decodable {
    struct Sender: Decodable {
        /// `person` or `agent`.
        let kind: String
        let account: SocialAccount?
        let agentId: String?
        let name: String
    }

    let id: String
    /// `invitation`, `mention` or `marked`.
    let kind: String
    let roomId: String
    let roomName: String
    let at: String
    let content: String?
    let sender: Sender?
    let invitation: SocialRoomInvitation?
}

/// One row of the Inbox.
struct InboxEntry: Identifiable, Equatable {
    let id: String
    /// Who sent it, as the row names them.
    let title: String
    let roomID: String
    let roomName: String
    let at: Date
    let message: String
    var invitationID: String?
    /// Whether the invitation still waits for an answer.
    var isOpen = false
    /// The connection the room is on, when that is not the account's own: a mention in a plain-key room.
    var connectionID: String?

    var isOpenInvitation: Bool { invitationID != nil && isOpen }

    static let invitationLine = "Invited you to join."

    /// An item from the server, or nil for one the app does not show, such as a declined invitation.
    init?(wire: InboxWireItem, viewer: SocialAccount) {
        guard let at = CowchatDate.parse(wire.at) else { return nil }
        if wire.kind == "invitation" {
            guard let invitation = wire.invitation, invitation.status != "declined" else { return nil }
            self.init(id: wire.id, title: invitation.from.displayName, roomID: wire.roomId, roomName: wire.roomName,
                      at: at, message: Self.invitationLine, invitationID: invitation.id,
                      isOpen: invitation.status == "pending")
            return
        }
        guard let sender = wire.sender, let content = wire.content else { return nil }
        let title: String
        if sender.kind == "agent", let owner = sender.account, owner.id != viewer.id {
            title = "\(Self.firstName(of: owner.displayName))'s \(sender.name)"
        } else if sender.kind == "agent" {
            title = sender.name
        } else {
            title = sender.account?.displayName ?? sender.name
        }
        self.init(id: wire.id, title: title, roomID: wire.roomId, roomName: wire.roomName, at: at,
                  message: Self.line(for: content, viewer: viewer))
    }

    init(id: String, title: String, roomID: String, roomName: String, at: Date, message: String,
         invitationID: String? = nil, isOpen: Bool = false, connectionID: String? = nil) {
        self.connectionID = connectionID
        self.id = id
        self.title = title
        self.roomID = roomID
        self.roomName = roomName
        self.at = at
        self.message = message
        self.invitationID = invitationID
        self.isOpen = isOpen
    }

    /// Open invitations first, then everything else, each newest first.
    static func ordered(_ entries: [InboxEntry]) -> [InboxEntry] {
        entries.sorted { lhs, rhs in
            if lhs.isOpenInvitation != rhs.isOpenInvitation { return lhs.isOpenInvitation }
            if lhs.at != rhs.at { return lhs.at > rhs.at }
            return lhs.id < rhs.id
        }
    }

    /// One line of the message, without a leading mention of the person.
    static func line(for content: String, viewer: SocialAccount) -> String {
        let preview = MessagePreview.line(for: content)
        guard preview.hasPrefix("@"), let space = preview.firstIndex(where: \.isWhitespace) else { return preview }
        let token = preview[preview.index(after: preview.startIndex)..<space]
            .trimmingCharacters(in: CharacterSet(charactersIn: ",:;"))
            .lowercased()
        let names = [viewer.handle, firstName(of: viewer.displayName), viewer.displayName].map { $0.lowercased() }
        guard names.contains(token) else { return preview }
        let rest = preview[space...].trimmingCharacters(in: .whitespaces)
        return rest.isEmpty ? preview : rest
    }

    static func firstName(of displayName: String) -> String {
        displayName.split(separator: " ").first.map(String.init) ?? displayName
    }
}

/// `GET /social/v1/team`: each teammate who shares their work, with their live sessions.
struct TeamReply: Decodable {
    let sharing: Bool
    let people: [TeamPerson]
}

struct TeamPerson: Decodable, Identifiable {
    let account: SocialAccount
    let sessions: [TeamSession]
    var id: String { account.id }
}

struct TeamSession: Decodable, Identifiable {
    struct RoomRef: Decodable, Equatable {
        let roomId: String
        let name: String
    }

    let id: String
    let folder: String
    let branch: String
    let state: String?
    let startedAt: String
    let room: RoomRef?
    let pullRequest: MessagePullRequest?

    /// The branch, or the folder when the session has no branch.
    var name: String { branch.isEmpty ? folder : branch }

    var stateLabel: String {
        switch state?.lowercased() {
        case "working": "Working"
        case "prompt": "Waiting"
        default: "Idle"
        }
    }
}

/// A link that asks to be someone's friend: `cowchat://friend/<slug>?server=<wss url>` or
/// `https://<host>/friends/<slug>`. The rules match the Mac app's.
struct FriendLinkRoute: Equatable, Identifiable {
    let slug: String
    let endpoint: URL
    var id: String { endpoint.absoluteString + "/" + slug }

    static func parse(_ url: URL) -> FriendLinkRoute? {
        guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.user == nil, parts.password == nil, parts.fragment == nil else { return nil }
        let slug: String
        let endpoint: URL
        if parts.scheme == "cowchat" {
            guard parts.host == "friend", parts.port == nil else { return nil }
            let segments = parts.path.split(separator: "/", omittingEmptySubsequences: false)
            guard segments.count == 2, segments[0].isEmpty else { return nil }
            slug = String(segments[1])
            let query = parts.queryItems ?? []
            guard query.count <= 1, query.allSatisfy({ $0.name == "server" && $0.value != nil }) else { return nil }
            if let value = query.first?.value {
                guard let candidate = URL(string: value), let normalized = normalize(candidate) else { return nil }
                endpoint = normalized
            } else {
                endpoint = URL(string: ConnectionProfile.defaultGlobalURLString)!
            }
        } else {
            guard parts.scheme == "https", parts.query == nil, let host = parts.host else { return nil }
            let segments = parts.path.split(separator: "/", omittingEmptySubsequences: false)
            guard segments.count == 3, segments[0].isEmpty, segments[1] == "friends" else { return nil }
            slug = String(segments[2])
            var origin = URLComponents()
            origin.scheme = "wss"
            origin.host = host
            origin.port = parts.port
            origin.path = "/ws"
            guard let candidate = origin.url, let normalized = normalize(candidate) else { return nil }
            endpoint = normalized
        }
        let isValidSlug = (3...40).contains(slug.utf8.count) && slug.utf8.allSatisfy {
            (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || $0 == 45 || $0 == 95
        }
        return isValidSlug ? FriendLinkRoute(slug: slug, endpoint: endpoint) : nil
    }

    static func normalize(_ url: URL) -> URL? {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.scheme == "wss", parts.host != nil, parts.user == nil, parts.password == nil,
              parts.query == nil, parts.fragment == nil, parts.path == "/ws" else { return nil }
        parts.host = parts.host?.lowercased()
        if parts.port == 443 { parts.port = nil }
        return parts.url
    }
}

/// Posts notifications and sets the app icon's badge. Tests replace it.
@MainActor
protocol InboxNotifying {
    func requestPermission() async -> Bool
    func post(id: String, title: String, body: String)
    func setBadge(_ count: Int)
}

/// The account calls the iPhone makes. Tests replace it.
protocol AccountAPI {
    func me() async throws -> SocialMe
    func updateMe(displayName: String, handle: String) async throws -> SocialMe
    func inbox() async throws -> [InboxWireItem]
    func acceptInvitation(id: String) async throws
    func declineInvitation(id: String) async throws
    func friends() async throws -> [SocialFriend]
    func friendRequests() async throws -> [SocialFriendRequest]
    func addFriend(handle: String) async throws
    func answerFriendRequest(id: String, accept: Bool) async throws
    func removeFriend(id: String) async throws
    func rooms() async throws -> [SocialRoom]
    func team() async throws -> TeamReply
    func capabilities() async throws -> SocialCapabilities
    /// What first-run setup made for this account, or nil when it has not run.
    func setup() async throws -> SocialSetup?
    func establishSetup(displayName: String, handle: String) async throws -> SocialSetup?
    func checkHandle(_ handle: String) async throws -> SocialHandleAvailability
    func friendLink() async throws -> SocialFriendLink
    func previewFriendLink(slug: String) async throws -> SocialFriendLinkPreview
    func requestFriendLink(slug: String) async throws
    /// Pending room invitations the person sent or received.
    func invitations() async throws -> [SocialRoomInvitation]
    /// The people and agents in an account room.
    func participants(roomID: String) async throws -> SocialParticipants
    /// The person's own agents.
    func agents() async throws -> [SocialAgent]
    func removeAgent(roomID: String, agentID: String) async throws
    func addAgents(roomID: String, agentIDs: [String]) async throws
    /// Makes an account room, inviting one friend with it, and returns its id.
    func createRoom(name: String, firstFriendID: String?) async throws -> String
    /// Revokes this device's account key.
    func logout() async throws
    /// Leaves an account room and takes the person's agents out of it.
    func leaveRoom(roomID: String) async throws
    func invite(friendID: String, roomID: String) async throws
}

struct SocialAccountAPI: AccountAPI {
    let api: SocialAPI

    private struct Anything: Decodable {}

    func me() async throws -> SocialMe { try await api.social(["me"]) }

    func updateMe(displayName: String, handle: String) async throws -> SocialMe {
        try await api.social(["me"], method: "PATCH", body: [
            "display_name": displayName.trimmingCharacters(in: .whitespacesAndNewlines),
            "handle": SocialAPI.normalizedHandle(handle),
        ])
    }

    func inbox() async throws -> [InboxWireItem] {
        var items: [InboxWireItem] = []
        var cursor = ""
        for _ in 0..<50 {
            let reply: InboxReply = try await api.social(["inbox"], query: [URLQueryItem(name: "cursor", value: cursor)])
            items += reply.items
            guard let next = reply.nextCursor, !next.isEmpty, next != cursor else { break }
            cursor = next
        }
        return items
    }

    func acceptInvitation(id: String) async throws {
        let _: Anything = try await api.social(["room-invitations", id, "accept"], method: "POST", body: ["agent_ids": [String]()])
    }

    func declineInvitation(id: String) async throws {
        let _: Anything = try await api.social(["room-invitations", id, "decline"], method: "POST")
    }

    func friends() async throws -> [SocialFriend] {
        let reply: SocialFriends = try await api.social(["friends"])
        return reply.friends
    }

    func friendRequests() async throws -> [SocialFriendRequest] {
        let reply: SocialRequests = try await api.social(["friend-requests"])
        return reply.requests
    }

    func addFriend(handle: String) async throws {
        let _: Anything = try await api.social(["friend-requests"], method: "POST", body: ["handle": SocialAPI.normalizedHandle(handle)])
    }

    func answerFriendRequest(id: String, accept: Bool) async throws {
        let _: Anything = try await api.social(["friend-requests", id, accept ? "accept" : "decline"], method: "POST")
    }

    func removeFriend(id: String) async throws {
        let _: Anything = try await api.social(["friends", id], method: "DELETE")
    }

    func rooms() async throws -> [SocialRoom] {
        let reply: SocialRooms = try await api.social(["rooms"])
        return reply.rooms
    }

    func team() async throws -> TeamReply { try await api.social(["team"]) }

    func capabilities() async throws -> SocialCapabilities { try await api.social(["capabilities"]) }

    func setup() async throws -> SocialSetup? {
        let reply: SocialSetupReply = try await api.social(["setup"])
        return reply.setup
    }

    func establishSetup(displayName: String, handle: String) async throws -> SocialSetup? {
        let reply: SocialSetupReply = try await api.social(["setup"], method: "POST",
                                                            body: ["display_name": displayName, "handle": handle])
        return reply.setup
    }

    func checkHandle(_ handle: String) async throws -> SocialHandleAvailability {
        try await api.social(["handles", "availability"], method: "POST", body: ["handle": handle])
    }

    func friendLink() async throws -> SocialFriendLink { try await api.social(["me", "friend-link"], method: "POST") }

    func previewFriendLink(slug: String) async throws -> SocialFriendLinkPreview {
        try await api.social(["friend-links", slug])
    }

    func requestFriendLink(slug: String) async throws {
        let _: Anything = try await api.social(["friend-links", slug, "request"], method: "POST")
    }

    func invitations() async throws -> [SocialRoomInvitation] {
        let reply: SocialInvitations = try await api.social(["room-invitations"])
        return reply.invitations
    }

    func participants(roomID: String) async throws -> SocialParticipants {
        try await api.social(["rooms", roomID, "participants"])
    }

    func agents() async throws -> [SocialAgent] {
        let reply: SocialAgents = try await api.social(["agents"])
        return reply.agents
    }

    func removeAgent(roomID: String, agentID: String) async throws {
        let _: Anything = try await api.social(["rooms", roomID, "agents", agentID], method: "DELETE")
    }

    func addAgents(roomID: String, agentIDs: [String]) async throws {
        let _: Anything = try await api.social(["rooms", roomID, "agents"], method: "POST", body: ["agent_ids": agentIDs])
    }

    func createRoom(name: String, firstFriendID: String?) async throws -> String {
        struct Reply: Decodable { let roomId: String }
        var body: [String: Any] = ["name": name, "agent_ids": [String](), "proposed_agent_ids": [String]()]
        if let firstFriendID { body["friend_id"] = firstFriendID }
        let reply: Reply = try await api.social(["rooms"], method: "POST", body: body)
        return reply.roomId
    }

    func logout() async throws {
        let _: Anything = try await api.social(["logout"], method: "POST")
    }

    func leaveRoom(roomID: String) async throws {
        let _: Anything = try await api.social(["rooms", roomID, "membership"], method: "DELETE")
    }

    func invite(friendID: String, roomID: String) async throws {
        let _: Anything = try await api.social(["rooms", roomID, "invitations"], method: "POST",
                                                body: ["friend_id": friendID, "proposed_agent_ids": [String]()])
    }
}
