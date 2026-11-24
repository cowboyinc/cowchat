import Foundation
import CowchatKit

@MainActor
final class ChatSession: ObservableObject {
    @Published var phase: SessionPhase = .signedOut { didSet { if phase != oldValue { onChange?() } } }
    @Published var displayName = ""
    @Published var server: ServerChoice = .defaultGlobal
    @Published var rooms: [RoomSummary] = [] { didSet { if rooms != oldValue { onChange?() } } }
    @Published var timelines: [String: RoomTimeline] = [:]
    @Published var members: [String: [RoomMember]] = [:]
    @Published private(set) var showAllRoomIDs: Set<String> = []
    /// Each room's live thinking signals, by agent.
    @Published private(set) var pulses: [String: [String: AgentThinkingPulse]] = [:]

    let connectionID: String
    /// Called after `phase` or `rooms` changes, so the connections store can republish its merged view.
    var onChange: (() -> Void)?

    private let connection: any CowchatConnectionProtocol
    private let storage: SessionStorage
    private let redeemInvite: InviteRedeemer
    private let sleep: (Duration) async throws -> Void
    private let fetchBlob: BlobFetcher
    private let pageSize = 50

    private var profile: ConnectionProfile?
    private(set) var myAgentID: String?
    private var serverRooms: [Room] = []
    private var messages: [String: [String: ChatMessage]] = [:]
    private var secrets: [String: String] = [:]
    private var readStates: [String: ReadState] = [:]
    private var liveActivity: [String: Date] = [:]
    private var joinedRoomIDs: Set<String> = []
    private var openRoomIDs: Set<String> = []
    private var archivedRoomIDs: Set<String> = []
    private var sessionStates: [String: String] = [:]
    private var transportUp = false { didSet { if !transportUp { caughtUp = [] } } }
    /// For each room, the newest message known to have nothing missing before it back to the oldest one loaded.
    private var caughtUpThrough: [String: Int] = [:]
    /// Rooms whose last catch-up finished on this connection. Only then does a live message extend the run.
    private var caughtUp: Set<String> = []
    private var reconnectTask: Task<Void, Never>?
    // Bumped by every sign-in, sign-out, background and reconnect restart, so work
    // started under an older generation drops its results instead of racing.
    private var generation = 0

    init(
        record: ConnectionRecord,
        connection: any CowchatConnectionProtocol,
        credentials: any CowchatCredentialStore,
        defaults: UserDefaults,
        redeemInvite: @escaping InviteRedeemer,
        sleep: @escaping (Duration) async throws -> Void,
        fetchBlob: @escaping BlobFetcher = { try await SocialAPI.secureSession.data(for: $0) }
    ) {
        self.fetchBlob = fetchBlob
        self.connection = connection
        self.connectionID = record.id
        self.storage = SessionStorage(defaults: defaults, credentials: credentials, connectionID: record.id)
        self.redeemInvite = redeemInvite
        self.sleep = sleep
        server = record.server
        displayName = record.displayName
        connection.onEvent = { [weak self] type, payload in self?.handleEvent(type, payload) }
        connection.onStatusChange = { [weak self] status in self?.handleStatus(status) }
    }

    // MARK: Sign in and out

    func restore() async {
        switch phase {
        case .signedOut, .failed: break
        case .connecting, .connected, .reconnecting: return
        }
        guard let key = storage.apiKey else { return }
        guard let error = await connectFresh(server: server, credential: .key(key), displayName: displayName)
            as? CowchatConnectionError else { return }
        if case .server = error { return }
        startReconnect()
    }

    func signIn(server: ServerChoice, credential: Credential, displayName: String) async {
        _ = await connectFresh(server: server, credential: credential, displayName: displayName)
    }

    /// Signs out. The saved key goes first: if the Keychain keeps it, this throws and the session stays signed
    /// in, which is how the next launch would find it anyway.
    func signOut() throws {
        try storage.eraseCredentials()
        stopActivity()
        phase = .signedOut
        connection.disconnect()
        resetAccountState()
    }

    /// Signs out and drops the agent identity too. For a connection that is being removed.
    func forget() throws {
        try signOut()
        try storage.eraseAll()
    }

