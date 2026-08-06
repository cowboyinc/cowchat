import Foundation

private struct MessageSearchContext: Equatable {
    let query: String
    let roomVersions: [String]
}

private struct FailedDraftRestoration {
    let generation: Int
    let content: String
}

@MainActor
final class ChatStore: ObservableObject {
    @Published var rooms: [Room] = []
    @Published var selectedRoomID: String?
    @Published var messages: [ChatMessage] = []
    @Published var roomMembers: [AgentPresence] = []
    @Published var draft = ""
    @Published var searchText = "" {
        didSet { scheduleMessageSearch(restartInFlight: true) }
    }
    @Published private(set) var messageSearchRoomIDs: Set<String> = []
    @Published private(set) var archivedRoomIDs: Set<String> = []
    @Published private(set) var setupRoomIDs: Set<String> = []
    @Published private(set) var roomSetupScreenIDs: Set<String> = []
    @Published private(set) var readState = RoomReadState()
    @Published private(set) var isSearchingMessages = false
    @Published private(set) var roomMessagePreviews: [String: String] = [:]
    @Published private(set) var lastThinkingAt: [String: [String: Date]] = [:]
    @Published var roomReadyNotice: Room?
    @Published var roomBeingRenamed: Room?
    @Published var connectionStatus: ConnectionStatus = .disconnected
    @Published private(set) var connectionProfile: ConnectionProfile
    @Published var errorMessage: String?
    @Published var isLoadingMessages = false
    @Published var isCreateRoomPresented = false
    @Published var createRoomParentID: String?

    private let connection: any CowchatConnectionProtocol
    private let connectionPreferences: ConnectionProfilePreferences?
    private let localServerSupervisor: (any LocalServerSupervising)?
    private let localServerRetryDelaysNanoseconds: [UInt64]
    private let defaults: UserDefaults
    private var connectionConfigurationError: Error?
    private var connectionAttemptTask: Task<Void, Never>?
    private var connectionAttemptGeneration = 0
    private var profileGeneration = 0
    private var reconnectTask: Task<Void, Never>?
    private var roomRefreshTask: Task<Void, Never>?
    private var roomLoadTask: Task<Void, Never>?
    private var messageSearchTask: Task<Void, Never>?
    private var setupReadinessTask: Task<Void, Never>?
    private var roomPreviewTask: Task<Void, Never>?
    private var messageSearchGeneration = 0
    private var activeMessageSearchContext: MessageSearchContext?
    private var completedMessageSearchContext: MessageSearchContext?
    private var setupReadinessGeneration = 0
    private var joinedRoomID: String?
    private var roomSelectionGeneration = 0
    private var roomMutationGeneration = 0
    private var roomMutationGenerationByID: [String: Int] = [:]
    private var isRefreshingRooms = false
    private var pendingDestructionRoomIDs: Set<String> = []
    private var confirmedDestructionRoomIDs: Set<String> = []
    private var destroyedRoomIDs: Set<String> = []
    private var draftsByRoomID: [String: String] = [:]
    private var failedDraftRestorationsByRoomID: [String: FailedDraftRestoration] = [:]
    private var sendGeneration = 0
    private var previewActivityByRoomID: [String: String] = [:]
    private(set) var agentID = ""
    let agentName = "Cowchat Mac"
    private var stableAgentID: String
    private var localPreferences: RoomLocalPreferences

    var selectedRoom: Room? {
        rooms.first { $0.roomID == selectedRoomID }
    }

    var filteredRooms: [Room] {
        RoomSidebarPresentation.filteredRooms(
            from: rooms,
            query: searchText,
            matchingMessageRoomIDs: messageSearchRoomIDs
        )
    }

    var unarchivedRooms: [Room] {
        rooms.filter { !archivedRoomIDs.contains($0.id) }
    }

    var archivedRooms: [Room] {
        rooms.filter { archivedRoomIDs.contains($0.id) }
    }

    static func resolveAgentID(defaults: UserDefaults, scope: String? = nil) -> String {
        let key = scope.map { "CowchatMac.agentID.\($0)" } ?? "CowchatMac.agentID"
        if let existing = defaults.string(forKey: key), !existing.isEmpty {
            return existing
        }
        let generated = "cowchat-mac-\(UUID().uuidString.lowercased())"
        defaults.set(generated, forKey: key)
        return generated
    }

    convenience init() {
        let credentialStore = KeychainCowchatCredentialStore()
        let preferences = ConnectionProfilePreferences(
            defaults: .standard,
            credentialStore: credentialStore
        )
        let profile: ConnectionProfile
        let configurationError: Error?
        do {
            profile = try preferences.loadSelectedProfile()
            configurationError = nil
        } catch {
            configurationError = error
            if (try? preferences.loadSelectedKind()) == .cowchatCloud {
                profile = .unavailableCowchatCloud(
                    urlString: preferences.loadSavedCloudURL()
                )
            } else {
                profile = .local
            }
        }
        self.init(
            connection: CowchatConnection(profile: profile),
            defaults: .standard,
            connectionProfile: profile,
            connectionPreferences: preferences,
            localServerSupervisor: LocalServerSupervisor(),
            connectionConfigurationError: configurationError
        )
    }

    init(
        connection: any CowchatConnectionProtocol,
        defaults: UserDefaults = .standard,
        connectionProfile: ConnectionProfile = .local,
        connectionPreferences: ConnectionProfilePreferences? = nil,
        localServerSupervisor: (any LocalServerSupervising)? = nil,
        connectionConfigurationError: Error? = nil,
        localServerRetryDelaysNanoseconds: [UInt64] = [
            50_000_000,
            100_000_000,
            200_000_000,
            400_000_000,
            800_000_000,
            1_600_000_000,
        ]
    ) {
        self.connection = connection
        self.connectionProfile = connectionProfile
        self.connectionPreferences = connectionPreferences
        self.localServerSupervisor = localServerSupervisor
        self.connectionConfigurationError = connectionConfigurationError
        self.localServerRetryDelaysNanoseconds = localServerRetryDelaysNanoseconds
        self.defaults = defaults
        let hadExistingAgentID = !(defaults.string(forKey: "CowchatMac.agentID") ?? "").isEmpty
        CowchatOnboarding.migrateExistingUser(
            defaults: defaults,
            hadExistingAgentID: hadExistingAgentID
        )
        stableAgentID = Self.resolveAgentID(
            defaults: defaults,
            scope: connectionProfile.persistentIdentityScope
        )
        localPreferences = Self.roomPreferences(defaults: defaults, profile: connectionProfile)
        archivedRoomIDs = localPreferences.archivedRoomIDs
        setupRoomIDs = localPreferences.pendingSetupRoomIDs
        roomSetupScreenIDs = localPreferences.pendingSetupScreenRoomIDs
        readState = localPreferences.roomReadState ?? RoomReadState()
        connection.onEvent = { [weak self] type, payload in
            self?.handleEvent(type: type, payload: payload)
        }
        connection.onStatusChange = { [weak self] status in
            self?.handleConnectionStatus(status)
        }
        connection.reconfigure(profile: connectionProfile)
        if let connectionConfigurationError {
            let message = connectionConfigurationError.localizedDescription
            errorMessage = message
            connectionStatus = .failed(message)
        }
    }

