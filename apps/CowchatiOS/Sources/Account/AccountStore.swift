import Foundation
import CowchatKit

enum AccountPart: Hashable {
    case inbox, friends, team
}

/// What a signed-in account sees beyond its rooms: the Inbox, friends, the team's work and its own profile.
/// One per account connection.
@MainActor
final class AccountStore: ObservableObject {
    let connectionID: String
    @Published private(set) var account: SocialAccount?
    @Published private(set) var inbox: [InboxEntry] = [] { didSet { onInboxChange?() } }
    @Published private(set) var readIDs: Set<String> { didSet { onInboxChange?() } }
    /// Called when the Inbox or its read marks change, so the app can recount what is unread.
    var onInboxChange: (() -> Void)?
    @Published private(set) var friends: [SocialFriend] = []
    @Published private(set) var requests: [SocialFriendRequest] = []
    @Published private(set) var team: [TeamPerson] = []
    /// The account's session rooms, by room id.
    @Published private(set) var sessionRooms: [String: SocialRoom.Session] = [:]
    @Published private(set) var rooms: [SocialRoom] = []
    /// The person's own agents, which they may add to a room.
    @Published private(set) var ownAgents: [SocialAgent] = []
    /// The parts whose last load failed, so a screen can say so instead of looking empty.
    @Published private(set) var failedParts: Set<AccountPart> = []
    @Published var errorMessage: String?
    /// True for an account that has signed in but never chosen a name and handle.
    @Published private(set) var needsSetup = false
    /// Mentions of the person in plain-key rooms on the same server, which the account's own Inbox cannot see.
    private(set) var communityEntries: [InboxEntry] = []
    private var communitySources: Set<String> = []

    func setCommunity(_ entries: [InboxEntry], from sources: Set<String>) {
        communityEntries = entries
        communitySources = sources
    }

    private let api: any AccountAPI
    private let defaults: UserDefaults
    /// The server this account is on, as a `wss://` address.
    let endpoint: URL?
    private var serverEntries: [InboxEntry] = []
    private var readKey: String { "CowchatiOS.connection.\(connectionID).inboxReadIDs" }
    private var seenKey: String { "CowchatiOS.connection.\(connectionID).inboxSeenIDs" }
    private var seenSourcesKey: String { "CowchatiOS.connection.\(connectionID).inboxSeenSources" }

    init(connectionID: String, api: any AccountAPI, defaults: UserDefaults, endpoint: URL? = nil) {
        self.connectionID = connectionID
        self.api = api
        self.defaults = defaults
        self.endpoint = endpoint
        readIDs = Set(defaults.stringArray(forKey: "CowchatiOS.connection.\(connectionID).inboxReadIDs") ?? [])
    }

    // Each part loads on its own, so a server without one route still shows the rest. A part that fails keeps
    // what it showed before and is marked.
    /// Returns the unread Inbox items that arrived since the last refresh, for a notification each. The first
    /// refresh on a device returns none: what was already there is not news.
    @discardableResult
    func refresh() async -> [InboxEntry] {
        var failed: Set<AccountPart> = []
        if let me = try? await api.me() { account = me.account }
        // Only a server that answered "no setup yet" counts; a failed request changes nothing.
        do {
            if try await api.capabilities().setupVersion == 1 {
                let made = try await api.setup()
                needsSetup = made == nil
            }
        } catch {}
        do {
            let wire = try await api.inbox()
            guard let account else { throw CowchatConnectionError.invalidResponse }
            serverEntries = wire.compactMap { InboxEntry(wire: $0, viewer: account) }
        } catch CowchatConnectionError.server {
            // The server answered and refused: it has no Inbox route. Its invitations route still works.
            if let account, let list = try? await api.invitations() {
                serverEntries = list.filter { $0.to.id == account.id && $0.status == "pending" }.map(invitationEntry)
            } else {
                failed.insert(.inbox)
            }
        } catch {
            failed.insert(.inbox)
        }
        let merged = InboxEntry.ordered(serverEntries + communityEntries)
        if merged != inbox { inbox = merged }
        do {
            let (friendList, requestList) = (try await api.friends(), try await api.friendRequests())
            friends = friendList
            requests = requestList
        } catch {
            failed.insert(.friends)
        }
        if let list = try? await api.agents() { ownAgents = list }
        if let list = try? await api.rooms() {
            rooms = list
            sessionRooms = Dictionary(list.compactMap { room in room.session.map { (room.roomId, $0) } },
                                      uniquingKeysWith: { first, _ in first })
        }
        do {
            team = try await api.team().people.filter { !$0.sessions.isEmpty }
        } catch {
            failed.insert(.team)
        }
        if failed != failedParts { failedParts = failed }
        return failed.contains(.inbox) ? [] : arrivals()
    }

