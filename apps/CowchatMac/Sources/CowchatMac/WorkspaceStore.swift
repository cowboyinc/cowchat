import Combine
import Foundation

enum GlobalSignupError: LocalizedError, Equatable {
    case signupClosed
    case rateLimited
    case malformedResponse
    case serverError(Int)

    var errorDescription: String? {
        switch self {
        case .signupClosed:
            return "This server doesn't hand out keys automatically. Paste an API key instead."
        case .rateLimited:
            return "The server is limiting new keys from your network. Try again later or paste an existing API key."
        case .malformedResponse:
            return "The server's signup reply was malformed."
        case let .serverError(status):
            return "The server could not create an API key (HTTP \(status))."
        }
    }
}

/// Keeps local, shared global, and account global connections independent.
/// Room identifiers are scoped to the store that owns them.
@MainActor
final class WorkspaceStore: ObservableObject {
    enum Server: String, CaseIterable, Identifiable {
        case local
        case global
        case accountGlobal

        var id: String { rawValue }

        var label: String {
            switch self {
            case .local: return "Local"
            case .global: return "Global rooms · API key"
            case .accountGlobal: return "Global rooms · account"
            }
        }
    }

    /// What the window's main area shows instead of a room: the Inbox or the Team page, chosen from the sidebar, or the
    /// Friends page, from the toolbar's people button.
    enum Page: Equatable {
        case inbox
        case team
        case friends
    }

    let local: ChatStore
    let notifications: SocialNotifications
    /// The person's live sessions, each with its room, read from the account's rooms and `session_state`.
    let liveSessions = LiveSessions()
    let inbox: InboxStore
    let team = TeamStore()
    /// The page the main area shows, or nil while it shows the open room.
    @Published private(set) var page: Page?
    /// Where first run stands, and its wait for the first session to register.
    lazy var firstRun: FirstRun = {
        let firstRun = FirstRun(sessions: liveSessions, defaults: defaults)
        firstRun.onFirstSession = { [weak self] session in Task { await self?.finishFirstRun(opening: session) } }
        return firstRun
    }()
    /// Claude Code's plugin and Codex's hooks, as first run and Settings show them.
    let agentConnections = AgentConnections()
    /// The account's teams and the one chosen in the sidebar's team row, which picks the team the Team page shows and the
    /// company room New room makes a room in.
    lazy var teams: TeamChoice = {
        let teams = TeamChoice(defaults: defaults)
        teams.onChoiceChanged = { [weak self] in
            guard let self else { return }
            self.team.use(teamID: teams.chosen?.id)
            Task { await self.refreshPages() }
        }
        return teams
    }()
    /// New room with an account room's draft, as a friend's row on the Friends page opens it.
    @Published var roomDraft: SocialRoomDraft?
    /// A room's people, as its menu's People… shows them.
    @Published var roomPeople: SocialParticipants?
    /// The session rooms the archive rule is asking the server about now, so a room is asked about once at a time.
    private var sessionRoomArchiveChecks: Set<String> = []
    private var sessionRoomArchiveScheduled = false
    private var watchingAccount: Set<AnyCancellable> = []
    @Published private(set) var global: ChatStore?
    @Published private(set) var accountGlobal: ChatStore?
    @Published private(set) var activeServer: Server = .local
    /// Inline error for the global-server settings card. Connection failures
    /// live on the store; this only carries save/enable problems.
    @Published var globalSetupError: String?
    /// The sidebar's filter. It is not kept, so the app starts on All, and it goes back to All on sign out when the
    /// signed-out sidebar does not offer it.
    @Published var sidebarFilter = SidebarFilter.all
    @Published var searchText = "" {
        didSet {
            local.searchText = searchText
            global?.searchText = searchText
            accountGlobal?.searchText = searchText
        }
    }

    private let preferences: ConnectionProfilePreferences
    private let socialCredentials: any CowchatCredentialStore
    private let defaults: UserDefaults
    private let requestAPIKey: (URL) async throws -> String
    private let makeAccountStore: (ConnectionProfile) -> ChatStore

