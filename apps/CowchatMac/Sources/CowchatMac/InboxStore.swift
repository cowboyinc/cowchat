import Foundation

/// A page of `GET /social/v1/inbox?cursor=` (Chad's plan, task 5): across the account's rooms, open invitations,
/// messages that mention the person, and messages agents marked for people in their Home and session rooms. In company
/// rooms only mentions count. `next_cursor` names the next page, or is missing on the last.
struct InboxReply: Decodable {
    let items: [InboxWireItem]
    let nextCursor: String?
}

/// One item as the server sends it: an invitation with the invitation, or a message with its text and sender.
struct InboxWireItem: Decodable {
    struct Sender: Decodable {
        /// `person` or `agent`, as the message's `sender_kind`.
        let kind: String
        /// The person who wrote it, or the person whose session it is.
        let account: SocialAccount?
        let agentId: String?
        /// The person's display name, or the session's name as its person's Home names it.
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

/// One row of the Inbox: who sent it, the room it is in, when, and one line of what it says.
struct InboxItem: Identifiable, Equatable {
    enum Sender: Equatable {
        /// A person, by their display name.
        case person(String)
        /// One of the person's own sessions, by its agent ID, with the name the server gave it.
        case ownSession(agentID: String, name: String)
        /// Someone else's session, with its person's first name when it is known.
        case session(owner: String?, name: String)
    }

    let id: String
    let sender: Sender
    let roomID: String
    /// The room's name as the server gives it. The page shows "Home" for the account's Home.
    let roomName: String
    /// Where the room is: the account's rooms, or the community rooms the API key reaches.
    let server: WorkspaceStore.Server
    let at: Date
    /// The message as the sidebar previews it, without a leading mention of the person, or the invitation's line.
    let message: String
    /// The invitation's ID, for an invitation to a room.
    var invitationID: String?
    /// Whether the invitation still waits for an answer.
    var isOpen = false

    var isInvitation: Bool { invitationID != nil }
    var isOpenInvitation: Bool { isInvitation && isOpen }

    // copy:
    /// The row's title: who sent it. `ownSessionName` gives the name Home shows for one of the person's sessions.
    func title(ownSessionName: (String) -> String?) -> String {
        switch sender {
        case let .person(name): return name
        case let .ownSession(agentID, name): return ownSessionName(agentID) ?? name
        case let .session(owner?, name): return "\(owner)'s \(name)"
        case let .session(nil, name): return name
        }
    }
}

extension InboxItem {
    // copy:
    static let invitationLine = "Invited you to join."

    /// An item from the server, or nil for one this app does not show, such as a declined invitation.
    init?(wire: InboxWireItem, viewer: SocialAccount) {
        guard let at = wire.at.cowchatDate else { return nil }
        if wire.kind == "invitation" {
            guard let invitation = wire.invitation, invitation.status != "declined" else { return nil }
            self.init(id: wire.id, sender: .person(invitation.from.displayName), roomID: wire.roomId,
                      roomName: wire.roomName, server: .accountGlobal, at: at, message: Self.invitationLine,
                      invitationID: invitation.id, isOpen: invitation.status == "pending")
            return
        }
        guard let sender = wire.sender, let content = wire.content else { return nil }
        let from: Sender
        if sender.kind == "agent" {
            if sender.account?.id == viewer.id {
                from = .ownSession(agentID: sender.agentId ?? "", name: sender.name)
            } else {
                from = .session(owner: sender.account.map { Self.firstName(of: $0.displayName) }, name: sender.name)
            }
        } else {
            from = .person(sender.account?.displayName ?? sender.name)
        }
        self.init(id: wire.id, sender: from, roomID: wire.roomId, roomName: wire.roomName, server: .accountGlobal,
                  at: at, message: Self.line(for: content, viewer: viewer))
    }

    /// The Inbox's order: open invitations first, newest first, then every other item, newest first.
    static func ordered(_ items: [InboxItem]) -> [InboxItem] {
        items.sorted { lhs, rhs in
            if lhs.isOpenInvitation != rhs.isOpenInvitation { return lhs.isOpenInvitation }
            if lhs.at != rhs.at { return lhs.at > rhs.at }
            return lhs.id < rhs.id
        }
    }

    /// A message's line: the sidebar's preview of it, with a leading mention of the person removed ("@Chad could you
    /// review…" reads "could you review…"). Other mentions stay.
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