    /// True when a key is saved for this connection, including one minted from an invite before a failed connect.
    var hasSavedKey: Bool { storage.apiKey != nil }

    func holds(key: String) -> Bool { storage.apiKey == key }

    /// The last four characters of this connection's key, for telling keys apart on screen.
    var keyEnding: String? { storage.apiKey.map { String($0.suffix(4)) } }

    /// True for a connection signed in to an account, connected or not.
    var holdsAccountKey: Bool { storage.apiKey?.hasPrefix("csoc_") == true }

    /// The endpoint and key for account calls, for a connection signed in to an account on a Global server.
    /// It does not need the socket, so a background refresh can read the Inbox.
    var accountCredentials: (endpoint: URL, key: String)? {
        guard let key = storage.apiKey, key.hasPrefix("csoc_"), case .global(let url) = server,
              let endpoint = (try? ConnectionProfile.cowchatCloud(urlString: url, apiKey: key))?.endpointURL else { return nil }
        return (endpoint, key)
    }

    /// Account keys (`csoc_`) cannot take room invites; plain keys can.
    var canRedeemRoomInvites: Bool {
        guard phase == .connected, let key = profile?.apiKey else { return false }
        return !key.hasPrefix("csoc_")
    }

    /// Adds the invited room to this connection's key and returns the room id.
    func redeemRoomInvite(token: String) async throws -> String {
        let roomID = try await connection.redeemInvite(token: token)
        storage.invitedRoomIDs.insert(roomID)
        storage.removedRoomIDs.remove(roomID)
        try? await loadRooms()
        return roomID
    }

    func wasAddedByInvite(roomID: String) -> Bool { storage.invitedRoomIDs.contains(roomID) }

    /// Takes an invited room off this iPhone. The key keeps its access on the server.
    func removeInvitedRoom(roomID: String) async throws {
        guard wasAddedByInvite(roomID: roomID) else { return }
        // The room's secret goes first. If the Keychain keeps it, the room stays, with the secret it still holds.
        try setSecret(nil, roomID: roomID)
        if joinedRoomIDs.remove(roomID) != nil { try? await connection.leave(roomID: roomID) }
        storage.invitedRoomIDs.remove(roomID)
        storage.removedRoomIDs.insert(roomID)
        openRoomIDs.remove(roomID)
        timelines[roomID] = nil
        messages[roomID] = nil
        members[roomID] = nil
        publishRooms()
    }

    private func connectFresh(server: ServerChoice, credential: Credential, displayName: String) async -> Error? {
        stopActivity()
        let generation = generation
        resetAccountState()
        phase = .connecting
        do {
            let key = try await apiKey(for: credential, on: server)
            if case .invite = credential {
                // Invites can be single use, so keep the minted key even if connecting fails next.
                try storage.save(apiKey: key)
            }
            let profile = try Self.profile(for: server, apiKey: key)
            self.profile = profile
            try await establish(profile: profile, displayName: displayName, generation: generation)
            try storage.save(apiKey: key)
            self.server = server
            self.displayName = displayName
            phase = .connected
            return nil
        } catch {
            guard generation == self.generation else { return error }
            connection.disconnect()
            self.server = server
            self.displayName = displayName
            phase = .failed(Self.message(for: error, profile: profile))
            return error
        }
    }

    private func apiKey(for credential: Credential, on server: ServerChoice) async throws -> String {
        switch credential {
        case .key(let key):
            let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { throw SessionError("Enter an API key.") }
            return trimmed
        case .invite(let token):
            guard case .global(let urlString) = server else {
                throw SessionError("Invite links work only with the Global server. Sign in to this server with an API key.")
            }
            let pasted = token.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !pasted.isEmpty else { throw SessionError("Enter an invite token.") }
            // The Mac app copies a whole connect prompt with the token inside it.
            let token = Self.inviteToken(in: pasted)
            let profile = try ConnectionProfile.cowchatCloud(urlString: urlString, apiKey: token)
            guard let socketURL = profile.endpointURL,
                  let endpoint = InviteRedemption.endpoint(forWebSocketURL: socketURL) else {
                throw ConnectionProfileError.invalidCloudURL
            }
            return try await redeemInvite(endpoint, token)
        }
    }