    lazy var social: SocialStore = {
        let endpoint = URL(string: configuredGlobalValues().url) ?? URL(string: ConnectionProfile.defaultGlobalURLString)!
        let social = SocialStore(endpoint: endpoint, credentials: socialCredentials)
        social.onSessionChanged = { [weak self] session in
            self?.notifications.setSession(session)
            self?.useSocialSession(session)
        }
        social.onSnapshot = { [weak self] snapshot in
            self?.notifications.receive(snapshot)
            Task { await self?.refreshPages() }
        }
        social.onRoomsChanged = { [weak self] in
            guard let self, self.social.savedSession != nil else { return }
            Task { try? await self.accountGlobal?.refreshRooms() }
        }
        social.$rooms.dropFirst().sink { [weak self] rooms in
            guard let self else { return }
            self.liveSessions.update(from: rooms, viewerID: self.social.account?.id)
            self.scheduleSessionRoomArchive()
        }
        .store(in: &self.watchingSocial)
        return social
    }()

    private var watchingSocial: Set<AnyCancellable> = []

    @Published private(set) var pendingFriendInvitation: FriendInvitationRoute?
    private static let pendingFriendInvitationKey = "CowchatMac.pendingFriendInvitation"

    func receiveFriendInvitation(_ url: URL) {
        guard let invitation = FriendInvitationRoute.parse(url) else { return }
        pendingFriendInvitation = invitation
        defaults.set(try? JSONEncoder().encode(invitation), forKey: Self.pendingFriendInvitationKey)
    }

    func clearFriendInvitation(_ invitation: FriendInvitationRoute) {
        guard pendingFriendInvitation == invitation else { return }
        pendingFriendInvitation = nil
        defaults.removeObject(forKey: Self.pendingFriendInvitationKey)
    }

    func useSocialSession(_ session: SocialSession?) {
        accountGlobal?.shutdownForRemoval()
        accountGlobal = nil
        watchingAccount = []
        liveSessions.clear()
        inbox.use(account: session?.account, scope: session.map(Self.accountScope))
        team.use(accountID: session?.account.id)
        // Signed in, the app opens on the Inbox, which gathers what needs the person across rooms (Patrick, 2
        // November). The account store still selects a room underneath, which shows when its cell is pressed.
        page = session == nil ? nil : .inbox
        if !SidebarFilter.offered(signedIn: session != nil).contains(sidebarFilter) { sidebarFilter = .all }
        if let session, let profile = try? ConnectionProfile.socialSession(session) {
            let store = makeAccountStore(profile)
            store.searchText = searchText
            accountGlobal = store
            watchSessions(on: store)
            store.start()
            liveSessions.update(from: social.rooms, viewerID: session.account.id)
        }
        if activeServer == .accountGlobal, accountGlobal == nil {
            // Signed out, the window opens on this Mac's lobby, whose chat the sidebar lists. A hosted lobby is hidden.
            activeServer = .local
        }
    }

    // MARK: Sessions and their rooms

    /// The account room made for one of the person's live sessions, once the account's rooms list it.
    func room(of session: LiveSession) -> Room? {
        accountGlobal?.rooms.first { $0.id == session.roomID }
    }

    /// The name of one of the person's own sessions, live or ended: its room's name, which the Inbox's titles use.
    func ownSessionName(of agentID: String) -> String? {
        guard let viewer = social.account?.id else { return nil }
        return social.rooms.first { $0.ownerId == viewer && $0.session?.agentId == agentID }?.name
    }

    /// Opens a session's room, as first run and New session do when the session registers. Nothing is drafted.
    func openSession(_ session: LiveSession) async {
        guard let store = accountGlobal else { return }
        if !store.rooms.contains(where: { $0.id == session.roomID }) { try? await store.refreshRooms() }
        guard accountGlobal === store, let room = store.rooms.first(where: { $0.id == session.roomID }) else { return }
        if !isSelected(room, on: .accountGlobal) { await select(room: room, on: .accountGlobal) }
    }

    /// Hands the account store's `session_state` frames to the live sessions, lists the rooms again when one names a
    /// room the account has not listed, and runs the archive rule on each frame and when the rooms change.
    private func watchSessions(on store: ChatStore) {
        store.onSessionState = { [weak self, weak store] event in
            guard let self, let store, self.accountGlobal === store else { return }
            if !self.liveSessions.apply(event) { Task { await self.social.refreshRooms() } }
            // An ended session's room may archive, and a resumed one's may come back.
            self.scheduleSessionRoomArchive()
        }
        store.$rooms.map(\.count).removeDuplicates().sink { [weak self] _ in self?.scheduleSessionRoomArchive() }
            .store(in: &watchingAccount)
    }