    func start() {
        if let connectionConfigurationError {
            let message = connectionConfigurationError.localizedDescription
            errorMessage = message
            connectionStatus = .failed(message)
            return
        }
        guard connectionProfile.isConnectable else {
            let message = ConnectionProfileError.cloudNotConfigured.localizedDescription
            errorMessage = message
            connectionStatus = .failed(message)
            return
        }
        guard connectionAttemptTask == nil,
              connectionStatus != .connecting,
              !connectionStatus.isConnected else { return }
        reconnectTask?.cancel()
        reconnectTask = nil
        connectionAttemptGeneration += 1
        let attemptGeneration = connectionAttemptGeneration
        let expectedProfile = connectionProfile
        let expectedProfileGeneration = profileGeneration
        let expectedAgentID = stableAgentID
        connectionAttemptTask = Task { [weak self] in
            guard let self,
                  !Task.isCancelled,
                  profileGeneration == expectedProfileGeneration,
                  connectionProfile == expectedProfile else { return }
            await connect(
                expectedProfile: expectedProfile,
                expectedProfileGeneration: expectedProfileGeneration,
                expectedAgentID: expectedAgentID
            )
            if connectionAttemptGeneration == attemptGeneration {
                connectionAttemptTask = nil
            }
        }
    }

    func connect() async {
        await connect(
            expectedProfile: connectionProfile,
            expectedProfileGeneration: profileGeneration,
            expectedAgentID: stableAgentID
        )
    }

    private func connect(
        expectedProfile: ConnectionProfile,
        expectedProfileGeneration: Int,
        expectedAgentID: String
    ) async {
        guard !Task.isCancelled,
              expectedProfileGeneration == profileGeneration,
              connectionProfile == expectedProfile else { return }
        errorMessage = nil
        do {
            try await connectTransport(expectedProfileGeneration: expectedProfileGeneration)
            guard expectedProfileGeneration == profileGeneration,
                  connectionProfile == expectedProfile else { return }
            let registration = try await connection.register(
                name: agentName,
                agentID: expectedAgentID
            )
            guard expectedProfileGeneration == profileGeneration,
                  connectionProfile == expectedProfile else { return }
            agentID = registration.agentID
            let desiredRoomID = selectedRoomID
            joinedRoomID = nil
            for restoredRoomID in registration.restoredRoomIDs.sorted()
                where restoredRoomID != desiredRoomID {
                guard expectedProfileGeneration == profileGeneration else { return }
                try await connection.leave(roomID: restoredRoomID)
                guard expectedProfileGeneration == profileGeneration else { return }
            }
            if let desiredRoomID,
               registration.restoredRoomIDs.contains(desiredRoomID) {
                joinedRoomID = desiredRoomID
            }
            connectionStatus = .connected
            reconnectTask?.cancel()
            reconnectTask = nil
            try await refreshRooms(selectFallbackForMissingSelection: false)
            guard expectedProfileGeneration == profileGeneration else { return }
            if let joinedRoomID,
               !rooms.contains(where: { $0.id == joinedRoomID }) {
                self.joinedRoomID = nil
            }
            startRoomRefreshLoop()
            if let selectedRoom {
                await select(room: selectedRoom)
            } else {
                let initial = rooms.first(where: { roomSetupScreenIDs.contains($0.id) })
                    ?? rooms.first(where: { $0.name.lowercased() == "lobby" })
                    ?? rooms.first
                if let initial { await select(room: initial) }
                else { selectedRoomID = nil }
            }
            guard expectedProfileGeneration == profileGeneration else { return }
            startSetupReadinessPolling()
        } catch {
            guard expectedProfileGeneration == profileGeneration,
                  connectionProfile == expectedProfile else { return }
            connectionStatus = .failed(error.localizedDescription)
            scheduleReconnect()
        }
    }

    var isLocalConnection: Bool {
        connectionProfile.kind == .local
    }

    var isCowchatCloudConfigured: Bool {
        (try? configuredCowchatCloudProfile()) != nil
    }

    var connectionTargetDescription: String {
        connectionProfile.displayName
    }

    var agentConnectionInstruction: String {
        if isLocalConnection { return "connect to the local server" }
        return "connect to Cowchat Cloud at \(connectionProfile.endpointDescription) using your Cowchat Cloud API key"
    }

    func configuredCowchatCloudValues() -> (url: String, apiKey: String) {
        do {
            guard let profile = try configuredCowchatCloudProfile() else {
                return (connectionPreferences?.loadSavedCloudURL() ?? "", "")
            }
            return (profile.endpointURL?.absoluteString ?? "", profile.apiKey)
        } catch {
            errorMessage = error.localizedDescription
            return (connectionPreferences?.loadSavedCloudURL() ?? "", "")
        }
    }

    func useLocalConnection() {
        _ = activate(profile: .local)
    }