    /// A person's first name, from their display name.
    static func firstName(of displayName: String) -> String {
        displayName.split(whereSeparator: \.isWhitespace).first.map(String.init) ?? displayName
    }
}

/// The person's Inbox: the server's items and the community rooms' mentions, polled with the friends every 30
/// seconds, and which of them the person has read, kept on this Mac for each account.
@MainActor
final class InboxStore: ObservableObject {
    /// How many of a community room's newest messages are read for mentions when the room has moved on.
    static let communityDepth = 50
    /// How many items the menu bar's panel shows.
    static let panelLimit = 4

    /// Every item, in the Inbox's order.
    @Published private(set) var items: [InboxItem] = []
    /// False until the first poll answers, while the page draws only its header.
    @Published private(set) var isLoaded = false
    @Published private(set) var readIDs: Set<String> = []
    /// The open invitation whose Join sheet is showing.
    @Published var joining: InboxItem?
    /// Whether the menu bar's panel is open.
    @Published var isPanelShown = false

    /// Where the read state is kept: the workspace's defaults, which also hold the app's settings.
    let defaults: UserDefaults
    private(set) var account: SocialAccount?
    private var scope: String?
    private var generation = 0
    private var invitations: [String: SocialRoomInvitation] = [:]
    /// Each community room's mentions of the person, by the room's activity when they were read.
    private var communityMentions: [String: (activity: String, items: [InboxItem])] = [:]

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    nonisolated static func readStateKey(accountScope: String) -> String { "CowchatMac.inbox.read.v1." + accountScope }

    /// Starts over for the account signed in, with what this Mac read of its Inbox before.
    func use(account: SocialAccount?, scope: String?) {
        guard account?.id != self.account?.id || scope != self.scope else { return }
        generation += 1
        self.account = account
        self.scope = scope
        items = []
        isLoaded = false
        joining = nil
        isPanelShown = false
        invitations = [:]
        communityMentions = [:]
        readIDs = scope.flatMap { defaults.array(forKey: Self.readStateKey(accountScope: $0)) as? [String] }
            .map(Set.init) ?? []
    }

    // MARK: Read state

    func isUnread(_ item: InboxItem) -> Bool { !readIDs.contains(item.id) }

    /// Every unread item, open invitations included, which the sidebar, the menu bar and the Dock show.
    var unreadCount: Int { items.filter(isUnread).count }

    /// Whether Mark all read has anything to do: an unread item that is not an open invitation.
    var hasUnreadMessages: Bool { items.contains { !$0.isOpenInvitation && isUnread($0) } }

    /// The menu bar panel's items: the newest unread ones.
    var newestUnread: [InboxItem] {
        Array(items.filter(isUnread).sorted { ($0.at, $0.id) > ($1.at, $1.id) }.prefix(Self.panelLimit))
    }

    func markRead(_ item: InboxItem) { setRead(readIDs.union([item.id])) }

    func markUnread(_ item: InboxItem) { setRead(readIDs.subtracting([item.id])) }

    /// Marks every unread message read. Open invitations stay unread, since they wait for an answer.
    func markAllRead() {
        setRead(readIDs.union(items.filter { !$0.isOpenInvitation }.map(\.id)))
    }

    private func setRead(_ ids: Set<String>) {
        guard ids != readIDs else { return }
        readIDs = ids
        if let scope { defaults.set(ids.sorted(), forKey: Self.readStateKey(accountScope: scope)) }
    }

    /// The invitation an item stands for, as the server last sent it.
    func invitation(for item: InboxItem) -> SocialRoomInvitation? {
        item.invitationID.flatMap { invitations[$0] }
    }

    // MARK: Polling

    /// Takes a poll's items. An invitation answered since the last poll is read, and one declined leaves the Inbox.
    func receive(_ fresh: [InboxItem], invitations: [SocialRoomInvitation] = []) {
        let answered = items.filter(\.isOpenInvitation).map(\.id).filter { id in
            fresh.contains { $0.id == id && $0.isInvitation && !$0.isOpenInvitation }
        }
        var seen: Set<String> = []
        let unique = fresh.filter { seen.insert($0.id).inserted }
        self.invitations = Dictionary(invitations.map { ($0.id, $0) }, uniquingKeysWith: { _, last in last })
        let ordered = InboxItem.ordered(unique)
        if ordered != items { items = ordered }
        isLoaded = true
        if !answered.isEmpty { setRead(readIDs.union(answered)) }
    }

    /// Takes an item out at once, as declining its invitation does, before the next poll confirms it.
    func remove(_ item: InboxItem) {
        items.removeAll { $0.id == item.id }
    }