    /// Runs the archive rule on the next turn, once however many changes ask for it.
    func scheduleSessionRoomArchive() {
        guard !sessionRoomArchiveScheduled else { return }
        sessionRoomArchiveScheduled = true
        Task { [weak self] in
            guard let self else { return }
            self.sessionRoomArchiveScheduled = false
            await self.applySessionRoomArchive()
        }
    }

    /// The archive rule for ended sessions (`SessionRoomArchive`): a resumed session's room comes back, and each ended
    /// session's room the rule has not handled, or that waits to be read, is checked against its participants. A
    /// failed participants call leaves the room for the next run.
    func applySessionRoomArchive() async {
        guard let store = accountGlobal, store.connectionStatus.isConnected, let viewer = social.account?.id else { return }
        let outcomes = store.sessionRoomArchiveOutcomes
        for roomID in SessionRoomArchive.resumed(social.rooms, viewerID: viewer, hasEnded: liveSessions.hasEnded,
                                                 outcomes: outcomes) {
            if outcomes[roomID] == .archived, let room = store.rooms.first(where: { $0.id == roomID }) {
                store.unarchive(room)
            }
            store.recordSessionRoomArchive(nil, roomID: roomID)
        }
        let candidates = SessionRoomArchive.roomsToCheck(social.rooms, viewerID: viewer, hasEnded: liveSessions.hasEnded,
                                                         outcomes: store.sessionRoomArchiveOutcomes)
        for candidate in candidates where !sessionRoomArchiveChecks.contains(candidate.roomId) {
            guard let room = store.rooms.first(where: { $0.id == candidate.roomId }) else { continue }
            if store.isArchived(room) {
                store.recordSessionRoomArchive(.archived, roomID: room.id)
                continue
            }
            if store.sessionRoomArchiveOutcomes[room.id] == .waiting,
               !SessionRoomArchive.isReady(isUnread: store.isUnread(room), isOpen: isSelected(room, on: .accountGlobal)) {
                continue
            }
            sessionRoomArchiveChecks.insert(room.id)
            defer { sessionRoomArchiveChecks.remove(room.id) }
            guard let api = try? social.readingAPI(),
                  let participants: SocialParticipants = try? await api.social(["rooms", room.id, "participants"]),
                  accountGlobal === store, social.account?.id == viewer else { continue }
            let decision = SessionRoomArchive.decide(
                hasSomeoneElse: SessionRoomArchive.hasSomeoneElse(members: participants.members, viewerID: viewer,
                                                                  roomID: room.id,
                                                                  sentInvitations: social.sentInvitations),
                isUnread: store.isUnread(room), isOpen: isSelected(room, on: .accountGlobal))
            switch decision {
            case .keep: store.recordSessionRoomArchive(.kept, roomID: room.id)
            case .wait: store.recordSessionRoomArchive(.waiting, roomID: room.id)
            case .archive:
                await store.archive(room)
                store.recordSessionRoomArchive(.archived, roomID: room.id)
            }
        }
    }

    var activeStore: ChatStore {
        switch activeServer {
        case .local: return local
        case .global: return global ?? local
        case .accountGlobal: return accountGlobal ?? local
        }
    }

    /// First run's Continue, which skips the wait for the first session: the account's rooms are listed again and the
    /// Inbox shows. Returns false when the account changed or its rooms could not be listed.
    func openStarter(session: SocialSession) async -> Bool {
        guard social.savedSession?.scopeId == session.scopeId,
              social.savedSession?.deviceKey == session.deviceKey,
              let store = accountGlobal else { return false }
        do { try await store.refreshRooms() } catch { return false }
        guard social.savedSession?.scopeId == session.scopeId,
              social.savedSession?.deviceKey == session.deviceKey,
              accountGlobal === store else { return false }
        await social.refreshRooms()
        show(.inbox)
        return social.savedSession?.scopeId == session.scopeId && accountGlobal === store
    }

    func openCreatedSocialRoom(roomID: String) async -> Bool {
        guard let store = accountGlobal else { return false }
        do { try await store.refreshRooms() } catch { return false }
        guard let room = store.rooms.first(where: { $0.id == roomID }) else { return false }
        await select(room: room, on: .accountGlobal)
        return true
    }

    var isGlobalEnabled: Bool { global != nil }