    func useCowchatCloud() {
        do {
            guard let profile = try configuredCowchatCloudProfile() else {
                errorMessage = "Add your Cowchat Cloud URL and API key in Settings first."
                return
            }
            _ = activate(profile: profile)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Retries a user-selected connection immediately. For Local this is the
    /// explicit boundary that permits one new helper launch after a previously
    /// latched crash, while background reconnects remain unable to respawn-loop.
    func reconnect() {
        guard !connectionStatus.isConnected else { return }
        reconnectTask?.cancel()
        reconnectTask = nil
        connectionAttemptTask?.cancel()
        connectionAttemptTask = nil
        connectionAttemptGeneration += 1
        // A manual retry starts a new transport session even when the selected
        // profile is unchanged. Fence every operation still awaiting the old
        // connection before either attempt can touch the shared transport.
        profileGeneration += 1
        let previousProfile = connectionProfile
        connection.disconnect()
        reloadSelectedProfileForReconnectIfNeeded()
        if previousProfile.kind == .local, connectionProfile.kind != .local {
            localServerSupervisor?.stopOwnedServer()
        }
        if connectionProfile.kind == .local {
            localServerSupervisor?.prepareForExplicitRetry()
        }
        connectionStatus = .disconnected
        start()
    }

    @discardableResult
    func saveAndUseCowchatCloud(url: String, apiKey: String) -> Bool {
        do {
            let profile = try ConnectionProfile.cowchatCloud(urlString: url, apiKey: apiKey)
            return activate(profile: profile)
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    func stopOwnedLocalServer() {
        localServerSupervisor?.stopOwnedServer()
    }

    func shutdownOwnedLocalServerForAppTermination() async {
        await localServerSupervisor?.shutdownOwnedServerForAppTermination()
    }

    private func connectTransport(expectedProfileGeneration: Int) async throws {
        do {
            try await connection.connect()
            guard expectedProfileGeneration == profileGeneration else {
                throw CancellationError()
            }
            return
        } catch {
            guard expectedProfileGeneration == profileGeneration,
                  connectionProfile.kind == .local,
                  let localServerSupervisor else { throw error }

            try await localServerSupervisor.launchIfNeeded()
            var lastError = error
            for delay in localServerRetryDelaysNanoseconds {
                if delay > 0 {
                    try await Task.sleep(nanoseconds: delay)
                }
                try Task.checkCancellation()
                guard expectedProfileGeneration == profileGeneration else {
                    throw CancellationError()
                }
                do {
                    try await connection.connect()
                    guard expectedProfileGeneration == profileGeneration else {
                        throw CancellationError()
                    }
                    return
                } catch {
                    lastError = error
                }
            }
            throw lastError
        }
    }

    private func configuredCowchatCloudProfile() throws -> ConnectionProfile? {
        guard let connectionPreferences else { return nil }
        return try connectionPreferences.loadConfiguredCloudProfile()
    }

    /// Startup can preserve a selected Cloud target while Keychain is temporarily
    /// unavailable. An explicit reconnect is the user-approved point to retry that
    /// persisted read; background reconnects keep the original fail-closed state.
    private func reloadSelectedProfileForReconnectIfNeeded() {
        guard connectionConfigurationError != nil || !connectionProfile.isConnectable,
              let connectionPreferences else { return }
        do {
            let reloadedProfile = try connectionPreferences.loadSelectedProfile()
            connectionConfigurationError = nil
            errorMessage = nil
            guard reloadedProfile != connectionProfile else { return }
            connectionProfile = reloadedProfile
            stableAgentID = Self.resolveAgentID(
                defaults: defaults,
                scope: reloadedProfile.persistentIdentityScope
            )
            connection.reconfigure(profile: reloadedProfile)
            resetServerBackedState(for: reloadedProfile)
        } catch {
            connectionConfigurationError = error
            errorMessage = error.localizedDescription
        }
    }

    @discardableResult
    private func activate(profile candidate: ConnectionProfile) -> Bool {
        let profile: ConnectionProfile
        do {
            profile = try connectionPreferences?.save(candidate) ?? candidate
        } catch {
            errorMessage = error.localizedDescription
            return false
        }

        guard profile != connectionProfile || !connectionStatus.isConnected else { return true }
        let previousProfile = connectionProfile
        reconnectTask?.cancel()
        reconnectTask = nil
        connectionAttemptTask?.cancel()
        connectionAttemptTask = nil
        connectionAttemptGeneration += 1
        profileGeneration += 1
        connection.disconnect()
        if previousProfile.kind == .local, profile.kind != .local {
            localServerSupervisor?.stopOwnedServer()
        }
        if profile.kind == .local {
            localServerSupervisor?.prepareForExplicitRetry()
        }
        connectionConfigurationError = nil
        errorMessage = nil
        connectionProfile = profile
        stableAgentID = Self.resolveAgentID(
            defaults: defaults,
            scope: profile.persistentIdentityScope
        )
        connection.reconfigure(profile: profile)
        resetServerBackedState(for: profile)
        start()
        return true
    }

    private func resetServerBackedState(for profile: ConnectionProfile) {
        roomRefreshTask?.cancel()
        roomRefreshTask = nil
        roomLoadTask?.cancel()
        roomLoadTask = nil
        messageSearchTask?.cancel()
        messageSearchTask = nil
        setupReadinessTask?.cancel()
        setupReadinessTask = nil
        roomPreviewTask?.cancel()
        roomPreviewTask = nil
        messageSearchGeneration += 1
        setupReadinessGeneration += 1
        roomSelectionGeneration += 1
        roomMutationGeneration += 1

        rooms = []
        selectedRoomID = nil
        messages = []
        roomMembers = []
        draft = ""
        searchText = ""
        messageSearchRoomIDs = []
        isSearchingMessages = false
        roomMessagePreviews = [:]
        lastThinkingAt = [:]
        roomReadyNotice = nil
        roomBeingRenamed = nil
        isLoadingMessages = false
        isCreateRoomPresented = false
        createRoomParentID = nil
        joinedRoomID = nil
        agentID = ""
        isRefreshingRooms = false
        pendingDestructionRoomIDs = []
        confirmedDestructionRoomIDs = []
        destroyedRoomIDs = []
        draftsByRoomID = [:]
        failedDraftRestorationsByRoomID = [:]
        previewActivityByRoomID = [:]
        activeMessageSearchContext = nil
        completedMessageSearchContext = nil

        localPreferences = Self.roomPreferences(defaults: defaults, profile: profile)
        archivedRoomIDs = localPreferences.archivedRoomIDs
        setupRoomIDs = localPreferences.pendingSetupRoomIDs
        roomSetupScreenIDs = localPreferences.pendingSetupScreenRoomIDs
        readState = localPreferences.roomReadState ?? RoomReadState()
        connectionStatus = .disconnected
    }

    private static func roomPreferences(
        defaults: UserDefaults,
        profile: ConnectionProfile
    ) -> RoomLocalPreferences {
        // Local preserves the pre-profile keys. Cloud uses its opaque account
        // identifier, so two principals on one endpoint never share UI state.
        return RoomLocalPreferences(
            defaults: defaults,
            scope: profile.persistentIdentityScope
        )
    }

    func refreshRooms(selectFallbackForMissingSelection: Bool = true) async throws {
        guard !isRefreshingRooms else { return }
        let expectedProfileGeneration = profileGeneration
        isRefreshingRooms = true
        defer {
            if expectedProfileGeneration == profileGeneration {
                isRefreshingRooms = false
            }
        }

        let baseline = roomMutationGeneration
        var refreshed = try await connection.listRooms()
        guard expectedProfileGeneration == profileGeneration else {
            throw CancellationError()
        }
        refreshed.removeAll { destroyedRoomIDs.contains($0.id) }
        if roomMutationGeneration != baseline {
            let currentByID = Dictionary(uniqueKeysWithValues: rooms.map { ($0.id, $0) })
            for (roomID, generation) in roomMutationGenerationByID where generation > baseline {
                refreshed.removeAll { $0.id == roomID }
                if let current = currentByID[roomID] { refreshed.append(current) }
            }
        }
        rooms = refreshed.sorted(by: roomSort)
        reconcileLocalRoomPreferences()
        if !readState.hasSeeded {
            readState.seed(rooms: rooms)
            localPreferences.saveRoomReadState(readState)
        }
        let readEntryCountBeforeReconcile = readState.entries.count
        readState.reconcile(validRoomIDs: Set(rooms.map(\.id)))
        if readState.entries.count != readEntryCountBeforeReconcile {
            localPreferences.saveRoomReadState(readState)
        }
        markSelectedRoomRead()
        if selectFallbackForMissingSelection,
           let selectedRoomID,
           !rooms.contains(where: { $0.id == selectedRoomID }) {
            if joinedRoomID == selectedRoomID { joinedRoomID = nil }
            await selectFallbackRoom(excluding: selectedRoomID)
            guard expectedProfileGeneration == profileGeneration else {
                throw CancellationError()
            }
        }
        scheduleRoomPreviewRefresh()
        if !searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            scheduleMessageSearch(restartInFlight: false)
        }
    }

    func select(room: Room) async {
        let expectedProfileGeneration = profileGeneration
        roomSelectionGeneration += 1
        let generation = roomSelectionGeneration
        saveDraft(for: selectedRoomID)
        selectedRoomID = room.roomID
        markSelectedRoomRead()
        draft = draftsByRoomID[room.roomID] ?? ""
        messages = []
        roomMembers = []
        guard connectionStatus.isConnected else {
            isLoadingMessages = false
            return
        }
        isLoadingMessages = true
        errorMessage = nil
        let previousTransition = roomLoadTask
        let transition = Task { [weak self] in
            guard let self else { return }
            await previousTransition?.value
            guard !Task.isCancelled,
                  expectedProfileGeneration == profileGeneration,
                  generation == roomSelectionGeneration else { return }
            do {
                if let joinedRoomID, joinedRoomID != room.roomID {
                    try await connection.leave(roomID: joinedRoomID)
                    guard !Task.isCancelled,
                          expectedProfileGeneration == profileGeneration else { return }
                    decrementRoomMemberCount(roomID: joinedRoomID)
                    self.joinedRoomID = nil
                }
                if joinedRoomID != room.roomID {
                    guard !Task.isCancelled,
                          expectedProfileGeneration == profileGeneration,
                          generation == roomSelectionGeneration else { return }
                    try await connection.join(roomID: room.roomID)
                    guard !Task.isCancelled,
                          expectedProfileGeneration == profileGeneration else { return }
                    self.joinedRoomID = room.roomID
                }
                // A newer selection may have arrived while join was in flight.
                // The next serialized transition will leave this actual joined
                // room; this stale load must not mutate the visible conversation.
                guard !Task.isCancelled,
                      expectedProfileGeneration == profileGeneration,
                      generation == roomSelectionGeneration else { return }
                let history = Self.visibleMessages(
                    in: try await connection.history(roomID: room.roomID, limit: 100)
                )
                guard !Task.isCancelled,
                      expectedProfileGeneration == profileGeneration,
                      generation == roomSelectionGeneration,
                      selectedRoomID == room.roomID else { return }
                messages = Self.merging(history: history, live: messages)
                if let latest = messages.last { updateRoomPreview(from: latest) }
                let members = try await connection.listAgents(roomID: room.roomID)
                guard !Task.isCancelled,
                      expectedProfileGeneration == profileGeneration,
                      generation == roomSelectionGeneration,
                      selectedRoomID == room.roomID else { return }
                roomMembers = members.sorted {
                    $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
                }
                updateRoomMemberCount(roomID: room.roomID, count: members.count)
                if hasCollaborator(in: members) {
                    markSetupRoomReadyIfNeeded(roomID: room.roomID)
                }
            } catch {
                guard !Task.isCancelled,
                      expectedProfileGeneration == profileGeneration,
                      generation == roomSelectionGeneration,
                      selectedRoomID == room.roomID else { return }
                present(error)
            }
            if !Task.isCancelled,
               expectedProfileGeneration == profileGeneration,
               generation == roomSelectionGeneration,
               selectedRoomID == room.roomID { isLoadingMessages = false }
        }
        roomLoadTask = transition
        await transition.value
    }

    func createRoom(name: String, description: String, ephemeral: Bool, isPublic: Bool) async -> Bool {
        let expectedProfileGeneration = profileGeneration
        do {
            let room = try await connection.createRoom(
                name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                description: description.trimmingCharacters(in: .whitespacesAndNewlines),
                parentID: createRoomParentID,
                ephemeral: ephemeral,
                isPublic: isPublic
            )
            guard expectedProfileGeneration == profileGeneration else { return false }
            if !rooms.contains(where: { $0.id == room.id }) {
                rooms.append(room)
                recordRoomMutation(roomID: room.id)
            }
            rooms.sort(by: roomSort)
            setupRoomIDs.insert(room.id)
            localPreferences.savePendingSetupRoomIDs(setupRoomIDs)
            roomSetupScreenIDs.insert(room.id)
            localPreferences.savePendingSetupScreenRoomIDs(roomSetupScreenIDs)
            isCreateRoomPresented = false
            createRoomParentID = nil
            await select(room: room)
            guard expectedProfileGeneration == profileGeneration else { return false }
            startSetupReadinessPolling()
            return true
        } catch {
            guard expectedProfileGeneration == profileGeneration else { return false }
            present(error)
            return false
        }
    }

    func presentCreateRoom(parentID: String? = nil) {
        createRoomParentID = parentID
        isCreateRoomPresented = true
    }

    func isArchived(_ room: Room) -> Bool {
        archivedRoomIDs.contains(room.id)
    }

    func isUnread(_ room: Room) -> Bool {
        readState.isUnread(room, selectedRoomID: selectedRoomID)
    }

    func isWorking(_ room: Room, at now: Date = Date()) -> Bool {
        RoomSidebarPresentation.isWorking(thinkingByAgent: lastThinkingAt[room.id], now: now)
    }

    func archive(_ room: Room) async {
        guard room.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame else {
            errorMessage = "The lobby cannot be archived."
            return
        }
        archivedRoomIDs.insert(room.id)
        localPreferences.saveArchivedRoomIDs(archivedRoomIDs)

        guard selectedRoomID == room.id else { return }
        await selectFallbackRoom(excluding: room.id)
    }

    func unarchive(_ room: Room) {
        guard archivedRoomIDs.remove(room.id) != nil else { return }
        localPreferences.saveArchivedRoomIDs(archivedRoomIDs)
    }

    func canDestroy(_ room: Room) -> Bool {
        room.roomID != "lobby"
            && room.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame
            && room.createdBy == (agentID.isEmpty ? stableAgentID : agentID)
    }

    func canRename(_ room: Room) -> Bool {
        canDestroy(room)
    }

    func presentRename(_ room: Room) {
        guard canRename(room) else {
            errorMessage = "Only the agent that created this room can rename it."
            return
        }
        roomBeingRenamed = room
    }

    func rename(_ room: Room, to proposedName: String) async -> Bool {
        let expectedProfileGeneration = profileGeneration
        guard canRename(room) else {
            errorMessage = "Only the agent that created this room can rename it."
            return false
        }
        guard connectionStatus.isConnected else {
            errorMessage = "Reconnect before renaming a room."
            return false
        }
        let name = proposedName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else {
            errorMessage = "Room names cannot be empty."
            return false
        }

        do {
            let updated = try await connection.rename(roomID: room.id, name: name)
            guard expectedProfileGeneration == profileGeneration else { return false }
            replaceRoom(updated)
            roomBeingRenamed = nil
            return true
        } catch {
            guard expectedProfileGeneration == profileGeneration else { return false }
            present(error)
            return false
        }
    }

    func destroy(_ room: Room) async -> Bool {
        let expectedProfileGeneration = profileGeneration
        guard canDestroy(room) else {
            errorMessage = "Only the agent that created this room can destroy it."
            return false
        }
        guard connectionStatus.isConnected else {
            errorMessage = "Reconnect before destroying a room."
            return false
        }

        pendingDestructionRoomIDs.insert(room.id)
        let wasSelected = selectedRoomID == room.id
        defer {
            if expectedProfileGeneration == profileGeneration {
                pendingDestructionRoomIDs.remove(room.id)
                confirmedDestructionRoomIDs.remove(room.id)
            }
        }
        do {
            try await connection.destroy(roomID: room.id)
            guard expectedProfileGeneration == profileGeneration else { return false }
            removeRoom(roomID: room.id)
            if wasSelected {
                await selectFallbackRoom(excluding: room.id)
                guard expectedProfileGeneration == profileGeneration else { return false }
            }
            return true
        } catch {
            guard expectedProfileGeneration == profileGeneration else { return false }
            // The lifecycle event is authoritative even if the correlated
            // request reply is lost or arrives as an error afterward.
            if confirmedDestructionRoomIDs.contains(room.id) {
                if wasSelected {
                    await selectFallbackRoom(excluding: room.id)
                    guard expectedProfileGeneration == profileGeneration else { return false }
                }
                return true
            }
            present(error)
            return false
        }
    }

    func completeRoomSetup(_ room: Room) async {
        let expectedProfileGeneration = profileGeneration
        roomSetupScreenIDs.remove(room.id)
        localPreferences.savePendingSetupScreenRoomIDs(roomSetupScreenIDs)
        if !room.ephemeral, let lobby = rooms.first(where: {
            $0.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame
        }) {
            await select(room: lobby)
            guard expectedProfileGeneration == profileGeneration else { return }
        }
        startSetupReadinessPolling()
    }

    func openRoomReadyNotice() async {
        let expectedProfileGeneration = profileGeneration
        guard let room = roomReadyNotice,
              rooms.contains(where: { $0.id == room.id }) else {
            roomReadyNotice = nil
            return
        }
        roomReadyNotice = nil
        await select(room: room)
        guard expectedProfileGeneration == profileGeneration else { return }
    }

    func sendDraft() {
        guard let room = selectedRoom, !room.encrypted else { return }
        let content = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !content.isEmpty else { return }
        guard connectionStatus.isConnected else {
            start()
            return
        }
        sendGeneration += 1
        let generation = sendGeneration
        let expectedProfileGeneration = profileGeneration
        draft = ""
        draftsByRoomID.removeValue(forKey: room.id)
        failedDraftRestorationsByRoomID.removeValue(forKey: room.id)
        Task {
            do {
                let message = try await connection.send(roomID: room.roomID, content: content)
                guard expectedProfileGeneration == profileGeneration else { return }
                if selectedRoomID == room.roomID { append(message) }
            } catch {
                guard expectedProfileGeneration == profileGeneration else { return }
                let visibleReplacement = selectedRoomID == room.roomID ? draft : ""
                let savedReplacement = draftsByRoomID[room.id] ?? ""
                let nonemptyReplacements = [visibleReplacement, savedReplacement].filter { !$0.isEmpty }
                let priorFailure = failedDraftRestorationsByRoomID[room.id]
                let onlyOlderFailureIsVisible = priorFailure.map { prior in
                    prior.generation < generation
                        && !nonemptyReplacements.isEmpty
                        && nonemptyReplacements.allSatisfy { $0 == prior.content }
                } ?? false
                if nonemptyReplacements.isEmpty || onlyOlderFailureIsVisible {
                    draftsByRoomID[room.id] = content
                    if selectedRoomID == room.roomID { draft = content }
                    failedDraftRestorationsByRoomID[room.id] = FailedDraftRestoration(
                        generation: generation,
                        content: content
                    )
                }
                present(error)
            }
        }
    }

    private func handleEvent(type: String, payload: [String: Any]) {
        switch type {
        case "message_received":
            if let message = try? decode(ChatMessage.self, payload) {
                lastThinkingAt = RoomSidebarPresentation.updatedThinkingByAgent(
                    lastThinkingAt, message: message, now: Date()
                )
                if message.isThinking {
                    updateRoomActivity(from: message)
                    break
                }
                if message.agentID != agentID {
                    markSetupRoomReadyIfNeeded(roomID: message.roomID)
                }
                if !currentSearchQuery.isEmpty,
                   message.content.localizedCaseInsensitiveContains(currentSearchQuery)
                    || message.agentName.localizedCaseInsensitiveContains(currentSearchQuery) {
                    messageSearchRoomIDs.insert(message.roomID)
                }
                if message.roomID == selectedRoomID {
                    append(message)
                } else {
                    updateRoomActivity(from: message)
                }
            }
        case "room_created":
            if let room = try? decode(Room.self, payload),
               !destroyedRoomIDs.contains(room.id),
               !rooms.contains(where: { $0.id == room.id }) {
                rooms.append(room)
                rooms.sort(by: roomSort)
                recordRoomMutation(roomID: room.id)
            }
        case "room_updated":
            if let room = try? decode(Room.self, payload) {
                replaceRoom(room)
            }
        case "room_destroyed":
            if let id = payload["room_id"] as? String {
                if pendingDestructionRoomIDs.contains(id) {
                    confirmedDestructionRoomIDs.insert(id)
                }
                let wasSelected = selectedRoomID == id
                removeRoom(roomID: id)
                if wasSelected, !pendingDestructionRoomIDs.contains(id) {
                    let expectedProfileGeneration = profileGeneration
                    Task { [weak self] in
                        guard let self,
                              expectedProfileGeneration == profileGeneration else { return }
                        await selectFallbackRoom(excluding: id)
                    }
                }
            }
        case "agent_joined", "agent_left":
            let expectedProfileGeneration = profileGeneration
            Task { [weak self] in
                guard let self,
                      expectedProfileGeneration == profileGeneration else { return }
                await refreshMembers()
            }
        case "presence_update":
            updatePresence(from: payload)
        case "thinking":
            // Live thinking pulses arrive as this dedicated event, not
            // message_received, and are excluded from the visible message
            // feed — so track the working signal only; never bump room
            // activity for content the user can't see.
            if let message = try? decode(ChatMessage.self, payload) {
                lastThinkingAt = RoomSidebarPresentation.updatedThinkingByAgent(
                    lastThinkingAt, message: message, now: Date()
                )
            }
        default:
            break
        }
    }

    private func handleConnectionStatus(_ status: ConnectionStatus) {
        connectionStatus = status
        if !status.isConnected {
            roomRefreshTask?.cancel()
            roomRefreshTask = nil
            messageSearchTask?.cancel()
            messageSearchTask = nil
            messageSearchGeneration += 1
            activeMessageSearchContext = nil
            completedMessageSearchContext = nil
            isSearchingMessages = false
            setupReadinessTask?.cancel()
            setupReadinessTask = nil
            setupReadinessGeneration += 1
            roomPreviewTask?.cancel()
            roomPreviewTask = nil
        }
        if case .failed = status {
            joinedRoomID = nil
            roomMembers = []
            scheduleReconnect()
        }
    }

    private func refreshMembers() async {
        let expectedProfileGeneration = profileGeneration
        guard connectionStatus.isConnected, let roomID = selectedRoomID else { return }
        guard let members = try? await connection.listAgents(roomID: roomID),
              expectedProfileGeneration == profileGeneration,
              selectedRoomID == roomID else { return }
        roomMembers = members.sorted {
            $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
        updateRoomMemberCount(roomID: roomID, count: members.count)
        if hasCollaborator(in: members) { markSetupRoomReadyIfNeeded(roomID: roomID) }
    }

    private func updatePresence(from payload: [String: Any]) {
        guard let agentID = payload["agent_id"] as? String,
              let index = roomMembers.firstIndex(where: { $0.agentID == agentID }) else { return }
        roomMembers[index] = roomMembers[index].updating(
            status: payload["status"] as? String,
            detail: payload["status_detail"] as? String,
            progress: payload["progress"] as? Int
        )
    }

    private func scheduleReconnect() {
        guard reconnectTask == nil else { return }
        reconnectTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            guard !Task.isCancelled, let self else { return }
            reconnectTask = nil
            start()
        }
    }

    private func startRoomRefreshLoop() {
        roomRefreshTask?.cancel()
        roomRefreshTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 30_000_000_000)
                guard !Task.isCancelled, let self, connectionStatus.isConnected else { return }
                try? await refreshRooms()
            }
        }
    }

