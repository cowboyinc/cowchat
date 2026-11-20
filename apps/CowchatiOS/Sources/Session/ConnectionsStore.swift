import Foundation
import CowchatKit

enum AddOutcome: Equatable {
    /// The invite added a room to a connection that already existed.
    case joinedRoom(RoomRef)
    case addedConnection(String)
}

/// The open list of connections. Each one has its own `ChatSession`, so a connection that fails or
/// reconnects leaves the others alone. Publishes one room list merged across all of them.
@MainActor
final class ConnectionsStore: ObservableObject {
    @Published private(set) var connections: [ConnectionRecord]
    @Published private(set) var rooms: [MergedRoom] = []
    @Published private(set) var phases: [String: SessionPhase] = [:]
    @Published private(set) var isAdding = false
    /// One per connection that is signed in to an account.
    @Published private(set) var accounts: [AccountStore] = []

    static let defaultDisplayName = "iPhone"
    private static let recordsKey = "CowchatiOS.connections"

    private let makeConnection: () -> any CowchatConnectionProtocol
    private let credentials: any CowchatCredentialStore
    private let defaults: UserDefaults
    private let redeemInvite: InviteRedeemer
    private let sleep: (Duration) async throws -> Void
    private let makeAccountAPI: (URL, String) -> (any AccountAPI)?
    private let notifier: (any InboxNotifying)?
    private static let notificationsKey = "CowchatiOS.notifications.enabled"
    @Published private(set) var notificationsEnabled: Bool
    /// Unread Inbox items across every account, for the Inbox tab's badge.
    @Published private(set) var inboxUnread = 0
    /// A friend link the app was opened with, waiting to be confirmed.
    @Published var pendingFriendLink: FriendLinkRoute?
    private var sessions: [String: ChatSession] = [:]

    convenience init() {
        self.init(
            makeConnection: { CowchatConnection(profile: .local) },
            credentials: KeychainCowchatCredentialStore(service: "inc.cowboy.cowchat.ios"),
            defaults: .standard,
            redeemInvite: InviteRedemption.redeem,
            sleep: { try await Task.sleep(for: $0) },
            notifier: SystemInboxNotifier()
        )
    }

    init(
        makeConnection: @escaping () -> any CowchatConnectionProtocol,
        credentials: any CowchatCredentialStore,
        defaults: UserDefaults,
        redeemInvite: @escaping InviteRedeemer,
        sleep: @escaping (Duration) async throws -> Void,
        makeAccountAPI: @escaping (URL, String) -> (any AccountAPI)? = { endpoint, key in
            (try? SocialAPI(endpoint: endpoint, key: key)).map(SocialAccountAPI.init)
        },
        notifier: (any InboxNotifying)? = nil
    ) {
        self.makeAccountAPI = makeAccountAPI
        self.notifier = notifier
        notificationsEnabled = defaults.bool(forKey: Self.notificationsKey)
        self.makeConnection = makeConnection
        self.credentials = credentials
        self.defaults = defaults
        self.redeemInvite = redeemInvite
        self.sleep = sleep
        connections = defaults.data(forKey: Self.recordsKey)
            .flatMap { try? JSONDecoder().decode([ConnectionRecord].self, from: $0) } ?? []
        for record in connections { adopt(makeSession(record)) }
        republish()
    }

    func session(for connectionID: String) -> ChatSession? { sessions[connectionID] }

    // MARK: Lifecycle

    func restore() async {
        await withTaskGroup(of: Void.self) { group in
            for session in sessions.values {
                group.addTask { await session.restore() }
            }
        }
    }

    func retry(connectionID: String) async {
        await sessions[connectionID]?.restore()
    }

    func refreshRooms() async {
        for session in sessions.values { await session.refreshRooms() }
    }

    func sceneBecameActive() {
        sessions.values.forEach { $0.sceneBecameActive() }
    }

    func sceneEnteredBackground() {
        sessions.values.forEach { $0.sceneEnteredBackground() }
    }

    // MARK: Adding