    init(
        local: ChatStore,
        preferences: ConnectionProfilePreferences,
        defaults: UserDefaults = .standard,
        global: ChatStore? = nil,
        accountGlobal: ChatStore? = nil,
        makeAccountStore: ((ConnectionProfile) -> ChatStore)? = nil,
        socialCredentials: any CowchatCredentialStore = KeychainCowchatCredentialStore(),
        requestAPIKey: @escaping (URL) async throws -> String = WorkspaceStore.requestAPIKeyOverHTTP
    ) {
        self.local = local
        self.notifications = SocialNotifications(defaults: defaults)
        self.inbox = InboxStore(defaults: defaults)
        self.preferences = preferences
        self.socialCredentials = socialCredentials
        self.defaults = defaults
        if let data = defaults.data(forKey: Self.pendingFriendInvitationKey),
           let pending = try? JSONDecoder().decode(FriendInvitationRoute.self, from: data),
           FriendInvitationRoute.validSlug(pending.slug), FriendInvitationRoute.normalizeEndpoint(pending.endpoint) != nil {
            pendingFriendInvitation = pending
        }
        self.global = global
        self.accountGlobal = accountGlobal
        self.makeAccountStore = makeAccountStore ?? { profile in
            ChatStore(connection: CowchatConnection(profile: profile), defaults: defaults, connectionProfile: profile)
        }
        self.requestAPIKey = requestAPIKey
    }

    convenience init() {
        let defaults = UserDefaults.standard
        let preferences = ConnectionProfilePreferences(
            defaults: defaults,
            credentialStore: KeychainCowchatCredentialStore()
        )
        #if DEBUG
        // Dev builds run as a bare `swift build` binary with no
        // Contents/Helpers, so allow the fixed Homebrew install locations as
        // helper fallbacks. Deliberately empty in release: the shipped app
        // only ever spawns the helper it was signed with.
        let devFallbackServerPaths = [
            "/opt/homebrew/bin/cowchat-server",
            "/usr/local/bin/cowchat-server",
        ]
        #else
        let devFallbackServerPaths: [String] = []
        #endif
        let local = ChatStore(
            connection: CowchatConnection(profile: .local),
            defaults: defaults,
            connectionProfile: .local,
            localServerSupervisor: LocalServerSupervisor(
                devFallbackServerPaths: devFallbackServerPaths
            )
        )
        var global: ChatStore?
        if preferences.isGlobalEnabled() {
            global = Self.makeGlobalStore(preferences: preferences, defaults: defaults)
            if global == nil {
                // Enabled but nothing stored at all: an interrupted first-time
                // setup. Fall back to off rather than showing a broken server.
                preferences.setGlobalEnabled(false)
            }
        }
        self.init(local: local, preferences: preferences, defaults: defaults, global: global)
    }

    func start() {
        local.start()
        notifications.onOpenInboxItem = { [weak self] id in Task { await self?.openInboxItem(id: id) } }
        notifications.setSession(social.savedSession)
        social.startMonitoring()
        global?.start()
        if let session = social.savedSession { useSocialSession(session) }
    }

    func shutdownForAppTermination() async {
        social.stopMonitoring()
        await local.shutdownOwnedLocalServerForAppTermination()
    }

    func store(for server: Server) -> ChatStore? {
        switch server {
        case .local: return local
        case .global: return global
        case .accountGlobal: return accountGlobal
        }
    }

    func isSelected(_ room: Room, on server: Server) -> Bool {
        page == nil && activeServer == server && store(for: server)?.selectedRoomID == room.id
    }

    /// Makes a server's selection the one the chat pane shows (e.g. after
    /// creating a room on the non-active server).
    func activate(server: Server) {
        guard store(for: server) != nil else { return }
        activeServer = server
    }

    func select(room: Room, on server: Server) async {
        guard let store = store(for: server) else { return }
        page = nil
        activeServer = server
        // A room inside a closed company room opens its parent first, so its cell shows selected.
        var parentID = room.parentID
        var seen: Set<String> = [room.id]
        while let id = parentID, seen.insert(id).inserted {
            store.setCollapsed(false, roomID: id)
            parentID = store.rooms.first { $0.id == id }?.parentID
        }
        await store.select(room: room)
        // The room that was open may be an ended session's, which archives once something else opens.
        scheduleSessionRoomArchive()
    }

    // MARK: The sidebar