    private var invitationSeenAt: [String: Date] = [:]

    // The invitations route carries no date, so an invitation is dated from when this phone first saw it.
    private func invitationEntry(_ invitation: SocialRoomInvitation) -> InboxEntry {
        let id = "invitation.\(invitation.id)"
        let seen = invitationSeenAt[id] ?? Date()
        invitationSeenAt[id] = seen
        return InboxEntry(id: id, title: invitation.from.displayName, roomID: invitation.roomId,
                          roomName: invitation.roomName, at: seen, message: InboxEntry.invitationLine,
                          invitationID: invitation.id, isOpen: true)
    }

    private func arrivals() -> [InboxEntry] {
        let ids = Set(inbox.map(\.id))
        // What a connection already held when it was first read is not news either.
        let seenSources = Set(defaults.stringArray(forKey: seenSourcesKey) ?? [])
        let newSources = communitySources.subtracting(seenSources)
        defaults.set(seenSources.union(communitySources).sorted(), forKey: seenSourcesKey)
        guard var known = defaults.stringArray(forKey: seenKey).map(Set.init) else {
            defaults.set(ids.sorted(), forKey: seenKey)
            return []
        }
        known.formUnion(communityEntries.filter { $0.connectionID.map(newSources.contains) == true }.map(\.id))
        defaults.set(ids.union(known).sorted(), forKey: seenKey)
        return inbox.filter { !known.contains($0.id) && isUnread($0) }
    }

    // MARK: First-run setup

    @discardableResult
    func establishSetup(displayName: String, handle: String) async -> Bool {
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let handle = SocialAPI.normalizedHandle(handle)
        var made: SocialSetup?
        guard await perform({ made = try await self.api.establishSetup(displayName: name, handle: handle) }),
              made != nil else { return false }
        needsSetup = false
        if let me = try? await api.me() { account = me.account }
        return true
    }

    func handleCheck(_ input: String) async -> SocialHandleCheck {
        let handle = SocialAPI.normalizedHandle(input)
        guard let reply = try? await api.checkHandle(handle) else { return .failed(handle) }
        if !reply.valid { return .invalid(handle) }
        return reply.available ? .available(handle) : .unavailable(handle)
    }

    // MARK: Friend links

    func serves(_ route: FriendLinkRoute) -> Bool {
        endpoint.flatMap(FriendLinkRoute.normalize) == route.endpoint
    }

    func previewFriend(_ route: FriendLinkRoute) async -> SocialFriendLinkPreview? {
        guard serves(route) else { return nil }
        return try? await api.previewFriendLink(slug: route.slug)
    }

    @discardableResult
    func requestFriend(_ route: FriendLinkRoute) async -> Bool {
        guard serves(route), await perform({ try await self.api.requestFriendLink(slug: route.slug) }) else { return false }
        await refresh()
        return true
    }

    /// The person's own friend link, to hand to someone else.
    func myFriendLink() async -> URL? {
        var link: SocialFriendLink?
        guard await perform({ link = try await self.api.friendLink() }) else { return nil }
        return link?.url
    }

    // MARK: Inbox

    func isUnread(_ entry: InboxEntry) -> Bool { !readIDs.contains(entry.id) }

    var unreadCount: Int { inbox.filter(isUnread).count }

    func markRead(_ entry: InboxEntry) { setRead(readIDs.union([entry.id])) }

    func markUnread(_ entry: InboxEntry) { setRead(readIDs.subtracting([entry.id])) }

    /// Open invitations stay unread, since they wait for an answer.
    func markAllRead() {
        setRead(readIDs.union(inbox.filter { !$0.isOpenInvitation }.map(\.id)))
    }

    private func setRead(_ ids: Set<String>) {
        guard ids != readIDs else { return }
        readIDs = ids
        defaults.set(ids.sorted(), forKey: readKey)
    }