    private func scheduleRoomPreviewRefresh() {
        roomPreviewTask?.cancel()
        roomPreviewTask = nil
        guard connectionStatus.isConnected else { return }
        let candidates = rooms.filter { room in
            !room.encrypted
                && previewActivityByRoomID[room.id] != (room.lastActivity ?? room.createdAt)
        }
        guard !candidates.isEmpty else { return }
        let expectedProfileGeneration = profileGeneration

        roomPreviewTask = Task { [weak self] in
            guard let self,
                  expectedProfileGeneration == profileGeneration else { return }
            for room in candidates {
                guard !Task.isCancelled,
                      expectedProfileGeneration == profileGeneration else { return }
                let capturedActivity = room.lastActivity ?? room.createdAt
                do {
                    let latest = try await latestVisibleMessage(roomID: room.id)
                    guard !Task.isCancelled,
                          expectedProfileGeneration == profileGeneration,
                          let current = rooms.first(where: { $0.id == room.id }),
                          (current.lastActivity ?? current.createdAt) == capturedActivity else {
                        continue
                    }
                    if let latest { updateRoomPreview(from: latest) }
                    previewActivityByRoomID[room.id] = capturedActivity
                } catch {
                    continue
                }
            }
        }
    }

    private func latestVisibleMessage(roomID: String) async throws -> ChatMessage? {
        var before: String?
        var seenCutoffs: Set<String> = []
        while !Task.isCancelled {
            let page = try await connection.history(roomID: roomID, limit: 20, before: before)
            if let latest = Self.visibleMessages(in: page).last { return latest }
            guard page.count == 20,
                  let cutoff = page.first?.timestamp,
                  !cutoff.isEmpty,
                  seenCutoffs.insert(cutoff).inserted else { return nil }
            before = cutoff
        }
        return nil
    }