    /// Every room of the three connections as the sidebar's list holds them, each with its live session.
    var sidebarRooms: [SidebarRoom] {
        Server.allCases.flatMap { server -> [SidebarRoom] in
            guard let store = store(for: server) else { return [] }
            return RoomSidebarPresentation.listedRooms(store.rooms, on: server).map { room in
                SidebarRoom(server: server, room: store.roomAsShown(room),
                            session: server == .accountGlobal ? liveSessions.session(inRoom: room.id) : nil,
                            isArchived: store.isArchived(room), isUnread: store.isUnread(room),
                            matchesMessages: store.messageSearchRoomIDs.contains(room.id))
            }
        }
    }

    /// The rooms the sidebar's search offers under To join: the Inbox's open invitations, and the rooms the API key may
    /// join and the person is not in.
    var sidebarJoinable: [SidebarJoinable] {
        let invitations = inbox.items.filter(\.isOpenInvitation).map {
            SidebarJoinable(name: $0.roomName, server: $0.server, at: $0.at, source: .invitation(itemID: $0.id))
        }
        let rooms = (global?.joinableRooms ?? []).map {
            SidebarJoinable(name: $0.name, description: $0.description, server: .global, at: $0.activityDate,
                            source: .room($0))
        }
        return invitations + rooms
    }

    /// The closed company rooms of the three connections, by their list IDs.
    var sidebarCollapsed: Set<String> {
        Set(Server.allCases.flatMap { server in
            (store(for: server)?.collapsedParentRoomIDs ?? []).map { SidebarRoom.id($0, on: server) }
        })
    }

    /// Opens a room the API key may join and the person is not in, from the sidebar search's To join group. Opening it
    /// joins it.
    func openJoinable(_ room: Room) async {
        guard let store = global else { return }
        page = nil
        activeServer = .global
        await store.openJoinable(room)
        scheduleSessionRoomArchive()
    }

    /// Opens the Join sheet for an open invitation, from the sidebar search's To join group. The sheet is the Inbox's,
    /// so the Inbox shows under it.
    func openInvitation(itemID: String) {
        guard let item = inbox.items.first(where: { $0.id == itemID && $0.isOpenInvitation }) else { return }
        show(.inbox)
        inbox.joining = item
    }

    // MARK: The Inbox and Team

    /// Shows a page in the main area in place of the room, which stays open under it.
    func show(_ page: Page) {
        self.page = page
        scheduleSessionRoomArchive()
    }

    /// The scope the Inbox keeps its read state under: the server and the account, as the social notifications'.
    nonisolated static func accountScope(of session: SocialSession) -> String {
        SocialNotificationSnapshot(session: session, requests: [], invitations: []).accountScope
    }

    /// Polls the Team page's and the Inbox's endpoints, as the friends are polled, then hands the Inbox's new items to
    /// the Mac's notifications. Team goes first, since it says whose a community room's session is.
    func refreshPages() async {
        guard let session = social.savedSession else { return }
        team.use(teamID: teams.chosen?.id)
        await team.refresh(social: social)
        let people = social.friends.map(\.account) + [session.account]
        await inbox.refresh(social: social, community: global, people: people) { [team] in team.owner(of: $0) }
        guard social.savedSession?.account.id == session.account.id, inbox.isLoaded else { return }
        notifications.receiveInbox(inbox.items, session: session) { [weak self] item in
            item.title { id in self?.ownSessionName(of: id) }
        }
    }

    /// Pressing an Inbox message, or an answered invitation: its room opens at its latest messages, and only then is it
    /// read. When the room cannot be opened the item stays unread, and the window's alert says so, as it does for the
    /// app's other failures.
    func open(_ item: InboxItem) async {
        guard let store = store(for: item.server) else {
            // copy:
            activeStore.errorMessage = "Reconnect before opening this room."
            return
        }
        if !store.rooms.contains(where: { $0.id == item.roomID }) { try? await store.refreshRooms() }
        guard let room = store.rooms.first(where: { $0.id == item.roomID }) else {
            // copy:
            activeStore.errorMessage = "This room can't be opened. You may no longer be in it."
            return
        }
        await select(room: room, on: item.server)
        if isSelected(room, on: item.server) { inbox.markRead(item) }
    }