    @discardableResult
    func accept(_ entry: InboxEntry) async -> Bool {
        guard let id = entry.invitationID else { return false }
        guard await perform({ try await self.api.acceptInvitation(id: id) }) else { return false }
        markRead(entry)
        await refresh()
        return true
    }

    func decline(_ entry: InboxEntry) async {
        guard let id = entry.invitationID, await perform({ try await self.api.declineInvitation(id: id) }) else { return }
        inbox.removeAll { $0.id == entry.id }
        await refresh()
    }

    // MARK: Friends

    var incomingRequests: [SocialFriendRequest] {
        requests.filter { $0.status == "pending" && $0.to.id == account?.id }
    }

    var sentRequests: [SocialFriendRequest] {
        requests.filter { $0.status == "pending" && $0.from.id == account?.id }
    }

    @discardableResult
    func addFriend(handle: String) async -> Bool {
        guard await perform({ try await self.api.addFriend(handle: handle) }) else { return false }
        await refresh()
        return true
    }

    func answer(_ request: SocialFriendRequest, accept: Bool) async {
        guard await perform({ try await self.api.answerFriendRequest(id: request.id, accept: accept) }) else { return }
        await refresh()
    }

    func removeFriend(_ friend: SocialFriend) async {
        guard await perform({ try await self.api.removeFriend(id: friend.id) }) else { return }
        await refresh()
    }

    // MARK: Rooms

    /// A room someone else owns can be left. The owner's own room cannot.
    func canLeave(roomID: String) -> Bool {
        guard let account, let room = rooms.first(where: { $0.roomId == roomID }) else { return false }
        return room.ownerId != account.id
    }

    func canInvite(roomID: String) -> Bool {
        guard let account, let room = rooms.first(where: { $0.roomId == roomID }) else { return false }
        return room.ownerId == account.id
    }

    @discardableResult
    func leave(roomID: String) async -> Bool {
        guard await perform({ try await self.api.leaveRoom(roomID: roomID) }) else { return false }
        rooms.removeAll { $0.roomId == roomID }
        return true
    }

    @discardableResult
    func invite(friendID: String, roomID: String) async -> Bool {
        await perform { try await self.api.invite(friendID: friendID, roomID: roomID) }
    }

    func isAccountRoom(_ roomID: String) -> Bool { rooms.contains { $0.roomId == roomID } }

    func participants(roomID: String) async -> SocialParticipants? {
        try? await api.participants(roomID: roomID)
    }

    /// The person may take out their own agents, and the room's owner any agent.
    func canRemove(_ agent: SocialAgent, from room: SocialParticipants) -> Bool {
        guard let account else { return false }
        return agent.ownerId == account.id || room.ownerId == account.id
    }

    func agentsToAdd(to room: SocialParticipants) -> [SocialAgent] {
        ownAgents.filter { own in !room.agents.contains { $0.id == own.id } }
    }

    @discardableResult
    func removeAgent(_ agentID: String, roomID: String) async -> Bool {
        await perform { try await self.api.removeAgent(roomID: roomID, agentID: agentID) }
    }

    @discardableResult
    func addAgents(_ agentIDs: [String], roomID: String) async -> Bool {
        await perform { try await self.api.addAgents(roomID: roomID, agentIDs: agentIDs) }
    }

    /// Makes an account room. The first friend is invited with the room and the rest through its invitations.
    func createRoom(name: String, friendIDs: [String]) async -> String? {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        var roomID: String?
        guard await perform({
            let made = try await self.api.createRoom(name: name, firstFriendID: friendIDs.first)
            roomID = made
            for id in friendIDs.dropFirst() { try await self.api.invite(friendID: id, roomID: made) }
        }) else { return roomID }
        await refresh()
        return roomID
    }

    /// Revokes this device's key on the server. The connection is removed whether or not the server answers.
    func signOut() async {
        try? await api.logout()
    }

    // MARK: Profile

    @discardableResult
    func saveProfile(displayName: String, handle: String) async -> Bool {
        do {
            account = try await api.updateMe(displayName: displayName, handle: handle).account
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    private func perform(_ work: () async throws -> Void) async -> Bool {
        do {
            try await work()
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }
}