    /// An invite joins a room on a connected plain-key connection to the same server when there is one.
    /// Anything else becomes a new connection.
    func add(
        server: ServerChoice,
        credential: Credential,
        name: String? = nil,
        displayName: String? = nil,
        accountID: String? = nil
    ) async throws -> AddOutcome {
        isAdding = true
        defer { isAdding = false }

        if case .invite(let pasted) = credential, let existing = inviteTaker(on: server) {
            do {
                let roomID = try await existing.redeemRoomInvite(token: ChatSession.inviteToken(in: pasted))
                return .joinedRoom(RoomRef(connectionID: existing.connectionID, roomID: roomID))
            } catch CowchatConnectionError.server(let message, _) {
                throw SessionError("The server did not accept this invite (\(message)).")
            }
        }

        if case .key(let pasted) = credential {
            let key = pasted.trimmingCharacters(in: .whitespacesAndNewlines)
            let isHeld = connections.contains {
                $0.server.isSameServer(as: server) && sessions[$0.id]?.holds(key: key) == true
            }
            if isHeld { throw SessionError("This key is already connected.") }
        }

        let record = ConnectionRecord(
            id: UUID().uuidString.lowercased(),
            server: server,
            name: Self.trimmed(name) ?? server.label,
            displayName: Self.trimmed(displayName) ?? Self.defaultDisplayName,
            fromInvite: { if case .invite = credential { true } else { false } }(),
            accountID: accountID
        )
        let session = makeSession(record)
        await session.signIn(server: server, credential: credential, displayName: record.displayName)

        if case .failed(let message) = session.phase {
            // A minted invite key may be single use, so that connection stays and can be retried.
            if session.hasSavedKey {
                append(record, session)
            } else {
                // A key a sign-in just made would otherwise stay valid on the server with nothing holding it.
                if accountID != nil, case .key(let key) = credential, case .global(let url) = server,
                   let endpoint = (try? ConnectionProfile.cowchatCloud(urlString: url, apiKey: key))?.endpointURL,
                   let api = makeAccountAPI(endpoint, key) {
                    Task { try? await api.logout() }
                }
                session.forget()
            }
            throw SessionError(message)
        }
        // A second sign-in to an account takes the place of the first.
        let replaced = accountID.map { id in
            connections.filter {
                $0.server.isSameServer(as: server) && ($0.accountID == id || account(for: $0.id)?.account?.id == id)
            }
        } ?? []
        append(record, session)
        replaced.forEach { remove(connectionID: $0.id) }
        return .addedConnection(record.id)
    }

    private func inviteTaker(on server: ServerChoice) -> ChatSession? {
        connections.lazy
            .filter { $0.server.isSameServer(as: server) }
            .compactMap { self.sessions[$0.id] }
            .first(where: \.canRedeemRoomInvites)
    }

    // MARK: Rooms

    var connectedConnections: [ConnectionRecord] {
        connections.filter { phases[$0.id] == .connected }
    }