    /// A click on an Inbox message's notification: the message's room opens, as pressing its row does. An item the
    /// Inbox no longer holds, or an open invitation, opens the Inbox.
    func openInboxItem(id: String) async {
        guard let item = inbox.items.first(where: { $0.id == id }), !item.isOpenInvitation else {
            show(.inbox)
            return
        }
        await open(item)
    }

    /// Join in the Join sheet: accepts the invitation with the chosen session, or none, then opens the room. The item
    /// is read once it is answered.
    @discardableResult
    func accept(_ item: InboxItem, with session: LiveSession?) async -> Bool {
        guard let invitationID = item.invitationID else { return false }
        var accepted = false
        await social.perform { api in
            struct Reply: Decodable { let roomId: String }
            let _: Reply = try await api.social(["room-invitations", invitationID, "accept"], method: "POST",
                                                 body: ["agent_ids": session.map { [$0.id] } ?? []])
            accepted = true
        }
        guard accepted else { return false }
        // The Join sheet closes once the server has the answer. Left open while the room loads, it came back over the
        // next page the window showed.
        if inbox.joining?.id == item.id { inbox.joining = nil }
        inbox.markRead(item)
        await refreshPages()
        await open(item)
        return true
    }

    /// Decline, in the Join sheet or the row's menu: the invitation is declined and its row leaves the Inbox.
    @discardableResult
    func decline(_ item: InboxItem) async -> Bool {
        guard let invitationID = item.invitationID else { return false }
        var declined = false
        await social.perform { api in
            let _: SocialAcknowledgement = try await api.social(["room-invitations", invitationID, "decline"],
                                                                method: "POST")
            declined = true
        }
        guard declined else { return false }
        inbox.remove(item)
        await refreshPages()
        return true
    }

    // MARK: Friends and New room

    /// A draft for an account room, made inside the chosen team's company room when the account has teams.
    func newRoomDraft(friendIDs: [String] = []) -> SocialRoomDraft {
        SocialRoomDraft(friendIDs: friendIDs, parentID: teams.chosen?.id)
    }

    /// Pressing a friend's row on the Friends page: New room with that friend picked.
    func newRoom(with friend: SocialFriend) {
        roomDraft = newRoomDraft(friendIDs: [friend.id])
    }

    /// People… in an account room's menu: the room's people and agents from the server.
    func showPeople(of roomID: String) async {
        await social.perform { api in
            let people: SocialParticipants = try await api.social(["rooms", roomID, "participants"])
            self.roomPeople = people
        }
    }

    /// Pressing a Team row that opens a room: the room its branch's newest session works in, on the account.
    func open(_ branch: TeamBranch) async {
        guard let roomID = branch.room?.roomId, let store = accountGlobal else { return }
        if !store.rooms.contains(where: { $0.id == roomID }) { try? await store.refreshRooms() }
        guard let room = store.rooms.first(where: { $0.id == roomID }) else { return }
        await select(room: room, on: .accountGlobal)
    }

    /// Saved global config for prefilling the settings fields. Falls back to
    /// Cowboy's well-known server when nothing is configured yet.
    func configuredGlobalValues() -> (url: String, apiKey: String) {
        do {
            guard let profile = try preferences.loadConfiguredCloudProfile() else {
                return (savedOrDefaultGlobalURL(), "")
            }
            return (profile.endpointURL?.absoluteString ?? savedOrDefaultGlobalURL(), profile.apiKey)
        } catch {
            return (savedOrDefaultGlobalURL(), "")
        }
    }

    /// Connects to a global server, minting an API key from its self-serve
    /// signup endpoint when the user left the key field empty.
    @discardableResult
    func connectToGlobal(url: String, apiKey: String) async -> Bool {
        let trimmedKey = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmedKey.isEmpty else {
            return saveGlobalConfiguration(url: url, apiKey: trimmedKey)
        }
        globalSetupError = nil
        guard let signupURL = Self.signupURL(forCloudURLString: url) else {
            globalSetupError = ConnectionProfileError.invalidCloudURL.localizedDescription
            return false
        }
        do {
            let mintedKey = try await requestAPIKey(signupURL)
            return saveGlobalConfiguration(url: url, apiKey: mintedKey)
        } catch {
            globalSetupError = error.localizedDescription
            return false
        }
    }

    /// `wss://host[:port]/…` → `https://host[:port]/api/keys`.
    nonisolated static func signupURL(forCloudURLString urlString: String) -> URL? {
        apiURL(forCloudURLString: urlString, path: "/api/keys")
    }