    /// Asks the server for the account's items, page by page, and reads the community rooms that moved on for mentions
    /// of the person. `people` names the people a community message's sender may be, and `owner` says whose a
    /// community room's session is. A server without the route leaves the Inbox as it was.
    func refresh(social: SocialStore, community: ChatStore?, people: [SocialAccount],
                 owner: @escaping (LiveSession) -> SocialAccount?) async {
        guard let account, social.savedSession?.account.id == account.id else { return }
        let ticket = generation
        var wire: [InboxWireItem] = []
        var cursor = ""
        do {
            for _ in 0..<50 {
                let reply: InboxReply = try await social.readingAPI()
                    .social(["inbox"], query: [URLQueryItem(name: "cursor", value: cursor)])
                guard ticket == generation else { return }
                wire += reply.items
                guard let next = reply.nextCursor, !next.isEmpty, next != cursor else { break }
                cursor = next
            }
        } catch {
            return
        }
        let mentions = await mentions(in: community, viewer: account, people: people, owner: owner, ticket: ticket)
        guard ticket == generation else { return }
        receive(wire.compactMap { InboxItem(wire: $0, viewer: account) } + mentions,
                invitations: wire.compactMap(\.invitation))
    }

    /// The person's mentions in the community rooms the API key reaches, which the account's Inbox endpoint can't see
    /// (Chad's plan, task 5). A room is read again only when its activity has moved on.
    private func mentions(in store: ChatStore?, viewer: SocialAccount, people: [SocialAccount],
                          owner: (LiveSession) -> SocialAccount?, ticket: Int) async -> [InboxItem] {
        guard let store, store.connectionStatus.isConnected else {
            return communityMentions.values.flatMap(\.items)
        }
        var found: [InboxItem] = []
        var listed: Set<String> = []
        for room in store.rooms where !room.encrypted {
            listed.insert(room.id)
            let activity = room.lastActivity ?? room.createdAt
            if let known = communityMentions[room.id], known.activity == activity {
                found += known.items
                continue
            }
            guard let messages = try? await store.recentMessages(in: room.id, limit: Self.communityDepth) else {
                found += communityMentions[room.id]?.items ?? []
                continue
            }
            guard ticket == generation else { return [] }
            let mentioning = messages.filter {
                $0.metadata.mentions.contains(viewer.id) && $0.agentID != store.agentID && !$0.isThinking
            }
            var agents: [AgentPresence] = []
            if mentioning.contains(where: { $0.metadata.senderKind == .agent }) {
                agents = (try? await store.agents(in: room.id)) ?? []
                guard ticket == generation else { return [] }
            }
            let items = mentioning.compactMap { message -> InboxItem? in
                guard let at = message.timestamp.cowchatDate else { return nil }
                return InboxItem(id: "community.\(room.id).\(message.id)",
                                 sender: Self.communitySender(of: message, agents: agents, people: people,
                                                              viewer: viewer, owner: owner),
                                 roomID: room.id, roomName: room.name, server: .global, at: at,
                                 message: InboxItem.line(for: message.content, viewer: viewer))
            }
            communityMentions[room.id] = (activity, items)
            found += items
        }
        communityMentions = communityMentions.filter { listed.contains($0.key) }
        return found
    }

    /// Who wrote a community room's message. On the API key's connection a person's messages carry their first name and
    /// a session's carry its name alone, so a person is found among `people`, and a session's person from its folder
    /// and branch.
    nonisolated static func communitySender(of message: ChatMessage, agents: [AgentPresence], people: [SocialAccount],
                                            viewer: SocialAccount,
                                            owner: (LiveSession) -> SocialAccount?) -> InboxItem.Sender {
        if message.metadata.senderKind == .agent {
            let session = agents.first { $0.agentID == message.agentID }.flatMap(LiveSession.init(agent:))
            guard let person = session.flatMap(owner) else { return .session(owner: nil, name: message.agentName) }
            if person.id == viewer.id { return .ownSession(agentID: message.agentID, name: message.agentName) }
            return .session(owner: InboxItem.firstName(of: person.displayName), name: message.agentName)
        }
        let name = message.agentName.lowercased()
        let person = people.first {
            $0.handle.lowercased() == name || $0.displayName.lowercased() == name
                || InboxItem.firstName(of: $0.displayName).lowercased() == name
        }
        return .person(person?.displayName ?? message.agentName)
    }
}