    /// A connection signed in to an account makes an account room, with any friends picked. Others use the room frame.
    func createRoom(connectionID: String, name: String, friendIDs: [String] = []) async throws -> RoomRef {
        guard let session = sessions[connectionID] else { throw SessionError("Choose a connection.") }
        guard let account = account(for: connectionID) else {
            return RoomRef(connectionID: connectionID, roomID: try await session.createRoom(name: name))
        }
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw SessionError("Enter a room name.") }
        guard let roomID = await account.createRoom(name: name, friendIDs: friendIDs) else {
            let message = account.errorMessage ?? "The room could not be made."
            account.errorMessage = nil
            throw SessionError(message)
        }
        await session.refreshRooms()
        return RoomRef(connectionID: connectionID, roomID: roomID)
    }

    // MARK: Accounts

    func source(of connectionID: String) -> ConnectionSource {
        if sessions[connectionID]?.holdsAccountKey == true {
            return .account(handle: account(for: connectionID)?.account?.handle)
        }
        if connections.first(where: { $0.id == connectionID })?.fromInvite == true { return .invite }
        return .key(ending: sessions[connectionID]?.keyEnding)
    }

    func account(for connectionID: String) -> AccountStore? { accounts.first { $0.connectionID == connectionID } }

    /// Re-reads every account, adds mentions from plain-key rooms on the same server, and notifies about what is new.
    func refreshAccounts() async {
        var arrived: [InboxEntry] = []
        for account in accounts {
            if let found = await communityMentions(for: account) {
                account.setCommunity(found.entries, from: found.sources)
            }
            arrived += await account.refresh()
        }
        guard let notifier, notificationsEnabled else { return }
        for entry in arrived { notifier.post(id: entry.id, title: entry.title, body: entry.message) }
        notifier.setBadge(accounts.reduce(0) { $0 + $1.unreadCount })
    }

    /// Nil when they cannot be read just now (the account has not loaded, or a connection is down), so the
    /// last ones stay.
    private func communityMentions(for account: AccountStore) async -> (entries: [InboxEntry], sources: Set<String>)? {
        guard let viewer = account.account,
              let server = connections.first(where: { $0.id == account.connectionID })?.server else { return nil }
        var entries: [InboxEntry] = []
        var sources: Set<String> = []
        for record in connections where record.id != account.connectionID && record.server.isSameServer(as: server) {
            guard let session = sessions[record.id], !session.holdsAccountKey else { continue }
            guard session.phase == .connected else { return nil }
            sources.insert(record.id)
            for found in await session.mentions(of: viewer.id) {
                guard let at = CowchatDate.parse(found.message.timestamp) else { continue }
                entries.append(InboxEntry(
                    id: "community.\(found.roomID).\(found.message.id)", title: found.message.agentName,
                    roomID: found.roomID, roomName: found.roomName, at: at,
                    message: InboxEntry.line(for: found.message.content, viewer: viewer), connectionID: record.id
                ))
            }
        }
        return (entries, sources)
    }

    /// Turning notifications on asks iOS for permission; a refusal leaves them off.
    @discardableResult
    func setNotificationsEnabled(_ enabled: Bool) async -> Bool {
        var granted = enabled
        if enabled { granted = await notifier?.requestPermission() ?? false }
        notificationsEnabled = granted
        defaults.set(granted, forKey: Self.notificationsKey)
        if !granted { notifier?.setBadge(0) }
        return granted
    }

    private func countUnread() {
        let total = accounts.reduce(0) { $0 + $1.unreadCount }
        if total != inboxUnread {
            inboxUnread = total
            if notificationsEnabled { notifier?.setBadge(total) }
        }
    }

    func openFriendLink(_ url: URL) {
        if let route = FriendLinkRoute.parse(url) { pendingFriendLink = route }
    }

    func canLeave(_ room: RoomRef) -> Bool {
        account(for: room.connectionID)?.canLeave(roomID: room.roomID) == true
    }

    /// Leaves an account room for good: the server takes the person and their agents out of it.
    func leave(_ room: RoomRef) async {
        guard let account = account(for: room.connectionID), await account.leave(roomID: room.roomID) else { return }
        await sessions[room.connectionID]?.refreshRooms()
    }

    private func syncAccounts() {
        var next = accounts.filter { account in connections.contains { $0.id == account.connectionID } }
        for record in connections where !next.contains(where: { $0.connectionID == record.id }) {
            guard let credentials = sessions[record.id]?.accountCredentials,
                  let api = makeAccountAPI(credentials.endpoint, credentials.key) else { continue }
            let account = AccountStore(connectionID: record.id, api: api, defaults: defaults, endpoint: credentials.endpoint)
            account.onInboxChange = { [weak self] in self?.countUnread() }
            next.append(account)
            Task {
                await account.refresh()
                // The handle has arrived, and room rows show it.
                republish()
            }
        }
        if next.map(\.connectionID) != accounts.map(\.connectionID) { accounts = next }
        countUnread()
    }

    // MARK: Search

    func messageMatches(query: String) async -> Set<RoomRef> {
        var matches: Set<RoomRef> = []
        for (id, session) in sessions {
            for roomID in await session.messageMatches(query: query) {
                matches.insert(RoomRef(connectionID: id, roomID: roomID))
            }
        }
        return matches
    }

    func joinableRooms() async -> [JoinableRoom] {
        var found: [JoinableRoom] = []
        for record in connections {
            guard let session = sessions[record.id] else { continue }
            found += await session.joinableRooms().map {
                JoinableRoom(ref: RoomRef(connectionID: record.id, roomID: $0.id), name: $0.name, connectionName: record.name)
            }
        }
        return found.sorted { ($0.name, $0.connectionName) < ($1.name, $1.connectionName) }
    }

    func join(_ room: RoomRef) async throws {
        guard let session = sessions[room.connectionID] else { throw SessionError("Choose a connection.") }
        try await session.join(roomID: room.roomID)
    }

    // MARK: Editing

    func rename(connectionID: String, to name: String) {
        guard let index = connections.firstIndex(where: { $0.id == connectionID }),
              let name = Self.trimmed(name) else { return }
        connections[index].name = name
        persist()
        republish()
    }

    func remove(connectionID: String) {
        guard let session = sessions.removeValue(forKey: connectionID) else { return }
        if let account = account(for: connectionID) { Task { await account.signOut() } }
        session.onChange = nil
        session.forget()
        let prefix = "CowchatiOS.connection.\(connectionID)."
        for key in defaults.dictionaryRepresentation().keys where key.hasPrefix(prefix) { defaults.removeObject(forKey: key) }
        connections.removeAll { $0.id == connectionID }
        persist()
        republish()
    }

    /// True for a room an invite added, and for the rooms of a connection an invite made.
    func canRemove(_ room: RoomRef) -> Bool {
        guard let record = connections.first(where: { $0.id == room.connectionID }) else { return false }
        if record.fromInvite { return room.roomID != "lobby" }
        return sessions[room.connectionID]?.wasAddedByInvite(roomID: room.roomID) == true
    }

    func remove(room: RoomRef) async {
        guard canRemove(room), let record = connections.first(where: { $0.id == room.connectionID }) else { return }
        if record.fromInvite {
            remove(connectionID: record.id)
        } else {
            await sessions[record.id]?.removeInvitedRoom(roomID: room.roomID)
        }
    }

    // MARK: Internals

    private func makeSession(_ record: ConnectionRecord) -> ChatSession {
        ChatSession(
            record: record,
            connection: makeConnection(),
            credentials: credentials,
            defaults: defaults,
            redeemInvite: redeemInvite,
            sleep: sleep
        )
    }

    private func adopt(_ session: ChatSession) {
        sessions[session.connectionID] = session
        session.onChange = { [weak self] in self?.republish() }
    }

    private func append(_ record: ConnectionRecord, _ session: ChatSession) {
        connections.append(record)
        adopt(session)
        persist()
        republish()
    }

    private func persist() {
        defaults.set(try? JSONEncoder().encode(connections), forKey: Self.recordsKey)
    }

    private func republish() {
        var phases: [String: SessionPhase] = [:]
        var merged: [MergedRoom] = []
        for record in connections {
            guard let session = sessions[record.id] else { continue }
            phases[record.id] = session.phase
            // A connection still named after its server, or sharing a name, also says whose it is: the account's
            // handle, the key's ending, or Invited. A room an invite added to a key says Invited too.
            let needsOwner = record.name == record.server.label
                || connections.contains { $0.id != record.id && $0.name == record.name }
            let owner = source(of: record.id).label
            merged += session.rooms.map { room in
                let label = session.wasAddedByInvite(roomID: room.id) ? ConnectionSource.invite.label : owner
                return MergedRoom(ref: RoomRef(connectionID: record.id, roomID: room.id),
                                  connectionName: needsOwner ? "\(record.name) · \(label)" : record.name, summary: room)
            }
        }
        merged.sort {
            // A session waiting for its person comes first.
            let (waitingA, waitingB) = ($0.summary.sessionState == "prompt", $1.summary.sessionState == "prompt")
            if waitingA != waitingB { return waitingA }
            let (a, b) = ($0.summary.lastActivity ?? .distantPast, $1.summary.lastActivity ?? .distantPast)
            if a != b { return a > b }
            return ($0.summary.name, $0.connectionName) < ($1.summary.name, $1.connectionName)
        }
        if self.phases != phases { self.phases = phases }
        if rooms != merged { rooms = merged }
        syncAccounts()
    }

    private static func trimmed(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
}