    /// `wss://host[:port]/…` → `https://host[:port]/api/invites/redeem`.
    nonisolated static func inviteRedeemURL(forCloudURLString urlString: String) -> URL? {
        apiURL(forCloudURLString: urlString, path: "/api/invites/redeem")
    }

    /// `wss://host[:port]/…` → `https://host[:port]/api/blobs/<id>`.
    nonisolated static func blobURL(forCloudURLString urlString: String, blobID: String) -> URL? {
        guard blobID.range(of: "^[A-Za-z0-9-]+$", options: .regularExpression) != nil else {
            return nil
        }
        return apiURL(forCloudURLString: urlString, path: "/api/blobs/\(blobID)")
    }

    private nonisolated static func apiURL(
        forCloudURLString urlString: String,
        path: String
    ) -> URL? {
        guard let profile = try? ConnectionProfile.cowchatCloud(
            urlString: urlString,
            apiKey: "placeholder-for-validation"
        ), let endpointURL = profile.endpointURL,
            var components = URLComponents(url: endpointURL, resolvingAgainstBaseURL: false)
        else { return nil }
        components.scheme = "https"
        components.path = path
        return components.url
    }

    nonisolated static func requestAPIKeyOverHTTP(_ signupURL: URL) async throws -> String {
        var request = URLRequest(url: signupURL)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: ["label": "Cowchat Mac"])
        request.timeoutInterval = 15
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        switch status {
        case 201:
            guard let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let key = payload["api_key"] as? String, !key.isEmpty else {
                throw GlobalSignupError.malformedResponse
            }
            return key
        case 401, 403:
            throw GlobalSignupError.signupClosed
        case 429:
            throw GlobalSignupError.rateLimited
        default:
            throw GlobalSignupError.serverError(status)
        }
    }

    @discardableResult
    func saveGlobalConfiguration(url: String, apiKey: String) -> Bool {
        globalSetupError = nil
        do {
            let candidate = try ConnectionProfile.cowchatCloud(urlString: url, apiKey: apiKey)
            if let session = social.savedSession, candidate.endpointURL != session.endpoint {
                globalSetupError = "Sign out of your Cowchat account before switching servers. Your shared global connection is unchanged."
                return false
            }
            if let global {
                guard global.saveAndUseCowchatCloud(url: url, apiKey: apiKey) else {
                    globalSetupError = global.errorMessage
                    return false
                }
                social.configureEndpoint(candidate.endpointURL!)
                return true
            }
            let saved = try preferences.save(candidate)
            attachGlobalStore(profile: saved, configurationError: nil)
            social.configureEndpoint(saved.endpointURL!)
            return true
        } catch {
            globalSetupError = error.localizedDescription
            return false
        }
    }

    func disableGlobalRooms() {
        globalSetupError = nil
        preferences.setGlobalEnabled(false)
        if activeServer == .global { activeServer = accountGlobal == nil ? .local : .accountGlobal }
        global?.shutdownForRemoval()
        global = nil
    }

    private func attachGlobalStore(profile: ConnectionProfile, configurationError: Error?) {
        let store = ChatStore(
            connection: CowchatConnection(profile: profile),
            defaults: defaults,
            connectionProfile: profile,
            connectionPreferences: preferences,
            connectionConfigurationError: configurationError
        )
        global = store
        store.searchText = searchText
        store.start()
    }

    private static func makeGlobalStore(
        preferences: ConnectionProfilePreferences,
        defaults: UserDefaults
    ) -> ChatStore? {
        let profile: ConnectionProfile
        let configurationError: Error?
        do {
            guard let loaded = try preferences.loadConfiguredCloudProfile() else { return nil }
            profile = loaded
            configurationError = nil
        } catch {
            // Keep the server visible with its saved endpoint; reconnect is
            // the user-approved point to retry the Keychain read.
            configurationError = error
            profile = .unavailableCowchatCloud(urlString: preferences.loadSavedCloudURL())
        }
        return ChatStore(
            connection: CowchatConnection(profile: profile),
            defaults: defaults,
            connectionProfile: profile,
            connectionPreferences: preferences,
            connectionConfigurationError: configurationError
        )
    }

    private func savedOrDefaultGlobalURL() -> String {
        preferences.loadSavedCloudURL() ?? ConnectionProfile.defaultGlobalURLString
    }

}