    static func inviteToken(in pasted: String) -> String {
        if case .invite(let token) = PastedCredential.detect(pasted) { return token }
        return pasted.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func profile(for server: ServerChoice, apiKey: String) throws -> ConnectionProfile {
        switch server {
        case .global(let url):
            return try .cowchatCloud(urlString: url, apiKey: apiKey)
        case .tcp(let host, let port):
            return .local(host: host, port: port, apiKey: apiKey)
        }
    }

    private func establish(profile: ConnectionProfile, displayName: String, generation: Int) async throws {
        connection.reconfigure(profile: profile)
        try await connection.connect()
        try checkCurrent(generation)
        let registration: CowchatRegistration
        do {
            registration = try await connection.register(name: displayName, agentID: storage.stableAgentID(apiKey: profile.apiKey))
        } catch CowchatConnectionError.server(_, .some("agent_id_taken")) {
            registration = try await connection.register(name: displayName, agentID: storage.rotateAgentID())
        }
        try checkCurrent(generation)
        myAgentID = registration.agentID
        joinedRoomIDs = registration.restoredRoomIDs
        transportUp = true
        try await loadRooms()
        try checkCurrent(generation)
    }

    private func checkCurrent(_ generation: Int) throws {
        guard generation == self.generation else { throw CancellationError() }
    }

    private func stopActivity() {
        generation += 1
        reconnectTask?.cancel()
        reconnectTask = nil
    }

    private func resetAccountState() {
        profile = nil
        myAgentID = nil
        serverRooms = []
        messages = [:]
        caughtUpThrough = [:]
        caughtUp = []
        secrets = [:]
        readStates = storage.readStates
        showAllRoomIDs = storage.showAllRoomIDs
        pulses = [:]
        sessionStates = [:]
        liveActivity = [:]
        joinedRoomIDs = []
        openRoomIDs = []
        transportUp = false
        rooms = []
        timelines = [:]
        members = [:]
    }

    private static func message(for error: Error, profile: ConnectionProfile?) -> String {
        let place = profile.map { $0.endpointURL?.host ?? $0.endpointDescription } ?? "the server"
        switch error {
        case CowchatConnectionError.server(let message, _):
            return "The server refused this key (\(message)). Check the key and try again."
        case is CowchatCredentialStoreError:
            return "Cowchat could not use this iPhone's Keychain."
        case is CowchatConnectionError, is URLError:
            return "Could not reach \(place). Check the address and your network, then try again."
        default:
            return (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
        }
    }

    // MARK: Rooms

    func refreshRooms() async {
        guard phase == .connected else { return }
        try? await loadRooms()
    }

    private func loadRooms() async throws {
        serverRooms = try await connection.listRooms()
        for room in serverRooms where secrets[room.id] == nil {
            secrets[room.id] = storage.secret(roomID: room.id)
        }
        publishRooms()
    }

    private func publishRooms() {
        let removed = storage.removedRoomIDs
        archivedRoomIDs = storage.archivedRoomIDs
        rooms = serverRooms.filter { !removed.contains($0.id) }.map(summary).sorted {
            ($0.lastActivity ?? .distantPast, $1.name) > ($1.lastActivity ?? .distantPast, $0.name)
        }
    }

    private func summary(_ room: Room) -> RoomSummary {
        let hasCiphertext = messages[room.id]?.values.contains { $0.content.hasPrefix("cow1:") } == true
        let activity = [CowchatDate.parse(room.lastActivity), liveActivity[room.id]].compactMap { $0 }.max()
        return RoomSummary(
            id: room.id,
            name: room.name,
            isEncrypted: room.encrypted || hasCiphertext,
            hasSecret: secrets[room.id] != nil,
            isUnread: readStates[room.id]?.isUnread ?? false,
            lastActivity: activity,
            isArchived: archivedRoomIDs.contains(room.id),
            sessionState: sessionStates[room.id]
        )
    }

    // MARK: Room actions

    /// The server lets only the agent that created a room rename it.
    func canRename(roomID: String) -> Bool {
        guard roomID != "lobby", let myAgentID else { return false }
        return serverRooms.first { $0.id == roomID }?.createdBy == myAgentID
    }

    func rename(roomID: String, to proposed: String) async throws {
        let name = proposed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { throw SessionError("Enter a room name.") }
        let updated = try await connection.rename(roomID: roomID, name: name)
        if let index = serverRooms.firstIndex(where: { $0.id == roomID }) { serverRooms[index] = updated }
        publishRooms()
    }

    /// Creates a private room and returns its id.
    func createRoom(name proposed: String) async throws -> String {
        let name = proposed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { throw SessionError("Enter a room name.") }
        guard name.localizedCaseInsensitiveCompare("lobby") != .orderedSame else {
            throw SessionError("\u{201C}lobby\u{201D} is reserved. Choose another name.")
        }
        guard phase == .connected else { throw SessionError("Reconnect before creating a room.") }
        do {
            let room = try await connection.createRoom(name: name, description: nil, parentID: nil, isPublic: false)
            if !serverRooms.contains(where: { $0.id == room.id }) { serverRooms.append(room) }
            publishRooms()
            return room.id
        } catch CowchatConnectionError.server(_, .some("room_name_taken")) {
            throw SessionError("A room named \(name) already exists.")
        }
    }

    /// A single-use invite token for the room. Account keys can make no other kind.
    func createInvite(roomID: String) async throws -> String {
        try await connection.createInvite(roomID: roomID, singleUse: true)
    }

    /// Where a file downloads from. Only the Global server keeps files, and it cannot read an encrypted room's.
    func attachmentRequest(_ attachment: FileAttachment, roomID: String) -> URLRequest? {
        guard let profile, profile.kind == .cowchatCloud, let socketURL = profile.endpointURL,
              serverRooms.first(where: { $0.id == roomID })?.encrypted != true,
              attachment.blobID.range(of: "^[A-Za-z0-9-]+$", options: .regularExpression) != nil,
              var components = URLComponents(url: socketURL, resolvingAgainstBaseURL: false) else { return nil }
        components.scheme = "https"
        components.path = "/api/blobs/\(attachment.blobID)"
        components.query = nil
        guard let url = components.url else { return nil }
        var request = URLRequest(url: url)
        request.setValue(profile.apiKey, forHTTPHeaderField: "x-cowchat-key")
        request.timeoutInterval = 60
        return request
    }

    func attachmentData(_ attachment: FileAttachment, roomID: String) async throws -> Data {
        guard let request = attachmentRequest(attachment, roomID: roomID) else {
            throw SessionError("This file cannot be opened here.")
        }
        let (data, response) = try await fetchBlob(request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw SessionError("This file is no longer available.")
        }
        return data
    }

    func setShowsAllMessages(_ showsAll: Bool, roomID: String) {
        if showsAll { showAllRoomIDs.insert(roomID) } else { showAllRoomIDs.remove(roomID) }
        storage.showAllRoomIDs = showAllRoomIDs
    }

    /// The unencrypted rooms with a message whose text or sender contains the query, newest 300 messages each.
    func messageMatches(query: String) async -> Set<String> {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty, phase == .connected else { return [] }
        var matches: Set<String> = []
        for room in serverRooms where !room.encrypted {
            var before: String?
            for _ in 0..<3 {
                guard !Task.isCancelled,
                      let page = try? await connection.history(roomID: room.id, limit: 100, before: before) else { break }
                if page.contains(where: {
                    !$0.isThinking && ($0.content.localizedCaseInsensitiveContains(query)
                        || $0.agentName.localizedCaseInsensitiveContains(query))
                }) {
                    matches.insert(room.id)
                    break
                }
                guard page.count == 100, let cutoff = page.first?.timestamp, cutoff != before else { break }
                before = cutoff
            }
        }
        return matches
    }

    private var mentionCache: [String: (activity: String, found: [ChatMessage])] = [:]

    /// Recent messages in unencrypted rooms that mention an account, newest 50 per room. A room is read again only
    /// when its activity has moved on.
    func mentions(of accountID: String) async -> [(roomID: String, roomName: String, message: ChatMessage)] {
        guard phase == .connected else { return [] }
        var all: [(roomID: String, roomName: String, message: ChatMessage)] = []
        for room in serverRooms where !room.encrypted {
            let activity = room.lastActivity ?? room.createdAt
            if mentionCache[room.id]?.activity != activity,
               let page = try? await connection.history(roomID: room.id, limit: pageSize, before: nil) {
                mentionCache[room.id] = (activity, page.filter {
                    $0.metadata.mentions.contains(accountID) && $0.agentID != myAgentID && !$0.isThinking
                })
            }
            all += (mentionCache[room.id]?.found ?? []).map { (room.id, room.name, $0) }
        }
        return all
    }

    /// Rooms this key may join but has not.
    func joinableRooms() async -> [Room] {
        guard phase == .connected else { return [] }
        return (try? await connection.listJoinableRooms()) ?? []
    }

    func join(roomID: String) async throws {
        try await joinIfNeeded(roomID)
        try await loadRooms()
    }

    func setArchived(_ archived: Bool, roomID: String) {
        guard roomID != "lobby" else { return }
        var ids = storage.archivedRoomIDs
        if archived { ids.insert(roomID) } else { ids.remove(roomID) }
        storage.archivedRoomIDs = ids
        publishRooms()
    }

    func open(roomID: String) async {
        let generation = generation
        openRoomIDs.insert(roomID)
        timelines[roomID, default: RoomTimeline()].isLoading = true
        var loaded = false
        do {
            try await joinIfNeeded(roomID)
            try await backfill(roomID: roomID, generation: generation)
            try checkCurrent(generation)
            loaded = true
            await refreshMembers(roomID)
        } catch {}
        // After a drop the reconnect finishes this room's load.
        guard generation == self.generation else { return }
        timelines[roomID]?.isLoading = false
        if loaded { markRead(roomID) }
    }

    func close(roomID: String) {
        openRoomIDs.remove(roomID)
    }

    func loadEarlier(roomID: String) async {
        guard let timeline = timelines[roomID], timeline.hasEarlier, !timeline.isLoading,
              let oldest = timeline.entries.first else { return }
        let generation = generation
        timelines[roomID]?.isLoading = true
        // By sequence, not time: messages that share a timestamp would be skipped at the page's edge.
        let page = try? await connection.history(roomID: roomID, limit: pageSize, beforeSeq: oldest.seq, kinds: [])
        guard generation == self.generation else { return }
        if let page {
            let countBefore = timelines[roomID]?.entries.count ?? 0
            merge(page, into: roomID)
            let gained = (timelines[roomID]?.entries.count ?? 0) > countBefore
            timelines[roomID]?.hasEarlier = page.count >= pageSize && gained
        }
        timelines[roomID]?.isLoading = false
    }

    private func joinIfNeeded(_ roomID: String) async throws {
        guard !joinedRoomIDs.contains(roomID) else { return }
        try await connection.join(roomID: roomID)
        joinedRoomIDs.insert(roomID)
    }

    /// How many pages one catch-up reads before it gives up on joining the old messages to the new.
    private static let catchUpPageLimit = 40

    // Pages back from the newest message, by sequence, until it joins what is already loaded, so time away
    // leaves no hole. The pages are held back until they join up: a catch-up that fails part way changes nothing,
    // so the next one starts from the same place. Where the gap is too long to fill, everything older than what
    // was fetched is let go: the timeline is then one unbroken run ending at the newest message, and Load earlier
    // carries on from its oldest.
    private func backfill(roomID: String, generation: Int) async throws {
        // Not the newest message on screen: one that arrived live after a failed catch-up proves nothing about
        // the messages before it.
        let tip = caughtUpThrough[roomID]
        caughtUp.remove(roomID)
        var staged: [ChatMessage] = []
        var cursor: Int?
        var joined = false
        var firstPageWasFull = false
        for pageIndex in 0..<Self.catchUpPageLimit {
            let page: [ChatMessage]
            do {
                page = try await connection.history(roomID: roomID, limit: pageSize, beforeSeq: cursor, kinds: [])
            } catch {
                // Only the session that asked may note what it learned: after a sign-out the read states are
                // no longer this catch-up's to write.
                if generation == self.generation { noteUnfetched(staged, roomID: roomID) }
                throw error
            }
            // Pages asked for by a session that has since signed out, gone to the background or lost the room are
            // not written: they would rebuild what was just cleared.
            try checkCurrent(generation)
            guard timelines[roomID] != nil else { return }
            staged.insert(contentsOf: page, at: 0)
            if pageIndex == 0 { firstPageWasFull = page.count >= pageSize }
            guard let tip, let oldest = page.first, page.count >= pageSize, oldest.seq > tip + 1 else {
                joined = true
                break
            }
            // A server that returns the same page again has nothing older to give.
            if let cursor, oldest.seq >= cursor {
                joined = true
                break
            }
            cursor = oldest.seq
        }
        if !joined, let floor = staged.first?.seq {
            messages[roomID] = messages[roomID]?.filter { $0.value.seq >= floor }
            timelines[roomID]?.entries.removeAll { $0.seq < floor }
        }
        merge(staged, into: roomID)
        if tip == nil { timelines[roomID]?.hasEarlier = firstPageWasFull }
        if !joined { timelines[roomID]?.hasEarlier = true }
        caughtUpThrough[roomID] = max(tip ?? 0, staged.last?.seq ?? 0, timelines[roomID]?.entries.last?.seq ?? 0)
        caughtUp.insert(roomID)
        // Only now, with nothing missing, may the read position move.
        staged.forEach(noteForReadState)
    }

    /// A catch-up that failed still learned that newer messages exist. The room shows as unread for them, though
    /// none is on screen yet.
    private func noteUnfetched(_ staged: [ChatMessage], roomID: String) {
        guard let newest = staged.last?.seq else { return }
        var state = readStates[roomID] ?? ReadState()
        guard newest > state.latest else { return }
        state.latest = newest
        saveReadState(state, roomID: roomID)
    }

    func refreshPeople(roomID: String) async { await refreshMembers(roomID) }

    private func refreshMembers(_ roomID: String) async {
        guard let agents = try? await connection.listAgents(roomID: roomID) else { return }
        members[roomID] = agents.map { RoomMember(id: $0.agentID, name: $0.name, status: $0.status) }
    }

    private func merge(_ incoming: [ChatMessage], into roomID: String) {
        let visible = incoming.filter { !$0.isThinking }
        for message in visible { messages[roomID, default: [:]][message.id] = message }
        let entries = visible.map { TimelineEntry(message: $0, myAgentID: myAgentID, secret: secrets[roomID]) }
        timelines[roomID] = (timelines[roomID] ?? RoomTimeline()).merging(entries)
    }

    /// Marks everything loaded in the room as read. Only a room with nothing missing: one whose catch-up failed or
    /// was refused keeps its read position, or the messages in its gap would count as read unseen.
    private func markRead(_ roomID: String) {
        guard caughtUp.contains(roomID) else { return }
        var state = readStates[roomID] ?? ReadState()
        state.latest = max(state.latest, timelines[roomID]?.entries.last?.seq ?? 0)
        state.lastRead = state.latest
        saveReadState(state, roomID: roomID)
    }

    private func saveReadState(_ state: ReadState, roomID: String) {
        readStates[roomID] = state
        storage.readStates = readStates
        publishRooms()
    }

    // MARK: Messages

    func send(roomID: String, text: String, mentions: [String]) async throws {
        let content: String
        if let secret = secrets[roomID] {
            content = try RoomCrypto.encrypt(text, secret: secret, roomID: roomID)
        } else if rooms.first(where: { $0.id == roomID })?.isEncrypted == true || storage.expectsSecret(roomID: roomID) {
            throw SessionError("Add this room's secret before sending.")
        } else {
            content = text
        }
        let sent = try await connection.send(roomID: roomID, content: content, mentions: mentions)
        receive(sent)
    }

    private func receive(_ message: ChatMessage) {
        let roomID = message.roomID
        if timelines[roomID] != nil {
            merge([message], into: roomID)
            if caughtUp.contains(roomID) { caughtUpThrough[roomID] = max(caughtUpThrough[roomID] ?? 0, message.seq) }
        }
        if let date = CowchatDate.parse(message.timestamp) {
            liveActivity[roomID] = max(liveActivity[roomID] ?? date, date)
        }
        clearPulse(roomID: roomID, agentID: message.agentID)
        noteForReadState(message)
    }

    /// A message newer than any seen makes its room unread, unless the room is open or the message is this
    /// phone's own or folded away.
    private func noteForReadState(_ message: ChatMessage) {
        let roomID = message.roomID
        var state = readStates[roomID] ?? ReadState()
        guard message.seq > state.latest else { return }
        // A message the room folds away does not make the room unread.
        let isFolded = !MessageVisibility.showsByDefault(message) && !showAllRoomIDs.contains(roomID)
        let wasRead = !state.isUnread
        state.latest = max(state.latest, message.seq)
        // While a loaded room still has a gap to fill, nothing past the gap counts as read, whoever sent it and
        // whether or not the room is on screen. The catch-up that closes the gap moves the read position.
        let hasGap = timelines[roomID] != nil && !caughtUp.contains(roomID)
        if !hasGap, openRoomIDs.contains(roomID) || message.agentID == myAgentID || (isFolded && wasRead) {
            state.lastRead = max(state.lastRead, message.seq)
        }
        saveReadState(state, roomID: roomID)
    }

    /// Saves or removes a room's secret. The room changes only once the Keychain has: a secret it refused is not
    /// used, and one it would not remove stays in use, so the screen never shows what the next launch won't.
    func setSecret(_ secret: String?, roomID: String) throws {
        let secret = secret?.isEmpty == true ? nil : secret
        do {
            try storage.setSecret(secret, roomID: roomID)
        } catch {
            throw SessionError(secret == nil
                ? "Cowchat could not remove this room's secret from this iPhone."
                : "Cowchat could not save this room's secret on this iPhone.")
        }
        secrets[roomID] = secret
        if var timeline = timelines[roomID], let raw = messages[roomID] {
            timeline.entries = timeline.entries.compactMap { entry in
                raw[entry.id].map { TimelineEntry(message: $0, myAgentID: myAgentID, secret: secret) }
            }
            timelines[roomID] = timeline
        }
        publishRooms()
    }

    private func handleEvent(_ type: String, _ payload: [String: Any]) {
        guard phase != .signedOut else { return }
        switch type {
        case "message_received":
            guard let message = decode(ChatMessage.self, payload), !message.isThinking else { return }
            receive(message)
        case "thinking":
            guard let message = decode(ChatMessage.self, payload) else { return }
            let text = message.content.trimmingCharacters(in: .whitespacesAndNewlines)
            pulses[message.roomID, default: [:]][message.agentID] = AgentThinkingPulse(
                agentID: message.agentID, agentName: message.agentName,
                text: text.isEmpty ? nil : text, progress: nil, at: Date()
            )
        case "presence_update":
            updatePresence(payload)
        case "session_state":
            guard let roomID = payload["room_id"] as? String, let state = payload["state"] as? String else { return }
            sessionStates[roomID] = state == "ended" ? nil : state
            // A new session's room may not be listed yet.
            if serverRooms.contains(where: { $0.id == roomID }) { publishRooms() } else { Task { await refreshRooms() } }
        case "room_created":
            guard let room = decode(Room.self, payload), !serverRooms.contains(where: { $0.id == room.id }) else { return }
            serverRooms.append(room)
            publishRooms()
        case "room_updated":
            guard let room = decode(Room.self, payload),
                  let index = serverRooms.firstIndex(where: { $0.id == room.id }) else { return }
            serverRooms[index] = room
            publishRooms()
        case "room_destroyed":
            guard let roomID = payload["room_id"] as? String else { return }
            serverRooms.removeAll { $0.id == roomID }
            forgetRoomState(roomID)
            publishRooms()
        case "agent_joined", "agent_left":
            guard let roomID = payload["room_id"] as? String, openRoomIDs.contains(roomID) else { return }
            Task { await refreshMembers(roomID) }
        default:
            break
        }
    }

    private func decode<T: Decodable>(_ type: T.Type, _ payload: [String: Any]) -> T? {
        (try? JSONSerialization.data(withJSONObject: payload)).flatMap { try? JSONDecoder().decode(type, from: $0) }
    }

    private func forgetRoomState(_ roomID: String) {
        joinedRoomIDs.remove(roomID)
        openRoomIDs.remove(roomID)
        timelines[roomID] = nil
        messages[roomID] = nil
        caughtUpThrough[roomID] = nil
        caughtUp.remove(roomID)
        members[roomID] = nil
        pulses[roomID] = nil
    }

    // The event names no room, so it applies to the open rooms the agent is in.
    private func updatePresence(_ payload: [String: Any]) {
        guard let agentID = payload["agent_id"] as? String, agentID != myAgentID else { return }
        let status = payload["status"] as? String
        for roomID in openRoomIDs {
            guard let index = members[roomID]?.firstIndex(where: { $0.id == agentID }) else { continue }
            members[roomID]?[index].status = status
            guard members[roomID]?[index].isActive == true else {
                clearPulse(roomID: roomID, agentID: agentID)
                continue
            }
            let existing = pulses[roomID]?[agentID]
            let detail = (payload["status_detail"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
            pulses[roomID, default: [:]][agentID] = AgentThinkingPulse(
                agentID: agentID,
                agentName: members[roomID]?[index].name ?? agentID,
                text: (detail?.isEmpty == false ? detail : nil) ?? existing?.text,
                progress: (payload["progress"] as? Int) ?? existing?.progress,
                at: Date()
            )
        }
    }

    private func clearPulse(roomID: String, agentID: String) {
        guard pulses[roomID]?[agentID] != nil else { return }
        pulses[roomID]?[agentID] = nil
        if pulses[roomID]?.isEmpty == true { pulses[roomID] = nil }
    }

    /// Where each live thinking signal sits in the room: under its agent's latest message, or at the end.
    func pulseAnchors(roomID: String, now: Date) -> (byMessageID: [String: [AgentThinkingPulse]], unanchored: [AgentThinkingPulse]) {
        ThinkingPulsePresentation.anchors(
            messages: (messages[roomID] ?? [:]).values.sorted { $0.seq < $1.seq },
            pulses: pulses[roomID] ?? [:],
            currentAgentID: myAgentID ?? "",
            now: now
        )
    }

    // MARK: Reconnect

    func sceneBecameActive() {
        guard phase == .reconnecting || (phase == .connected && !transportUp) else { return }
        startReconnect()
    }

    func sceneEnteredBackground() {
        guard phase == .connected || phase == .reconnecting else { return }
        stopActivity()
        connection.disconnect()
        transportUp = false
        phase = .reconnecting
    }

    private func handleStatus(_ status: ConnectionStatus) {
        switch status {
        case .failed, .disconnected:
            transportUp = false
            if phase == .connected { startReconnect() }
        case .connecting, .connected:
            break
        }
    }

    private func startReconnect() {
        guard let profile else { return }
        stopActivity()
        phase = .reconnecting
        let generation = generation
        reconnectTask = Task { [weak self] in
            await self?.reconnect(profile: profile, generation: generation)
        }
    }

    private func reconnect(profile: ConnectionProfile, generation: Int) async {
        var failures = 0
        while generation == self.generation {
            do {
                try await establish(profile: profile, displayName: displayName, generation: generation)
                for roomID in timelines.keys.sorted() {
                    do {
                        try await joinIfNeeded(roomID)
                        try await backfill(roomID: roomID, generation: generation)
                    } catch CowchatConnectionError.server {
                        // The server refused this one room, say because it is gone. The rest still load.
                    }
                    timelines[roomID]?.isLoading = false
                }
                try checkCurrent(generation)
                openRoomIDs.forEach(markRead)
                phase = .connected
                return
            } catch is CancellationError {
                return
            } catch let error as CowchatConnectionError {
                guard generation == self.generation else { return }
                if case .server = error {
                    connection.disconnect()
                    phase = .failed(Self.message(for: error, profile: profile))
                    return
                }
                do { try await sleep(ReconnectBackoff.delay(afterFailures: failures)) } catch { return }
                failures += 1
            } catch {
                guard generation == self.generation else { return }
                do { try await sleep(ReconnectBackoff.delay(afterFailures: failures)) } catch { return }
                failures += 1
            }
        }
    }
}