    private func startSetupReadinessPolling() {
        guard setupReadinessTask == nil,
              connectionStatus.isConnected,
              !setupRoomIDs.isEmpty else { return }
        setupReadinessGeneration += 1
        let generation = setupReadinessGeneration
        let expectedProfileGeneration = profileGeneration
        setupReadinessTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self,
                      expectedProfileGeneration == profileGeneration,
                      connectionStatus.isConnected,
                      !setupRoomIDs.isEmpty else { break }
                await pollSetupRoomReadiness()
                guard expectedProfileGeneration == profileGeneration,
                      !setupRoomIDs.isEmpty else { break }
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
            guard let self,
                  expectedProfileGeneration == profileGeneration,
                  setupReadinessGeneration == generation else { return }
            setupReadinessTask = nil
        }
    }

    func pollSetupRoomReadiness() async {
        let expectedProfileGeneration = profileGeneration
        guard connectionStatus.isConnected else { return }
        for roomID in Array(setupRoomIDs) {
            guard !Task.isCancelled,
                  expectedProfileGeneration == profileGeneration else { return }
            if let members = try? await connection.listAgents(roomID: roomID),
               expectedProfileGeneration == profileGeneration,
               hasCollaborator(in: members) {
                markSetupRoomReadyIfNeeded(roomID: roomID)
            }
        }
    }

    private func append(_ message: ChatMessage) {
        guard !messages.contains(where: { $0.id == message.id }) else { return }
        messages.append(message)
        messages.sort { $0.seq < $1.seq }
        if messages.count > 200 { messages.removeFirst(messages.count - 200) }
        updateRoomActivity(from: message)
    }

    private func updateRoomActivity(from message: ChatMessage) {
        updateRoomPreview(from: message)
        guard let activity = message.timestamp.cowchatDate,
              let index = rooms.firstIndex(where: { $0.roomID == message.roomID }) else { return }
        let currentActivity = rooms[index].activityDate
        if let currentActivity, activity <= currentActivity { return }
        rooms[index] = rooms[index].updating(lastActivity: message.timestamp)
        rooms.sort(by: roomSort)
        recordRoomMutation(roomID: message.roomID)
        if message.roomID == selectedRoomID { markSelectedRoomRead() }
    }

    private func updateRoomPreview(from message: ChatMessage) {
        guard !message.isThinking else { return }
        let preview = message.content
            .components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        guard !preview.isEmpty else { return }
        roomMessagePreviews[message.roomID] = String(preview.prefix(140))
        previewActivityByRoomID[message.roomID] = message.timestamp
    }

    private func updateRoomMemberCount(roomID: String, count: Int) {
        guard let index = rooms.firstIndex(where: { $0.roomID == roomID }),
              rooms[index].memberCount != count else { return }
        rooms[index] = rooms[index].updating(memberCount: count)
        recordRoomMutation(roomID: roomID)
    }

    private func decrementRoomMemberCount(roomID: String) {
        guard let index = rooms.firstIndex(where: { $0.roomID == roomID }),
              let count = rooms[index].memberCount else { return }
        rooms[index] = rooms[index].updating(memberCount: max(0, count - 1))
        recordRoomMutation(roomID: roomID)
    }

    private func scheduleMessageSearch(restartInFlight: Bool) {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        let searchableRooms = rooms.filter { !$0.encrypted }
        let context = MessageSearchContext(
            query: query,
            roomVersions: searchableRooms.map {
                "\($0.id)\u{0}\($0.lastActivity ?? $0.createdAt)"
            }.sorted()
        )

        if !restartInFlight,
           activeMessageSearchContext == context
            || completedMessageSearchContext == context {
            return
        }

        messageSearchGeneration += 1
        let generation = messageSearchGeneration
        messageSearchTask?.cancel()
        messageSearchTask = nil
        activeMessageSearchContext = nil
        completedMessageSearchContext = nil
        messageSearchRoomIDs = []
        guard !query.isEmpty, connectionStatus.isConnected else {
            isSearchingMessages = false
            return
        }

        isSearchingMessages = true
        activeMessageSearchContext = context
        messageSearchTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 250_000_000)
            guard !Task.isCancelled,
                  let self,
                  generation == messageSearchGeneration,
                  activeMessageSearchContext == context else { return }

            var matchingRoomIDs: Set<String> = []
            for room in searchableRooms {
                guard !Task.isCancelled,
                      generation == messageSearchGeneration,
                      activeMessageSearchContext == context else { return }
                if (try? await roomContainsSearchMatch(room, query: query)) == true {
                    matchingRoomIDs.insert(room.id)
                }
            }

            guard !Task.isCancelled,
                  generation == messageSearchGeneration,
                  activeMessageSearchContext == context else { return }
            messageSearchRoomIDs.formUnion(matchingRoomIDs)
            isSearchingMessages = false
            activeMessageSearchContext = nil
            completedMessageSearchContext = context
            messageSearchTask = nil
        }
    }

    private func roomContainsSearchMatch(_ room: Room, query: String) async throws -> Bool {
        var before: String?
        var seenCutoffs: Set<String> = []
        while !Task.isCancelled {
            let page = try await connection.history(roomID: room.id, limit: 100, before: before)
            if Self.visibleMessages(in: page).contains(where: { message in
                message.content.localizedCaseInsensitiveContains(query)
                    || message.agentName.localizedCaseInsensitiveContains(query)
            }) {
                return true
            }
            guard page.count == 100,
                  let cutoff = page.first?.timestamp,
                  !cutoff.isEmpty,
                  seenCutoffs.insert(cutoff).inserted else { return false }
            before = cutoff
        }
        return false
    }

    private var currentSearchQuery: String {
        searchText.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func markSelectedRoomRead() {
        guard let selectedRoomID,
              let room = rooms.first(where: { $0.id == selectedRoomID }) else { return }
        let before = readState.entries[selectedRoomID]
        readState.markRead(roomID: selectedRoomID, activityDate: room.activityDate)
        // Persist only on real change — this runs on every refresh tick while
        // a room stays selected.
        if readState.entries[selectedRoomID] != before {
            localPreferences.saveRoomReadState(readState)
        }
    }

    private func reconcileLocalRoomPreferences() {
        let validRoomIDs = Set(rooms.map(\.id))
        let archived = archivedRoomIDs.intersection(validRoomIDs)
        if archived != archivedRoomIDs {
            archivedRoomIDs = archived
            localPreferences.saveArchivedRoomIDs(archivedRoomIDs)
        }
        let pendingSetup = setupRoomIDs.intersection(validRoomIDs)
        if pendingSetup != setupRoomIDs {
            setupRoomIDs = pendingSetup
            localPreferences.savePendingSetupRoomIDs(setupRoomIDs)
        }
        let setupScreens = roomSetupScreenIDs.intersection(pendingSetup)
        if setupScreens != roomSetupScreenIDs {
            roomSetupScreenIDs = setupScreens
            localPreferences.savePendingSetupScreenRoomIDs(roomSetupScreenIDs)
        }
        if let roomBeingRenamed,
           !validRoomIDs.contains(roomBeingRenamed.id) {
            self.roomBeingRenamed = nil
        }
        if let createRoomParentID,
           !validRoomIDs.contains(createRoomParentID) {
            self.createRoomParentID = nil
            isCreateRoomPresented = false
        }
    }

    private func removeRoom(roomID: String) {
        destroyedRoomIDs.insert(roomID)
        rooms.removeAll { $0.roomID == roomID }
        archivedRoomIDs.remove(roomID)
        setupRoomIDs.remove(roomID)
        localPreferences.savePendingSetupRoomIDs(setupRoomIDs)
        roomSetupScreenIDs.remove(roomID)
        localPreferences.savePendingSetupScreenRoomIDs(roomSetupScreenIDs)
        draftsByRoomID.removeValue(forKey: roomID)
        failedDraftRestorationsByRoomID.removeValue(forKey: roomID)
        messageSearchRoomIDs.remove(roomID)
        roomMessagePreviews.removeValue(forKey: roomID)
        previewActivityByRoomID.removeValue(forKey: roomID)
        lastThinkingAt.removeValue(forKey: roomID)
        if roomReadyNotice?.id == roomID { roomReadyNotice = nil }
        if roomBeingRenamed?.id == roomID { roomBeingRenamed = nil }
        if createRoomParentID == roomID {
            createRoomParentID = nil
            isCreateRoomPresented = false
        }
        localPreferences.saveArchivedRoomIDs(archivedRoomIDs)
        recordRoomMutation(roomID: roomID)

        if joinedRoomID == roomID { joinedRoomID = nil }
        if selectedRoomID == roomID {
            roomSelectionGeneration += 1
            selectedRoomID = nil
            messages = []
            roomMembers = []
            isLoadingMessages = false
        }
    }

    private func replaceRoom(_ room: Room) {
        guard !destroyedRoomIDs.contains(room.id) else { return }
        guard let index = rooms.firstIndex(where: { $0.id == room.id }) else {
            rooms.append(room)
            rooms.sort(by: roomSort)
            recordRoomMutation(roomID: room.id)
            return
        }
        let existing = rooms[index]
        let merged = Room(
            roomID: room.roomID,
            name: room.name,
            description: room.description,
            parentID: room.parentID,
            ephemeral: room.ephemeral,
            createdAt: room.createdAt,
            createdBy: room.createdBy,
            visibility: room.visibility,
            lastActivity: room.lastActivity ?? existing.lastActivity,
            memberCount: room.memberCount ?? existing.memberCount,
            encrypted: room.encrypted
        )
        rooms[index] = merged
        rooms.sort(by: roomSort)
        if roomReadyNotice?.id == room.id { roomReadyNotice = merged }
        if roomBeingRenamed?.id == room.id { roomBeingRenamed = merged }
        recordRoomMutation(roomID: room.id)
    }

    private func selectFallbackRoom(excluding roomID: String) async {
        let candidates = unarchivedRooms.filter { $0.id != roomID }
        let fallback = candidates.first(where: {
            $0.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame
        }) ?? candidates.first
        guard let fallback else {
            selectedRoomID = nil
            messages = []
            roomMembers = []
            isLoadingMessages = false
            return
        }
        await select(room: fallback)
    }

    private func markSetupRoomReadyIfNeeded(roomID: String) {
        guard setupRoomIDs.remove(roomID) != nil,
              let room = rooms.first(where: { $0.id == roomID }) else { return }
        localPreferences.savePendingSetupRoomIDs(setupRoomIDs)
        roomSetupScreenIDs.remove(roomID)
        localPreferences.savePendingSetupScreenRoomIDs(roomSetupScreenIDs)
        if selectedRoomID != roomID { roomReadyNotice = room }
    }

    private func hasCollaborator(in members: [AgentPresence]) -> Bool {
        members.contains { $0.id != agentID }
    }

    private func saveDraft(for roomID: String?) {
        guard let roomID else { return }
        if draft.isEmpty {
            draftsByRoomID.removeValue(forKey: roomID)
        } else {
            draftsByRoomID[roomID] = draft
        }
    }

    private func recordRoomMutation(roomID: String) {
        roomMutationGeneration += 1
        roomMutationGenerationByID[roomID] = roomMutationGeneration
    }

    static func merging(history: [ChatMessage], live: [ChatMessage]) -> [ChatMessage] {
        var byID: [String: ChatMessage] = [:]
        for message in history + live { byID[message.id] = message }
        return byID.values.sorted { $0.seq < $1.seq }
    }

    static func visibleMessages(in messages: [ChatMessage]) -> [ChatMessage] {
        messages.filter { !$0.isThinking }
    }

    private func decode<T: Decodable>(_ type: T.Type, _ object: [String: Any]) throws -> T {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try JSONDecoder().decode(type, from: data)
    }

    /// Lobby-first ordering for the dashboard grid and fallback selection.
    /// Deliberately NOT shared with RoomSidebarPresentation.sortedByRecency:
    /// the sidebar excludes Lobby (it lives in its own nav row) and sorts by
    /// parsed recency alone.
    private func roomSort(_ lhs: Room, _ rhs: Room) -> Bool {
        guard lhs.id != rhs.id else { return false }
        let lhsIsLobby = lhs.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame
        let rhsIsLobby = rhs.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame
        if lhsIsLobby != rhsIsLobby { return lhsIsLobby }
        return (lhs.lastActivity ?? lhs.createdAt) > (rhs.lastActivity ?? rhs.createdAt)
    }

    private func present(_ error: Error) {
        errorMessage = error.localizedDescription
    }
}
