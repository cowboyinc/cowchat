import XCTest
@testable import CowchatMac

@MainActor
private final class MockRoomConnection: CowchatConnectionProtocol {
    var onEvent: ((String, [String: Any]) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?
    var operations: [String] = []
    var blockedRoomID: String?
    var agentsByRoom: [String: [AgentPresence]] = [:]
    var historiesByRoom: [String: [ChatMessage]] = [:]
    var historyRequests: [(roomID: String, limit: Int, before: String?)] = []
    var listedRooms: [Room] = []
    var roomToCreate: Room?
    var blocksCreate = false
    var registration = CowchatRegistration(agentID: "")
    var registeredAgentIDs: [String] = []
    var blocksNextRegistration = false
    var destroyedRoomIDs: [String] = []
    var blocksDestroy = false
    var emitsDestroyEventBeforeReply = false
    var destroyShouldFailAfterEvent = false
    var renamedRooms: [(String, String)] = []
    var blocksListRooms = false
    var blocksListAgents = false
    var capturesListAgentRequests = false
    var listAgentRequests: [String] = []
    var capturesHistoryRequests = false
    var blockedHistoryLimit: Int?
    var blocksSend = false
    var sendShouldFail = false
    var connectCallCount = 0
    var connectFailuresRemaining = 0
    var blocksConnect = false
    var registrationError: Error?
    var createRoomError: Error?
    private var connectContinuation: CheckedContinuation<Void, Never>?
    private var registrationContinuation: CheckedContinuation<Void, Never>?
    private var createContinuation: CheckedContinuation<Void, Never>?
    private var destroyContinuation: CheckedContinuation<Void, Never>?
    private var joinContinuation: CheckedContinuation<Void, Never>?
    private var listRoomsContinuation: CheckedContinuation<Void, Never>?
    private var listAgentsContinuation: CheckedContinuation<Void, Never>?
    private var capturedListAgentContinuations: [Int: CheckedContinuation<[AgentPresence], Error>] = [:]
    private var capturedHistoryContinuations: [Int: CheckedContinuation<[ChatMessage], Error>] = [:]
    private var historyContinuation: CheckedContinuation<Void, Never>?
    private var sendContinuations: [String: CheckedContinuation<Void, Never>] = [:]

    func connect() async throws {
        connectCallCount += 1
        // Production-faithful: CowchatConnection.connect() emits .connecting at
        // entry, then .failed(<transport message>) on a failed attempt, before
        // any local-server-supervisor error is ever thrown (CowchatConnection.swift:136,
        // failConnection). ChatStore must not infer "already surfaced" from
        // connectionStatus, since this overwrites it mid-attempt.
        onStatusChange?(.connecting)
        if blocksConnect {
            await withCheckedContinuation { connectContinuation = $0 }
        }
        if connectFailuresRemaining > 0 {
            connectFailuresRemaining -= 1
            let error = CowchatConnectionError.transport("Connection refused")
            onStatusChange?(.failed(error.localizedDescription))
            throw error
        }
    }
    func register(name: String, agentID: String) async throws -> CowchatRegistration {
        registeredAgentIDs.append(agentID)
        if blocksNextRegistration {
            blocksNextRegistration = false
            await withCheckedContinuation { registrationContinuation = $0 }
        }
        if let registrationError { throw registrationError }
        return CowchatRegistration(
            agentID: registration.agentID.isEmpty ? agentID : registration.agentID,
            restoredRoomIDs: registration.restoredRoomIDs
        )
    }
    func listRooms() async throws -> [Room] {
        operations.append("listRooms")
        if blocksListRooms {
            await withCheckedContinuation { listRoomsContinuation = $0 }
        }
        return listedRooms
    }
    func listAgents(roomID: String) async throws -> [AgentPresence] {
        listAgentRequests.append(roomID)
        let requestNumber = listAgentRequests.count
        if capturesListAgentRequests {
            return try await withTaskCancellationHandler {
                try Task.checkCancellation()
                return try await withCheckedThrowingContinuation { continuation in
                    if Task.isCancelled {
                        continuation.resume(throwing: CancellationError())
                    } else {
                        capturedListAgentContinuations[requestNumber] = continuation
                    }
                }
            } onCancel: {
                Task { @MainActor [weak self] in
                    self?.capturedListAgentContinuations
                        .removeValue(forKey: requestNumber)?
                        .resume(throwing: CancellationError())
                }
            }
        }
        if blocksListAgents {
            await withCheckedContinuation { listAgentsContinuation = $0 }
        }
        return agentsByRoom[roomID] ?? []
    }
    func createRoom(
        name: String,
        description: String?,
        parentID: String?,
        ephemeral: Bool,
        isPublic: Bool
    ) async throws -> Room {
        operations.append("create:\(name)")
        if let createRoomError { throw createRoomError }
        if blocksCreate {
            await withCheckedContinuation { createContinuation = $0 }
        }
        guard let roomToCreate else { fatalError("set roomToCreate before creating") }
        return roomToCreate
    }
    func destroy(roomID: String) async throws {
        operations.append("destroy:\(roomID)")
        destroyedRoomIDs.append(roomID)
        if blocksDestroy {
            await withCheckedContinuation { destroyContinuation = $0 }
        }
        if emitsDestroyEventBeforeReply {
            onEvent?("room_destroyed", ["room_id": roomID])
        }
        if destroyShouldFailAfterEvent {
            throw NSError(domain: "MockRoomConnection.destroy", code: 1)
        }
    }
    func rename(roomID: String, name: String) async throws -> Room {
        operations.append("rename:\(roomID):\(name)")
        renamedRooms.append((roomID, name))
        guard let room = listedRooms.first(where: { $0.id == roomID }) else {
            fatalError("set listedRooms before renaming")
        }
        return Room(
            roomID: room.roomID,
            name: name,
            description: room.description,
            parentID: room.parentID,
            ephemeral: room.ephemeral,
            createdAt: room.createdAt,
            createdBy: room.createdBy,
            visibility: room.visibility,
            lastActivity: room.lastActivity,
            memberCount: room.memberCount,
            encrypted: room.encrypted
        )
    }
    func join(roomID: String) async throws {
        operations.append("join:\(roomID)")
        if roomID == blockedRoomID {
            await withCheckedContinuation { joinContinuation = $0 }
        }
    }
    func leave(roomID: String) async throws { operations.append("leave:\(roomID)") }
    func history(roomID: String, limit: Int, before: String?) async throws -> [ChatMessage] {
        historyRequests.append((roomID, limit, before))
        let requestNumber = historyRequests.count
        if capturesHistoryRequests {
            return try await withTaskCancellationHandler {
                try Task.checkCancellation()
                return try await withCheckedThrowingContinuation { continuation in
                    if Task.isCancelled {
                        continuation.resume(throwing: CancellationError())
                    } else {
                        capturedHistoryContinuations[requestNumber] = continuation
                    }
                }
            } onCancel: {
                Task { @MainActor [weak self] in
                    self?.capturedHistoryContinuations
                        .removeValue(forKey: requestNumber)?
                        .resume(throwing: CancellationError())
                }
            }
        }
        if blockedHistoryLimit == limit {
            blockedHistoryLimit = nil
            await withCheckedContinuation { historyContinuation = $0 }
        }
        let messages = historiesByRoom[roomID] ?? []
        let eligible: ArraySlice<ChatMessage>
        if let before,
           let cutoff = messages.firstIndex(where: { $0.timestamp == before }) {
            eligible = messages[..<cutoff]
        } else {
            eligible = messages[...]
        }
        return Array(eligible.suffix(limit))
    }
    func send(roomID: String, content: String) async throws -> ChatMessage {
        operations.append("send:\(roomID):\(content)")
        if blocksSend {
            await withCheckedContinuation { sendContinuations[content] = $0 }
        }
        if sendShouldFail {
            throw NSError(domain: "MockRoomConnection", code: 1)
        }
        guard let message = historiesByRoom[roomID]?.last else {
            fatalError("set a response message before sending successfully")
        }
        return message
    }
    func disconnect() {}

    func resumeBlockedJoin() {
        blockedRoomID = nil
        joinContinuation?.resume()
        joinContinuation = nil
    }

    func resumeBlockedConnect() {
        blocksConnect = false
        connectContinuation?.resume()
        connectContinuation = nil
    }

    func resumeBlockedRegistration() {
        registrationContinuation?.resume()
        registrationContinuation = nil
    }

    func resumeBlockedCreate() {
        blocksCreate = false
        createContinuation?.resume()
        createContinuation = nil
    }

    func resumeBlockedDestroy() {
        blocksDestroy = false
        destroyContinuation?.resume()
        destroyContinuation = nil
    }

    func resumeBlockedListRooms() {
        blocksListRooms = false
        listRoomsContinuation?.resume()
        listRoomsContinuation = nil
    }

    func resumeBlockedListAgents() {
        blocksListAgents = false
        listAgentsContinuation?.resume()
        listAgentsContinuation = nil
    }

    func completeListAgentRequest(
        _ requestNumber: Int,
        with result: Result<[AgentPresence], Error>
    ) {
        capturedListAgentContinuations.removeValue(forKey: requestNumber)?.resume(with: result)
    }

    func completeHistoryRequest(
        _ requestNumber: Int,
        with result: Result<[ChatMessage], Error>
    ) {
        capturedHistoryContinuations.removeValue(forKey: requestNumber)?.resume(with: result)
    }

    func resumeBlockedHistory() {
        blockedHistoryLimit = nil
        historyContinuation?.resume()
        historyContinuation = nil
    }

    func resumeBlockedSend(content: String? = nil) {
        if let content, let continuation = sendContinuations.removeValue(forKey: content) {
            continuation.resume()
            return
        }
        blocksSend = false
        let continuations = sendContinuations.values
        sendContinuations.removeAll()
        for continuation in continuations { continuation.resume() }
    }
}

private final class RoomTransitionCredentialStore: CowchatCredentialStore {
    var credential: String?
    var readError: Error?
    var writeError: Error?

    func credential(for _: String) throws -> String? {
        if let readError { throw readError }
        return credential
    }

    func setCredential(_ credential: String?, for _: String) throws {
        if let writeError { throw writeError }
        self.credential = credential
    }
}

@MainActor
private final class MockLocalServerSupervisor: LocalServerSupervising {
    var ownsRunningServer = false
    var launchCount = 0
    var stopCount = 0
    var appTerminationShutdownCount = 0
    var explicitRetryCount = 0
    var launchError: Error?

    func launchIfNeeded() async throws {
        launchCount += 1
        if let launchError { throw launchError }
        ownsRunningServer = true
    }

    func stopOwnedServer() {
        stopCount += 1
        ownsRunningServer = false
    }

    func shutdownOwnedServerForAppTermination() async {
        appTerminationShutdownCount += 1
        stopOwnedServer()
    }

    func prepareForExplicitRetry() {
        explicitRetryCount += 1
    }
}

final class RoomTransitionTests: XCTestCase {
    @MainActor
    private func makeStore(
        connection: MockRoomConnection,
        memberRefreshRetryDelayNanoseconds: UInt64 = 2_000_000_000
    ) -> ChatStore {
        let suiteName = "RoomTransitionTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(true, forKey: ChatStore.didCreateDefaultRoomKey)
        return ChatStore(
            connection: connection,
            defaults: defaults,
            memberRefreshRetryDelayNanoseconds: memberRefreshRetryDelayNanoseconds
        )
    }

    @MainActor
    private func makeFreshInstallStore(connection: MockRoomConnection) -> ChatStore {
        let suiteName = "RoomTransitionTests.fresh.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        return ChatStore(connection: connection, defaults: defaults)
    }

    @MainActor
    private func makeProfileSwitchingStore(connection: MockRoomConnection) -> ChatStore {
        let suiteName = "RoomTransitionTests.profile.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(true, forKey: ChatStore.didCreateDefaultRoomKey)
        defaults.set("profile-owner", forKey: "CowchatMac.agentID")
        let preferences = ConnectionProfilePreferences(
            defaults: defaults,
            credentialStore: RoomTransitionCredentialStore()
        )
        return ChatStore(
            connection: connection,
            defaults: defaults,
            connectionPreferences: preferences,
            localServerRetryDelaysNanoseconds: []
        )
    }

    @MainActor
    private func switchToTestCloud(_ store: ChatStore, connection: MockRoomConnection) {
        connection.connectFailuresRemaining = 1
        XCTAssertTrue(
            store.saveAndUseCowchatCloud(
                url: "wss://cloud.example/ws",
                apiKey: "profile-switch-key"
            )
        )
        XCTAssertEqual(store.connectionProfile.kind, .cowchatCloud)
    }

    @MainActor
    func testExistingLocalServerConnectsWithoutLaunchingBundledServer() async {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        let defaults = UserDefaults(suiteName: "RoomTransitionTests.\(UUID().uuidString)")!
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: [0]
        )

        await store.connect()

        XCTAssertEqual(connection.connectCallCount, 1)
        XCTAssertEqual(supervisor.launchCount, 0)
        XCTAssertEqual(store.connectionStatus, .connected)
    }

    @MainActor
    func testMissingLocalServerLaunchesOnceAndRetriesTransport() async {
        let connection = MockRoomConnection()
        connection.connectFailuresRemaining = 1
        let supervisor = MockLocalServerSupervisor()
        let defaults = UserDefaults(suiteName: "RoomTransitionTests.\(UUID().uuidString)")!
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: [0]
        )

        await store.connect()

        XCTAssertEqual(connection.connectCallCount, 2)
        XCTAssertEqual(supervisor.launchCount, 1)
        XCTAssertTrue(supervisor.ownsRunningServer)
        XCTAssertEqual(store.connectionStatus, .connected)
    }

    @MainActor
    func testRegistrationFailureDoesNotLaunchBundledServer() async {
        let connection = MockRoomConnection()
        connection.registrationError = CowchatConnectionError.server(message: "Protocol mismatch", code: nil)
        let supervisor = MockLocalServerSupervisor()
        let defaults = UserDefaults(suiteName: "RoomTransitionTests.\(UUID().uuidString)")!
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: [0]
        )

        await store.connect()

        XCTAssertEqual(connection.connectCallCount, 1)
        XCTAssertEqual(supervisor.launchCount, 0)
        XCTAssertEqual(store.connectionStatus, .failed("Protocol mismatch"))
    }

    @MainActor
    func testCloudTransportFailureNeverLaunchesLocalServer() async throws {
        let connection = MockRoomConnection()
        connection.connectFailuresRemaining = 1
        let supervisor = MockLocalServerSupervisor()
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "test-key"
        )
        let defaults = UserDefaults(suiteName: "RoomTransitionTests.\(UUID().uuidString)")!
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            connectionProfile: profile,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: [0]
        )

        await store.connect()

        XCTAssertEqual(connection.connectCallCount, 1)
        XCTAssertEqual(supervisor.launchCount, 0)
        XCTAssertFalse(store.connectionStatus.isConnected)
    }

    @MainActor
    func testUnavailableSelectedCloudNeverFallsBackToLocalServer() async {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        let profile = ConnectionProfile.unavailableCowchatCloud(
            urlString: "wss://cloud.example/ws"
        )
        let configurationError = ConnectionProfileError.missingAPIKey
        let defaults = UserDefaults(suiteName: "RoomTransitionTests.\(UUID().uuidString)")!
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            connectionProfile: profile,
            localServerSupervisor: supervisor,
            connectionConfigurationError: configurationError,
            localServerRetryDelaysNanoseconds: [0]
        )

        store.start()
        for _ in 0..<10 { await Task.yield() }

        XCTAssertEqual(store.connectionProfile.kind, .cowchatCloud)
        XCTAssertEqual(connection.connectCallCount, 0)
        XCTAssertEqual(supervisor.launchCount, 0)
        XCTAssertEqual(store.connectionStatus, .failed(configurationError.localizedDescription))
    }

    @MainActor
    func testManualReconnectReloadsTransientlyUnavailableSelectedCloudProfile() async throws {
        let suiteName = "RoomTransitionTests.cloud-reload.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        let credentialStore = RoomTransitionCredentialStore()
        let preferences = ConnectionProfilePreferences(
            defaults: defaults,
            credentialStore: credentialStore
        )
        let savedProfile = try preferences.save(
            ConnectionProfile.cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "transient-key"
            )
        )
        let transientError = NSError(
            domain: "RoomTransitionTests.Keychain",
            code: 2,
            userInfo: [NSLocalizedDescriptionKey: "Keychain is temporarily unavailable"]
        )
        credentialStore.readError = transientError
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            connectionProfile: .unavailableCowchatCloud(
                urlString: preferences.loadSavedCloudURL()
            ),
            connectionPreferences: preferences,
            localServerSupervisor: supervisor,
            connectionConfigurationError: transientError,
            localServerRetryDelaysNanoseconds: []
        )

        credentialStore.readError = nil
        store.reconnect()
        for _ in 0..<100 where !store.connectionStatus.isConnected {
            await Task.yield()
        }

        XCTAssertEqual(store.connectionProfile, savedProfile)
        XCTAssertEqual(store.connectionStatus, .connected)
        XCTAssertEqual(connection.connectCallCount, 1)
        XCTAssertEqual(connection.registeredAgentIDs.count, 1)
        XCTAssertEqual(supervisor.launchCount, 0)
        XCTAssertNil(store.errorMessage)
    }

    @MainActor
    func testRepeatedStartCallsShareOneConnectionAttempt() async {
        let connection = MockRoomConnection()
        connection.blocksConnect = true
        let defaults = UserDefaults(suiteName: "RoomTransitionTests.\(UUID().uuidString)")!
        let store = ChatStore(connection: connection, defaults: defaults)

        store.start()
        store.start()
        while connection.connectCallCount == 0 { await Task.yield() }

        XCTAssertEqual(connection.connectCallCount, 1)
        connection.resumeBlockedConnect()
        while !store.connectionStatus.isConnected { await Task.yield() }
        XCTAssertEqual(connection.connectCallCount, 1)
    }

    @MainActor
    func testProfileSwitchBeforeScheduledConnectDoesNotStartCanceledAttempt() async {
        let connection = MockRoomConnection()
        let store = makeProfileSwitchingStore(connection: connection)

        store.start()
        XCTAssertTrue(store.saveAndUseCowchatCloud(
            url: "wss://cloud.example/ws",
            apiKey: "scheduled-switch-key"
        ))
        while !store.connectionStatus.isConnected { await Task.yield() }
        for _ in 0..<10 { await Task.yield() }

        XCTAssertEqual(connection.connectCallCount, 1)
        XCTAssertEqual(connection.registeredAgentIDs.count, 1)
        XCTAssertEqual(store.connectionProfile.kind, .cowchatCloud)
    }

    @MainActor
    func testSwitchingFromLocalToCloudStopsOnlyTheOwnedHelper() async {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        supervisor.ownsRunningServer = true
        let suiteName = "RoomTransitionTests.stop-helper.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        let preferences = ConnectionProfilePreferences(
            defaults: defaults,
            credentialStore: RoomTransitionCredentialStore()
        )
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            connectionPreferences: preferences,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: []
        )

        XCTAssertTrue(store.saveAndUseCowchatCloud(
            url: "wss://cloud.example/ws",
            apiKey: "stop-owned-key"
        ))

        XCTAssertEqual(supervisor.stopCount, 1)
        XCTAssertFalse(supervisor.ownsRunningServer)
    }

    @MainActor
    func testReturningToLocalAndManualReconnectPermitOneExplicitHelperRetry() async throws {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        let cloud = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "retry-key"
        )
        let defaults = UserDefaults(suiteName: "RoomTransitionTests.retry-helper.\(UUID().uuidString)")!
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            connectionProfile: cloud,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: []
        )

        store.useLocalConnection()
        XCTAssertEqual(supervisor.explicitRetryCount, 1)
        while !store.connectionStatus.isConnected { await Task.yield() }

        store.connectionStatus = .failed("transient local failure")
        store.reconnect()
        XCTAssertEqual(supervisor.explicitRetryCount, 2)
    }

    @MainActor
    func testBundledServerFailureSurfacesInErrorMessage() async {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        supervisor.launchError = LocalServerSupervisorError.launchFailed("spawn denied")
        connection.connectFailuresRemaining = 99
        let suiteName = "RoomTransitionTests.spawnfail.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(true, forKey: ChatStore.didCreateDefaultRoomKey)
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: []
        )

        await store.connect()

        XCTAssertNotNil(store.errorMessage)
        XCTAssertTrue(store.errorMessage?.contains("spawn denied") == true)
    }

    @MainActor
    func testLatchedSpawnFailureAlertsOnceNotEveryReconnect() async {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        supervisor.launchError = LocalServerSupervisorError.launchFailed("spawn denied")
        connection.connectFailuresRemaining = 99
        let suiteName = "RoomTransitionTests.spawnfail-repeat.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(true, forKey: ChatStore.didCreateDefaultRoomKey)
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: []
        )

        await store.connect()
        XCTAssertNotNil(store.errorMessage)

        // The user dismisses the alert. A subsequent reconnect attempt that
        // hits the same latched failure must not re-fire it.
        store.errorMessage = nil
        await store.connect()

        XCTAssertNil(store.errorMessage)
    }

    @MainActor
    func testUndismissedSpawnFailureAlertSurvivesBackgroundReconnect() async {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        supervisor.launchError = LocalServerSupervisorError.launchFailed("spawn denied")
        connection.connectFailuresRemaining = 99
        let suiteName = "RoomTransitionTests.spawnfail-undismissed.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(true, forKey: ChatStore.didCreateDefaultRoomKey)
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: []
        )

        await store.connect()
        XCTAssertNotNil(store.errorMessage)

        // The user has NOT dismissed the alert. A background reconnect attempt
        // that hits the same latched failure must not silently clear it.
        await store.connect()

        XCTAssertEqual(store.errorMessage, LocalServerSupervisorError.launchFailed("spawn denied").localizedDescription)
    }

    @MainActor
    func testSpawnFailureAlertClearsWhenBackgroundReconnectSucceeds() async {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        supervisor.launchError = LocalServerSupervisorError.launchFailed("spawn denied")
        connection.connectFailuresRemaining = 99
        let suiteName = "RoomTransitionTests.spawnfail-recovers.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(true, forKey: ChatStore.didCreateDefaultRoomKey)
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: []
        )

        await store.connect()
        XCTAssertNotNil(store.errorMessage)

        // The transient outage self-heals; the user never touched the alert.
        supervisor.launchError = nil
        connection.connectFailuresRemaining = 0
        await store.connect()

        XCTAssertTrue(store.connectionStatus.isConnected)
        XCTAssertNil(store.errorMessage)
    }

    @MainActor
    func testProfileSwitchDiscardsCreateCompletionFromOldServer() async throws {
        let localRoom = try decodeRoom(id: "local-room", name: "Local room", createdBy: "profile-owner")
        let staleCreated = try decodeRoom(id: "stale-created", name: "Stale", createdBy: "profile-owner")
        let connection = MockRoomConnection()
        connection.listedRooms = [localRoom]
        connection.roomToCreate = staleCreated
        let store = makeProfileSwitchingStore(connection: connection)
        await store.connect()
        connection.blocksCreate = true

        let creation = Task {
            await store.createRoom(
                name: "Stale",
                description: "",
                ephemeral: false,
                isPublic: true
            )
        }
        while !connection.operations.contains("create:Stale") { await Task.yield() }

        switchToTestCloud(store, connection: connection)
        connection.resumeBlockedCreate()

        let creationSucceeded = await creation.value
        XCTAssertFalse(creationSucceeded)
        XCTAssertFalse(store.rooms.contains(where: { $0.id == staleCreated.id }))
        XCTAssertNil(store.selectedRoomID)
    }

    @MainActor
    func testManualReconnectDiscardsCreateCompletionFromPriorSession() async throws {
        let staleCreated = try decodeRoom(
            id: "stale-before-reconnect",
            name: "Stale",
            createdBy: "profile-owner"
        )
        let connection = MockRoomConnection()
        connection.roomToCreate = staleCreated
        let store = makeProfileSwitchingStore(connection: connection)
        await store.connect()
        connection.blocksCreate = true

        let creation = Task {
            await store.createRoom(
                name: "Stale",
                description: "",
                ephemeral: false,
                isPublic: true
            )
        }
        while !connection.operations.contains("create:Stale") { await Task.yield() }

        store.connectionStatus = .failed("transport dropped")
        store.reconnect()
        connection.resumeBlockedCreate()

        let creationSucceeded = await creation.value
        XCTAssertFalse(creationSucceeded)
        XCTAssertFalse(store.rooms.contains(where: { $0.id == staleCreated.id }))
    }

    @MainActor
    func testChangingCloudCredentialUsesANewStableAgentIdentity() async {
        let connection = MockRoomConnection()
        let store = makeProfileSwitchingStore(connection: connection)

        XCTAssertTrue(store.saveAndUseCowchatCloud(
            url: "wss://cloud.example/ws",
            apiKey: "first-key"
        ))
        while connection.registeredAgentIDs.count < 1 { await Task.yield() }
        let firstProfile = store.connectionProfile
        let firstAgentID = connection.registeredAgentIDs[0]

        XCTAssertTrue(store.saveAndUseCowchatCloud(
            url: "wss://cloud.example/ws",
            apiKey: "second-key"
        ))
        while connection.registeredAgentIDs.count < 2 { await Task.yield() }
        let secondProfile = store.connectionProfile
        let secondAgentID = connection.registeredAgentIDs[1]

        XCTAssertNotEqual(firstProfile.cloudAccountID, secondProfile.cloudAccountID)
        XCTAssertNotEqual(firstAgentID, secondAgentID)
        XCTAssertTrue(firstAgentID.hasPrefix("cowchat-mac-"))
        XCTAssertTrue(secondAgentID.hasPrefix("cowchat-mac-"))
    }

    @MainActor
    func testProfileSwitchDuringRegistrationCannotRestoreOldIdentity() async {
        let connection = MockRoomConnection()
        connection.blocksNextRegistration = true
        let store = makeProfileSwitchingStore(connection: connection)

        store.start()
        while connection.registeredAgentIDs.count < 1 { await Task.yield() }
        let localAgentID = connection.registeredAgentIDs[0]

        XCTAssertTrue(store.saveAndUseCowchatCloud(
            url: "wss://cloud.example/ws",
            apiKey: "cloud-key"
        ))
        while connection.registeredAgentIDs.count < 2 { await Task.yield() }
        let cloudAgentID = connection.registeredAgentIDs[1]
        while !store.connectionStatus.isConnected { await Task.yield() }

        connection.resumeBlockedRegistration()
        for _ in 0..<10 { await Task.yield() }

        XCTAssertNotEqual(localAgentID, cloudAgentID)
        XCTAssertEqual(store.connectionProfile.kind, .cowchatCloud)
        XCTAssertEqual(store.agentID, cloudAgentID)
    }

    @MainActor
    func testCloudSaveFailureReturnsFalseAndKeepsCurrentProfile() async {
        let suiteName = "RoomTransitionTests.save-failure.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        let credentialStore = RoomTransitionCredentialStore()
        let preferences = ConnectionProfilePreferences(
            defaults: defaults,
            credentialStore: credentialStore
        )
        let connection = MockRoomConnection()
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            connectionPreferences: preferences
        )

        XCTAssertTrue(store.saveAndUseCowchatCloud(
            url: "wss://cloud.example/ws",
            apiKey: "stable-key"
        ))
        while connection.registeredAgentIDs.isEmpty { await Task.yield() }
        let originalProfile = store.connectionProfile
        credentialStore.writeError = NSError(
            domain: "RoomTransitionTests.Keychain",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: "Injected Keychain write failure"]
        )

        XCTAssertFalse(store.saveAndUseCowchatCloud(
            url: "wss://cloud.example/ws",
            apiKey: "stable-key"
        ))
        XCTAssertEqual(store.connectionProfile, originalProfile)
        XCTAssertEqual(store.errorMessage, "Injected Keychain write failure")
    }

    @MainActor
    func testProfileSwitchDoesNotRestoreFailedDraftFromOldServer() async throws {
        let localRoom = try decodeRoom(id: "local-room", name: "Local room", createdBy: "profile-owner")
        let connection = MockRoomConnection()
        connection.listedRooms = [localRoom]
        connection.historiesByRoom[localRoom.id] = [try decodeMessage(
            id: "reply",
            roomID: localRoom.id,
            content: "sent",
            timestamp: "2026-08-04T12:00:00Z",
            sequence: 1
        )]
        let store = makeProfileSwitchingStore(connection: connection)
        await store.connect()
        connection.blocksSend = true
        connection.sendShouldFail = true
        store.draft = "belongs to local"

        store.sendDraft()
        while !connection.operations.contains("send:local-room:belongs to local") {
            await Task.yield()
        }

        switchToTestCloud(store, connection: connection)
        connection.resumeBlockedSend(content: "belongs to local")
        for _ in 0..<10 { await Task.yield() }

        XCTAssertEqual(store.draft, "")
        XCTAssertNil(store.errorMessage)
        XCTAssertTrue(store.rooms.isEmpty)
    }

    @MainActor
    func testProfileSwitchDoesNotRemoveSameIDRoomAfterOldDestroyCompletes() async throws {
        let localRoom = try decodeRoom(id: "shared-id", name: "Local room", createdBy: "profile-owner")
        let cloudRoom = try decodeRoom(id: "shared-id", name: "Cloud room", createdBy: "another-agent")
        let connection = MockRoomConnection()
        connection.listedRooms = [localRoom]
        let store = makeProfileSwitchingStore(connection: connection)
        await store.connect()
        connection.blocksDestroy = true

        let destruction = Task { await store.destroy(localRoom) }
        while !connection.operations.contains("destroy:shared-id") { await Task.yield() }

        switchToTestCloud(store, connection: connection)
        store.rooms = [cloudRoom]
        connection.resumeBlockedDestroy()

        let destructionSucceeded = await destruction.value
        XCTAssertFalse(destructionSucceeded)
        XCTAssertEqual(store.rooms.map(\.name), ["Cloud room"])
    }

    @MainActor
    func testProfileSwitchDiscardsRoomRefreshFromOldServer() async throws {
        let staleRoom = try decodeRoom(id: "stale", name: "Stale")
        let cloudRoom = try decodeRoom(id: "cloud", name: "Cloud")
        let connection = MockRoomConnection()
        connection.listedRooms = [staleRoom]
        connection.blocksListRooms = true
        let store = makeProfileSwitchingStore(connection: connection)

        let refresh = Task { try? await store.refreshRooms() }
        while !connection.operations.contains("listRooms") { await Task.yield() }

        switchToTestCloud(store, connection: connection)
        store.rooms = [cloudRoom]
        connection.resumeBlockedListRooms()
        await refresh.value

        XCTAssertEqual(store.rooms.map(\.id), ["cloud"])
    }

    @MainActor
    func testRoomRefreshPreservesRoomCreatedWhileRequestIsInFlight() async throws {
        let existingData = Data(#"""
        {"room_id":"existing","name":"existing","created_at":"2026-07-11T12:00:00Z","visibility":"public"}
        """#.utf8)
        let existing = try JSONDecoder().decode(Room.self, from: existingData)
        let connection = MockRoomConnection()
        connection.listedRooms = [existing]
        connection.blocksListRooms = true
        let store = makeStore(connection: connection)
        store.rooms = [existing]

        let refresh = Task { try await store.refreshRooms() }
        while !connection.operations.contains("listRooms") { await Task.yield() }
        connection.onEvent?("room_created", [
            "room_id": "new-room",
            "name": "new-room",
            "created_at": "2026-08-04T21:50:00Z",
            "visibility": "public",
        ])
        connection.resumeBlockedListRooms()
        try await refresh.value

        XCTAssertEqual(Set(store.rooms.map(\.id)), ["existing", "new-room"])
    }

    @MainActor
    func testRoomListRefreshUpdatesUnselectedActivityAndMembership() async throws {
        func room(_ id: String, activity: String, members: Int) throws -> Room {
            let data = try JSONSerialization.data(withJSONObject: [
                "room_id": id,
                "name": id,
                "ephemeral": false,
                "created_at": "2026-07-11T12:00:00Z",
                "visibility": "public",
                "last_activity": activity,
                "member_count": members,
            ])
            return try JSONDecoder().decode(Room.self, from: data)
        }

        let stale = try room("unselected", activity: "2026-07-11T12:00:00Z", members: 0)
        let fresh = try room("unselected", activity: "2026-08-04T21:45:00Z", members: 3)
        let connection = MockRoomConnection()
        connection.listedRooms = [fresh]
        let store = makeStore(connection: connection)
        store.rooms = [stale]

        try await store.refreshRooms()

        XCTAssertEqual(store.rooms.first?.lastActivity, "2026-08-04T21:45:00Z")
        XCTAssertEqual(store.rooms.first?.memberCount, 3)
    }

    @MainActor
    func testReconnectFallsBackWhenPreviouslySelectedRoomNoLongerExists() async throws {
        let lobby = try decodeRoom(id: "lobby", name: "Lobby")
        let connection = MockRoomConnection()
        connection.listedRooms = [lobby]
        let store = makeStore(connection: connection)
        store.selectedRoomID = "destroyed-while-offline"

        await store.connect()

        XCTAssertEqual(store.selectedRoomID, lobby.id)
        XCTAssertEqual(connection.operations, ["listRooms", "join:lobby"])
    }

    @MainActor
    func testReconnectLeavesRestoredRoomBeforeJoiningOfflineSelection() async throws {
        let lobby = try decodeRoom(id: "lobby", name: "Lobby")
        let roomA = try decodeRoom(id: "A", name: "A")
        let roomB = try decodeRoom(id: "B", name: "B")
        let connection = MockRoomConnection()
        connection.registration = CowchatRegistration(
            agentID: "stable-agent",
            restoredRoomIDs: [roomA.id]
        )
        connection.listedRooms = [lobby, roomA, roomB]
        let store = makeStore(connection: connection)
        store.selectedRoomID = roomB.id

        await store.connect()

        XCTAssertEqual(store.selectedRoomID, roomB.id)
        XCTAssertEqual(connection.operations, ["leave:A", "listRooms", "join:B"])
    }

    @MainActor
    func testJoiningRoomSynchronizesActiveMemberCount() async throws {
        let roomData = Data(#"""
        {
          "room_id":"room",
          "name":"room",
          "ephemeral":false,
          "created_at":"2026-07-11T12:00:00Z",
          "visibility":"public"
        }
        """#.utf8)
        let agentData = Data(#"""
        {
          "agent_id":"agent",
          "name":"Agent",
          "capabilities":[]
        }
        """#.utf8)
        let room = try JSONDecoder().decode(Room.self, from: roomData)
        let agent = try JSONDecoder().decode(AgentPresence.self, from: agentData)
        let connection = MockRoomConnection()
        connection.agentsByRoom[room.id] = [agent]
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected

        await store.select(room: room)

        XCTAssertEqual(store.rooms.first?.memberCount, 1)
    }

    @MainActor
    func testJoiningRoomPublishesMembersBeforeSlowHistoryFinishes() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let collaborator = try decodeAgent(id: "collaborator", name: "Claude")
        let connection = MockRoomConnection()
        connection.agentsByRoom[room.id] = [collaborator]
        connection.blockedHistoryLimit = 100
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected

        let selection = Task { await store.select(room: room) }
        while !connection.historyRequests.contains(where: { $0.roomID == room.id && $0.limit == 100 }) {
            await Task.yield()
        }
        for _ in 0..<100 where store.roomMembers.isEmpty {
            await Task.yield()
        }

        XCTAssertEqual(store.roomMembers.map(\.id), [collaborator.id])

        connection.resumeBlockedHistory()
        await selection.value
    }

    @MainActor
    func testJoiningRoomPublishesHistoryBeforeSlowMemberRefreshFinishes() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let message = try decodeMessage(
            id: "existing-message",
            roomID: room.id,
            content: "Existing history",
            timestamp: "2026-08-04T12:00:00Z",
            sequence: 1
        )
        let connection = MockRoomConnection()
        connection.historiesByRoom[room.id] = [message]
        connection.blocksListAgents = true
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected

        let selection = Task { await store.select(room: room) }
        for _ in 0..<100 where store.messages.isEmpty {
            await Task.yield()
        }

        XCTAssertEqual(store.messages.map(\.id), [message.id])
        XCTAssertFalse(store.isLoadingMessages)

        connection.resumeBlockedListAgents()
        await selection.value
    }

    @MainActor
    func testFreshJoinDoesNotSubtractMacFromPreJoinFallbackCount() async throws {
        let room = try decodeRoom(id: "room", name: "room", memberCount: 1)
        let connection = MockRoomConnection()
        connection.blocksListAgents = true
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected

        let selection = Task { await store.select(room: room) }
        while connection.listAgentRequests.isEmpty { await Task.yield() }

        XCTAssertTrue(connection.operations.contains("join:\(room.id)"))
        XCTAssertFalse(store.fallbackMemberCountIncludesCurrentAgent(in: room.id))
        XCTAssertEqual(store.rooms.first?.memberCount, 1)
        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: store.roomMembers,
                currentAgentID: store.agentID,
                fallbackMemberCount: store.rooms.first?.memberCount,
                fallbackMemberCountIncludesCurrentAgent:
                    store.fallbackMemberCountIncludesCurrentAgent(in: room.id)
            ),
            "1 agent connected"
        )

        connection.resumeBlockedListAgents()
        await selection.value
    }

    @MainActor
    func testRoomRefreshReconcilesMembersForSelectedRoomWithHistory() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let collaborator = try decodeAgent(id: "collaborator", name: "Claude")
        let connection = MockRoomConnection()
        connection.listedRooms = [room]
        connection.historiesByRoom[room.id] = [
            try decodeMessage(
                id: "existing-message",
                roomID: room.id,
                content: "Existing history",
                timestamp: "2026-08-04T12:00:00Z",
                sequence: 1
            ),
        ]
        let store = makeStore(connection: connection)

        await store.connect()
        XCTAssertEqual(store.messages.count, 1)
        XCTAssertTrue(store.roomMembers.isEmpty)

        connection.agentsByRoom[room.id] = [collaborator]
        try await store.refreshRooms()

        XCTAssertEqual(store.roomMembers.map(\.id), [collaborator.id])
    }

    @MainActor
    func testUnknownWorkingPresenceRecordsActivityAndRepairsMemberSnapshot() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let collaborator = AgentPresence(
            agentID: "collaborator",
            name: "Claude",
            status: "working"
        )
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected
        await store.select(room: room)
        connection.agentsByRoom[room.id] = [collaborator]
        let requestsBeforePresence = connection.listAgentRequests.count

        connection.onEvent?("presence_update", [
            "agent_id": collaborator.id,
            "agent_name": collaborator.name,
            "status": "working",
        ])
        for _ in 0..<100 where !store.roomMembers.contains(where: { $0.id == collaborator.id }) {
            await Task.yield()
        }

        XCTAssertGreaterThan(connection.listAgentRequests.count, requestsBeforePresence)
        XCTAssertNotNil(store.recentAgentActivityAt[room.id]?[collaborator.id])
        XCTAssertEqual(store.roomMembers.map(\.id), [collaborator.id])
    }

    @MainActor
    func testRecentActivityIsRestoredFromOneShotMessageHistory() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let now = Date()
        let connection = MockRoomConnection()
        connection.historiesByRoom[room.id] = [
            try decodeMessage(
                id: "normal-message",
                roomID: room.id,
                content: "Starting the review",
                timestamp: ISO8601DateFormatter().string(from: now.addingTimeInterval(-30)),
                sequence: 1,
                agentID: "claude",
                agentName: "Claude"
            ),
            try decodeMessage(
                id: "thinking-message",
                roomID: room.id,
                content: "Reviewing",
                timestamp: ISO8601DateFormatter().string(from: now.addingTimeInterval(-20)),
                sequence: 2,
                agentID: "codex",
                agentName: "Codex",
                metadataType: "thinking"
            ),
        ]
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected

        await store.select(room: room)

        XCTAssertEqual(store.messages.map(\.id), ["normal-message"])
        XCTAssertEqual(
            Set(store.recentAgentActivityAt[room.id].map { Array($0.keys) } ?? []),
            ["claude", "codex"]
        )
        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [],
                currentAgentID: store.agentID,
                fallbackMemberCount: 0,
                fallbackMemberCountIncludesCurrentAgent: true,
                recentActivityByAgent: store.recentAgentActivityAt[room.id],
                now: now
            ),
            "2 agents active recently"
        )
    }

    @MainActor
    func testNormalMessageDoesNotEraseRecentWorkingPresence() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let now = Date()
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected
        await store.select(room: room)

        connection.onEvent?("presence_update", [
            "agent_id": "collaborator",
            "agent_name": "Claude",
            "status": "working",
        ])
        connection.onEvent?("message_received", [
            "message_id": "message",
            "room_id": room.id,
            "agent_id": "collaborator",
            "agent_name": "Claude",
            "content": "Still working",
            "timestamp": ISO8601DateFormatter().string(from: now),
            "seq": 1,
        ])

        XCTAssertNotNil(store.recentAgentActivityAt[room.id]?["collaborator"])
        XCTAssertNil(store.lastThinkingAt[room.id]?["collaborator"])
    }

    @MainActor
    func testMemberRefreshCoalescesPresenceChurnAndAppliesOneDirtyRerun() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let existingMessage = try decodeMessage(
            id: "existing",
            roomID: room.id,
            content: "Existing history",
            timestamp: "2026-08-04T12:00:00Z",
            sequence: 1
        )
        let olderAgent = try decodeAgent(id: "older", name: "Older")
        let newerAgent = try decodeAgent(id: "newer", name: "Newer")
        let connection = MockRoomConnection()
        connection.historiesByRoom[room.id] = [existingMessage]
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected
        await store.select(room: room)
        connection.capturesListAgentRequests = true

        let firstRequest = connection.listAgentRequests.count + 1
        for index in 0..<25 {
            connection.onEvent?("presence_update", [
                "agent_id": "unknown-\(index)",
                "agent_name": "Unknown \(index)",
                "status": "working",
            ])
        }
        while connection.listAgentRequests.count < firstRequest { await Task.yield() }
        for _ in 0..<20 { await Task.yield() }
        XCTAssertEqual(connection.listAgentRequests.count, firstRequest)

        for index in 25..<50 {
            connection.onEvent?("presence_update", [
                "agent_id": "unknown-\(index)",
                "agent_name": "Unknown \(index)",
                "status": "working",
            ])
        }
        for _ in 0..<20 { await Task.yield() }
        XCTAssertEqual(connection.listAgentRequests.count, firstRequest)

        connection.completeListAgentRequest(firstRequest, with: .success([olderAgent]))
        let dirtyRerun = firstRequest + 1
        while connection.listAgentRequests.count < dirtyRerun { await Task.yield() }
        XCTAssertEqual(connection.listAgentRequests.count, dirtyRerun)

        connection.completeListAgentRequest(dirtyRerun, with: .success([newerAgent]))
        for _ in 0..<100 where store.roomMembers.map(\.id) != [newerAgent.id] {
            await Task.yield()
        }
        XCTAssertEqual(store.roomMembers.map(\.id), [newerAgent.id])
        XCTAssertEqual(connection.listAgentRequests.count, dirtyRerun)
    }

    @MainActor
    func testMemberRefreshRetriesOneTransientFailureAndAppliesTheResult() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let existingMessage = try decodeMessage(
            id: "existing",
            roomID: room.id,
            content: "Existing history",
            timestamp: "2026-08-04T12:00:00Z",
            sequence: 1
        )
        let collaborator = try decodeAgent(id: "collaborator", name: "Claude")
        let connection = MockRoomConnection()
        connection.historiesByRoom[room.id] = [existingMessage]
        let store = makeStore(
            connection: connection,
            memberRefreshRetryDelayNanoseconds: 0
        )
        store.rooms = [room]
        store.connectionStatus = .connected
        await store.select(room: room)
        connection.capturesListAgentRequests = true

        let firstRequest = connection.listAgentRequests.count + 1
        connection.onEvent?("agent_joined", ["room_id": room.id])
        while connection.listAgentRequests.count < firstRequest { await Task.yield() }
        connection.completeListAgentRequest(
            firstRequest,
            with: .failure(CowchatConnectionError.timeout)
        )

        let retryRequest = firstRequest + 1
        while connection.listAgentRequests.count < retryRequest { await Task.yield() }
        connection.completeListAgentRequest(retryRequest, with: .success([collaborator]))
        for _ in 0..<100 where store.roomMembers.map(\.id) != [collaborator.id] {
            await Task.yield()
        }

        XCTAssertEqual(store.roomMembers.map(\.id), [collaborator.id])
        XCTAssertEqual(connection.listAgentRequests.count, retryRequest)
    }

    @MainActor
    func testMemberRefreshFailureGetsOnlyOneAutomaticRetry() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let existingMessage = try decodeMessage(
            id: "existing",
            roomID: room.id,
            content: "Existing history",
            timestamp: "2026-08-04T12:00:00Z",
            sequence: 1
        )
        let connection = MockRoomConnection()
        connection.historiesByRoom[room.id] = [existingMessage]
        let store = makeStore(
            connection: connection,
            memberRefreshRetryDelayNanoseconds: 0
        )
        store.rooms = [room]
        store.connectionStatus = .connected
        await store.select(room: room)
        connection.capturesListAgentRequests = true

        let firstRequest = connection.listAgentRequests.count + 1
        connection.onEvent?("agent_joined", ["room_id": room.id])
        while connection.listAgentRequests.count < firstRequest { await Task.yield() }
        connection.completeListAgentRequest(
            firstRequest,
            with: .failure(CowchatConnectionError.timeout)
        )

        let retryRequest = firstRequest + 1
        while connection.listAgentRequests.count < retryRequest { await Task.yield() }
        connection.completeListAgentRequest(
            retryRequest,
            with: .failure(CowchatConnectionError.timeout)
        )
        for _ in 0..<100 { await Task.yield() }

        XCTAssertEqual(connection.listAgentRequests.count, retryRequest)
        XCTAssertTrue(store.roomMembers.isEmpty)
    }

    @MainActor
    func testRoomSwitchStartsNewMemberRefreshBeforeOldRequestCompletes() async throws {
        let roomA = try decodeRoom(id: "A", name: "A")
        let roomB = try decodeRoom(id: "B", name: "B")
        let memberB = try decodeAgent(id: "member-b", name: "Member B")
        let connection = MockRoomConnection()
        connection.historiesByRoom = [
            roomA.id: [
                try decodeMessage(
                    id: "message-a",
                    roomID: roomA.id,
                    content: "A",
                    timestamp: "2026-08-04T12:00:00Z",
                    sequence: 1
                ),
            ],
            roomB.id: [
                try decodeMessage(
                    id: "message-b",
                    roomID: roomB.id,
                    content: "B",
                    timestamp: "2026-08-04T12:00:01Z",
                    sequence: 2
                ),
            ],
        ]
        let store = makeStore(connection: connection)
        store.rooms = [roomA, roomB]
        store.connectionStatus = .connected
        connection.capturesListAgentRequests = true

        let staleSelection = Task { await store.select(room: roomA) }
        let staleRequest = 1
        for _ in 0..<100 where connection.listAgentRequests.count < staleRequest {
            await Task.yield()
        }
        XCTAssertEqual(connection.listAgentRequests.count, staleRequest)

        let selection = Task { await store.select(room: roomB) }
        let newRoomRequest = staleRequest + 1
        for _ in 0..<100 where connection.listAgentRequests.count < newRoomRequest {
            await Task.yield()
        }
        let startedBeforeStaleRequestCompleted =
            connection.listAgentRequests.count >= newRoomRequest
        XCTAssertTrue(
            startedBeforeStaleRequestCompleted,
            "the new room must not wait for the stale room's request to time out"
        )
        if !startedBeforeStaleRequestCompleted {
            // Let a regressed implementation unwind so the test reports the
            // assertion cleanly instead of leaking its captured continuation.
            connection.completeListAgentRequest(
                staleRequest,
                with: .failure(CowchatConnectionError.timeout)
            )
            for _ in 0..<100 where connection.listAgentRequests.count < newRoomRequest {
                await Task.yield()
            }
        }
        guard connection.listAgentRequests.count >= newRoomRequest else {
            selection.cancel()
            staleSelection.cancel()
            return XCTFail("the new room never started its member refresh")
        }
        XCTAssertEqual(connection.listAgentRequests[newRoomRequest - 1], roomB.id)
        connection.completeListAgentRequest(newRoomRequest, with: .success([memberB]))
        await selection.value
        await staleSelection.value

        XCTAssertEqual(store.selectedRoomID, roomB.id)
        XCTAssertEqual(store.roomMembers.map(\.id), [memberB.id])
        XCTAssertEqual(
            connection.listAgentRequests.filter { $0 == roomA.id }.count,
            1
        )
    }

    @MainActor
    func testRoomSwitchCancelsStaleHistoryWithoutSkippingSerializedLeave() async throws {
        let roomA = try decodeRoom(id: "A", name: "A")
        let roomB = try decodeRoom(id: "B", name: "B")
        let messageA = try decodeMessage(
            id: "message-a",
            roomID: roomA.id,
            content: "stale A history",
            timestamp: "2026-08-04T12:00:00Z",
            sequence: 1
        )
        let messageB = try decodeMessage(
            id: "message-b",
            roomID: roomB.id,
            content: "current B history",
            timestamp: "2026-08-04T12:00:01Z",
            sequence: 2
        )
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.rooms = [roomA, roomB]
        store.connectionStatus = .connected
        connection.capturesHistoryRequests = true

        let staleSelection = Task { await store.select(room: roomA) }
        for _ in 0..<100 where connection.historyRequests.isEmpty {
            await Task.yield()
        }
        XCTAssertEqual(connection.historyRequests.first?.roomID, roomA.id)

        let selection = Task { await store.select(room: roomB) }
        for _ in 0..<100 where connection.historyRequests.count < 2 {
            await Task.yield()
        }
        let startedBeforeStaleHistoryCompleted = connection.historyRequests.count >= 2
        XCTAssertTrue(
            startedBeforeStaleHistoryCompleted,
            "the new room must cancel a stale history request instead of waiting for its timeout"
        )
        if !startedBeforeStaleHistoryCompleted {
            // Let a regressed implementation unwind so the test reports the
            // assertion cleanly instead of leaking its captured continuation.
            connection.completeHistoryRequest(1, with: .success([messageA]))
            for _ in 0..<100 where connection.historyRequests.count < 2 {
                await Task.yield()
            }
        }
        guard connection.historyRequests.count >= 2 else {
            selection.cancel()
            staleSelection.cancel()
            return XCTFail("the new room never started its history request")
        }

        XCTAssertEqual(connection.historyRequests[1].roomID, roomB.id)
        connection.completeHistoryRequest(2, with: .success([messageB]))
        await selection.value
        await staleSelection.value

        XCTAssertEqual(connection.operations, ["join:A", "leave:A", "join:B"])
        XCTAssertEqual(store.selectedRoomID, roomB.id)
        XCTAssertEqual(store.messages.map(\.id), [messageB.id])
    }

    @MainActor
    func testMemberRefreshEventStormDoesNotBypassRetryBackoff() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let connection = MockRoomConnection()
        let store = makeStore(
            connection: connection,
            memberRefreshRetryDelayNanoseconds: 60_000_000_000
        )
        store.rooms = [room]
        store.connectionStatus = .connected
        await store.select(room: room)
        connection.capturesListAgentRequests = true

        let firstRequest = connection.listAgentRequests.count + 1
        connection.onEvent?("agent_joined", ["room_id": room.id])
        for _ in 0..<100 where connection.listAgentRequests.count < firstRequest {
            await Task.yield()
        }
        XCTAssertEqual(connection.listAgentRequests.count, firstRequest)
        connection.completeListAgentRequest(
            firstRequest,
            with: .failure(CowchatConnectionError.timeout)
        )
        // Let the failed coordinator install its delayed retry before the
        // next event burst arrives.
        for _ in 0..<20 { await Task.yield() }

        for index in 0..<50 {
            connection.onEvent?("presence_update", [
                "agent_id": "unknown-\(index)",
                "agent_name": "Unknown \(index)",
                "status": "working",
            ])
        }
        for _ in 0..<100 { await Task.yield() }

        XCTAssertEqual(
            connection.listAgentRequests.count,
            firstRequest,
            "events during backoff must coalesce into the scheduled retry"
        )
        connection.onStatusChange?(.disconnected)
        if connection.listAgentRequests.count > firstRequest {
            for request in (firstRequest + 1)...connection.listAgentRequests.count {
                connection.completeListAgentRequest(
                    request,
                    with: .failure(CancellationError())
                )
            }
        }
    }

    @MainActor
    func testConnectionFailureInvalidatesInFlightMemberRefresh() async throws {
        let room = try decodeRoom(id: "room", name: "room")
        let collaborator = try decodeAgent(id: "collaborator", name: "Claude")
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected
        await store.select(room: room)
        connection.capturesListAgentRequests = true

        let request = connection.listAgentRequests.count + 1
        connection.onEvent?("agent_joined", ["room_id": room.id])
        while connection.listAgentRequests.count < request { await Task.yield() }

        connection.onStatusChange?(.failed("offline"))
        connection.completeListAgentRequest(request, with: .success([collaborator]))
        for _ in 0..<20 { await Task.yield() }

        XCTAssertTrue(store.roomMembers.isEmpty)
        XCTAssertFalse(store.connectionStatus.isConnected)
    }

    @MainActor
    func testSwitchingRoomsRemovesThisClientFromPreviousActiveCount() async throws {
        func room(_ id: String) throws -> Room {
            let data = try JSONSerialization.data(withJSONObject: [
                "room_id": id,
                "name": id,
                "ephemeral": false,
                "created_at": "2026-07-11T12:00:00Z",
                "visibility": "public",
            ])
            return try JSONDecoder().decode(Room.self, from: data)
        }

        let agentData = Data(#"""
        {"agent_id":"agent","name":"Agent","capabilities":[]}
        """#.utf8)
        let agent = try JSONDecoder().decode(AgentPresence.self, from: agentData)
        let roomA = try room("A")
        let roomB = try room("B")
        let connection = MockRoomConnection()
        connection.registration = CowchatRegistration(agentID: agent.id)
        connection.listedRooms = [roomA, roomB]
        connection.agentsByRoom = ["A": [agent], "B": [agent]]
        let store = makeStore(connection: connection)

        await store.connect()
        await store.select(room: roomA)
        await store.select(room: roomB)

        XCTAssertEqual(store.rooms.first(where: { $0.id == "A" })?.memberCount, 0)
        XCTAssertEqual(store.rooms.first(where: { $0.id == "B" })?.memberCount, 1)
    }

    @MainActor
    func testLiveMessageAdvancesRoomActivityWithoutSelectingIt() throws {
        let roomData = Data(#"""
        {
          "room_id":"room",
          "name":"room",
          "ephemeral":false,
          "created_at":"2026-07-11T12:00:00Z",
          "visibility":"public",
          "last_activity":"2026-07-12T12:00:00Z"
        }
        """#.utf8)
        let room = try JSONDecoder().decode(Room.self, from: roomData)
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.searchText = "hello"

        connection.onEvent?("message_received", [
            "message_id": "message-1",
            "room_id": "room",
            "agent_id": "agent",
            "agent_name": "Agent",
            "content": "hello",
            "timestamp": "2026-08-04T21:30:00Z",
            "seq": 1,
        ])

        XCTAssertEqual(store.rooms.first?.lastActivity, "2026-08-04T21:30:00Z")
        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertEqual(store.messageSearchRoomIDs, [room.id])
        XCTAssertEqual(store.roomMessagePreviews[room.id], "hello")
        XCTAssertFalse(store.isSearchingMessages)
    }

    @MainActor
    func testLiveThinkingMessageIsExcludedFromChatAndSearch() throws {
        let room = try decodeRoom(id: "room", name: "Room")
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.selectedRoomID = room.id
        store.searchText = "secret"

        connection.onEvent?("message_received", [
            "message_id": "thinking-1",
            "room_id": room.id,
            "agent_id": "collaborator",
            "agent_name": "Collaborator",
            "content": "secret working note",
            "metadata": ["type": "thinking"],
            "timestamp": "2026-08-04T21:31:00Z",
            "seq": 1,
        ])

        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertFalse(store.messageSearchRoomIDs.contains(room.id))
        XCTAssertNil(store.roomMessagePreviews[room.id])
    }

    /// The server broadcasts thinking pulses as a dedicated `thinking` event
    /// (not `message_received`) precisely so wait-style listeners don't wake
    /// on them — see cowchat-server/src/handler.rs. `handleEvent` must still
    /// route that event type into the working-signal tracking.
    @MainActor
    func testLiveThinkingUsesReceiptTimeDespiteAgentClockSkew() throws {
        let roomData = Data(#"""
        {
          "room_id":"room",
          "name":"room",
          "ephemeral":false,
          "created_at":"2026-07-11T12:00:00Z",
          "visibility":"public",
          "last_activity":"2026-07-12T12:00:00Z"
        }
        """#.utf8)
        let room = try JSONDecoder().decode(Room.self, from: roomData)
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.rooms = [room]
        let receivedAfter = Date()

        connection.onEvent?("thinking", [
            "message_id": "thinking-1",
            "room_id": room.id,
            "agent_id": "collaborator",
            "agent_name": "Collaborator",
            "content": "still working on it",
            "metadata": ["type": "thinking"],
            "timestamp": ISO8601DateFormatter().string(
                from: receivedAfter.addingTimeInterval(-3_600)
            ),
            "seq": 1,
        ])

        let recordedActivity = try XCTUnwrap(
            store.recentAgentActivityAt[room.id]?["collaborator"]
        )
        XCTAssertGreaterThanOrEqual(recordedActivity, receivedAfter)
        XCTAssertLessThanOrEqual(recordedActivity, Date())
        XCTAssertTrue(store.isWorking(room, at: Date()))
        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertEqual(store.rooms.first?.lastActivity, "2026-07-12T12:00:00Z")
    }

    @MainActor
    func testMessageSearchPaginatesBeyondNewestHundredRows() async throws {
        let room = try decodeRoom(id: "archive", name: "Archive")
        let messages = try (1...101).map { sequence in
            try JSONDecoder().decode(
                ChatMessage.self,
                from: JSONSerialization.data(withJSONObject: [
                    "message_id": "message-\(sequence)",
                    "room_id": room.id,
                    "agent_id": "agent",
                    "agent_name": "Agent",
                    "content": sequence == 1 ? "old needle" : "recent chatter",
                    "timestamp": String(format: "timestamp-%03d", sequence),
                    "seq": sequence,
                ])
            )
        }
        let connection = MockRoomConnection()
        connection.historiesByRoom[room.id] = messages
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected

        store.searchText = "needle"
        for _ in 0..<100 {
            if store.messageSearchRoomIDs.contains(room.id) { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertEqual(store.messageSearchRoomIDs, [room.id])
        XCTAssertFalse(store.isSearchingMessages)
    }

    @MainActor
    func testRoomRefreshDoesNotRestartUnchangedInFlightHistorySearch() async throws {
        let room = try decodeRoom(id: "archive", name: "Archive")
        let messages = try (1...101).map { sequence in
            try decodeMessage(
                id: "message-\(sequence)",
                roomID: room.id,
                content: sequence == 1 ? "old needle" : "recent chatter",
                timestamp: String(format: "timestamp-%03d", sequence),
                sequence: sequence
            )
        }
        let connection = MockRoomConnection()
        connection.listedRooms = [room]
        connection.historiesByRoom[room.id] = messages
        connection.blockedHistoryLimit = 100
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected

        store.searchText = "needle"
        while !connection.historyRequests.contains(where: { $0.limit == 100 }) {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        try await store.refreshRooms()
        try await Task.sleep(nanoseconds: 350_000_000)

        XCTAssertEqual(connection.historyRequests.filter { $0.limit == 100 }.count, 1)
        connection.resumeBlockedHistory()
        for _ in 0..<100 where !store.messageSearchRoomIDs.contains(room.id) {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertEqual(store.messageSearchRoomIDs, [room.id])
        XCTAssertFalse(store.isSearchingMessages)
    }

    @MainActor
    func testStaleHistorySearchCompletionCannotOverwriteNewGeneration() async throws {
        let roomA = try decodeRoom(id: "A", name: "A")
        let roomB = try decodeRoom(id: "B", name: "B")
        let connection = MockRoomConnection()
        connection.historiesByRoom[roomA.id] = [
            try decodeMessage(
                id: "old-match",
                roomID: roomA.id,
                content: "needle",
                timestamp: "timestamp-001",
                sequence: 1
            ),
        ]
        connection.historiesByRoom[roomB.id] = []
        connection.blockedHistoryLimit = 100
        let store = makeStore(connection: connection)
        store.rooms = [roomA]
        store.connectionStatus = .connected

        store.searchText = "needle"
        while !connection.historyRequests.contains(where: { $0.roomID == roomA.id }) {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        store.searchText = ""
        store.rooms = [roomB]
        store.searchText = "needle"
        for _ in 0..<100 where store.isSearchingMessages {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertEqual(store.messageSearchRoomIDs, [])

        connection.resumeBlockedHistory()
        for _ in 0..<20 { await Task.yield() }

        XCTAssertEqual(store.messageSearchRoomIDs, [])
        XCTAssertFalse(store.isSearchingMessages)
    }

    @MainActor
    func testRapidAToBToCSwitchLeavesTheActuallyJoinedRoom() async throws {
        func room(_ id: String) throws -> Room {
            let json: [String: Any] = [
                "room_id": id,
                "name": id,
                "ephemeral": false,
                "created_at": "2026-07-11T12:00:00Z",
                "visibility": "public",
            ]
            return try JSONDecoder().decode(
                Room.self,
                from: JSONSerialization.data(withJSONObject: json)
            )
        }

        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.connectionStatus = .connected
        let roomA = try room("A")
        let roomB = try room("B")
        let roomC = try room("C")
        await store.select(room: roomA)

        connection.blockedRoomID = "B"
        let selectingB = Task { await store.select(room: roomB) }
        while !connection.operations.contains("join:B") { await Task.yield() }
        let selectingC = Task { await store.select(room: roomC) }
        connection.resumeBlockedJoin()
        await selectingB.value
        await selectingC.value

        XCTAssertEqual(
            connection.operations,
            ["join:A", "leave:A", "join:B", "leave:B", "join:C"]
        )
        XCTAssertEqual(store.selectedRoomID, "C")
    }

    @MainActor
    func testArchivePreferencesPersistWithoutMutatingServer() async throws {
        let suiteName = "RoomTransitionPreferencesTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let connection = MockRoomConnection()
        let room = try decodeRoom(id: "design", name: "Design")
        let store = ChatStore(connection: connection, defaults: defaults)
        store.rooms = [room]

        await store.archive(room)

        XCTAssertTrue(store.isArchived(room))
        XCTAssertTrue(connection.operations.isEmpty)

        let reloaded = ChatStore(connection: connection, defaults: defaults)
        XCTAssertTrue(reloaded.isArchived(room))
    }

    @MainActor
    func testCreatorCanDestroyRoomAndLobbyCannotBeDestroyed() async throws {
        let suiteName = "RoomTransitionDestroyTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("creator-agent", forKey: "CowchatMac.agentID")
        let connection = MockRoomConnection()
        let room = try decodeRoom(id: "design", name: "Design", createdBy: "creator-agent")
        let lobby = try decodeRoom(id: "lobby", name: "Lobby", createdBy: "creator-agent")
        let store = ChatStore(connection: connection, defaults: defaults)
        store.rooms = [lobby, room]
        store.connectionStatus = .connected

        let destroyedRoom = await store.destroy(room)
        XCTAssertTrue(destroyedRoom)
        XCTAssertEqual(connection.destroyedRoomIDs, ["design"])
        XCTAssertEqual(store.rooms.map(\.id), ["lobby"])

        let destroyedLobby = await store.destroy(lobby)
        XCTAssertFalse(destroyedLobby)
        XCTAssertEqual(connection.destroyedRoomIDs, ["design"])
    }

    @MainActor
    func testDestroyEventBeforeReplyStillSelectsLobbyFallback() async throws {
        let suiteName = "RoomTransitionDestroyRaceTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("creator-agent", forKey: "CowchatMac.agentID")
        let connection = MockRoomConnection()
        connection.emitsDestroyEventBeforeReply = true
        let room = try decodeRoom(id: "design", name: "Design", createdBy: "creator-agent")
        let lobby = try decodeRoom(id: "lobby", name: "Lobby")
        let store = ChatStore(connection: connection, defaults: defaults)
        store.rooms = [lobby, room]
        store.selectedRoomID = room.id
        store.connectionStatus = .connected

        let destroyed = await store.destroy(room)
        XCTAssertTrue(destroyed)
        XCTAssertEqual(store.selectedRoomID, lobby.id)
    }

    @MainActor
    func testDestroyEventBeforeReplyErrorIsStillTreatedAsSuccess() async throws {
        let suiteName = "RoomTransitionDestroyReplyErrorTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("creator-agent", forKey: "CowchatMac.agentID")
        let connection = MockRoomConnection()
        connection.emitsDestroyEventBeforeReply = true
        connection.destroyShouldFailAfterEvent = true
        let room = try decodeRoom(id: "design", name: "Design", createdBy: "creator-agent")
        let lobby = try decodeRoom(id: "lobby", name: "Lobby")
        let store = ChatStore(connection: connection, defaults: defaults)
        store.rooms = [lobby, room]
        store.selectedRoomID = room.id
        store.connectionStatus = .connected

        let destroyed = await store.destroy(room)

        XCTAssertTrue(destroyed)
        XCTAssertEqual(store.selectedRoomID, lobby.id)
        XCTAssertNil(store.errorMessage)
        XCTAssertEqual(connection.operations, ["destroy:design", "join:lobby"])
    }

    @MainActor
    func testDestroyedRoomClearsRenameAndSubroomCreationState() throws {
        let parent = try decodeRoom(id: "parent", name: "Parent")
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        store.rooms = [parent]
        store.roomBeingRenamed = parent
        store.presentCreateRoom(parentID: parent.id)

        connection.onEvent?("room_destroyed", ["room_id": parent.id])
        connection.onEvent?("room_updated", [
            "room_id": parent.id,
            "name": "Stale Parent",
            "ephemeral": false,
            "created_at": "2026-08-04T12:00:00Z",
            "visibility": "public",
        ])

        XCTAssertNil(store.roomBeingRenamed)
        XCTAssertNil(store.createRoomParentID)
        XCTAssertFalse(store.isCreateRoomPresented)
        XCTAssertFalse(store.rooms.contains(where: { $0.id == parent.id }))
    }

    @MainActor
    func testCreatorCanRenameRoomAndLiveUpdateReplacesIt() async throws {
        let suiteName = "RoomTransitionRenameTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("creator-agent", forKey: "CowchatMac.agentID")
        let connection = MockRoomConnection()
        let room = try decodeRoom(id: "design", name: "Design", createdBy: "creator-agent")
        connection.listedRooms = [room]
        let store = ChatStore(connection: connection, defaults: defaults)
        store.rooms = [room]
        store.connectionStatus = .connected

        let renamed = await store.rename(room, to: "  Product Design  ")

        XCTAssertTrue(renamed)
        XCTAssertEqual(connection.renamedRooms.first?.0, "design")
        XCTAssertEqual(connection.renamedRooms.first?.1, "Product Design")
        XCTAssertEqual(store.rooms.first?.name, "Product Design")

        connection.onEvent?("room_updated", [
            "room_id": "design",
            "name": "Design Systems",
            "ephemeral": false,
            "created_at": "2026-08-04T12:00:00Z",
            "created_by": "creator-agent",
            "visibility": "public",
        ])
        XCTAssertEqual(store.rooms.first?.name, "Design Systems")
    }

    @MainActor
    func testMessageSearchMatchesHistoryContentAfterDebounce() async throws {
        let room = try decodeRoom(id: "release", name: "Release")
        let messageData = Data(#"""
        {
          "message_id":"message",
          "room_id":"release",
          "agent_id":"agent",
          "agent_name":"Agent",
          "content":"deployment succeeded",
          "timestamp":"2026-08-04T22:00:00Z",
          "seq":1
        }
        """#.utf8)
        let message = try JSONDecoder().decode(ChatMessage.self, from: messageData)
        let connection = MockRoomConnection()
        connection.historiesByRoom[room.id] = [message]
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected

        store.searchText = "  deployment  "
        for _ in 0..<100 {
            if store.messageSearchRoomIDs.contains(room.id) { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertEqual(store.messageSearchRoomIDs, [room.id])
    }

    @MainActor
    func testDraftsRemainBoundToTheirOriginalRooms() async throws {
        let roomA = try decodeRoom(id: "A", name: "A")
        let roomB = try decodeRoom(id: "B", name: "B")
        let store = makeStore(connection: MockRoomConnection())
        store.rooms = [roomA, roomB]

        await store.select(room: roomA)
        store.draft = "secret for A"
        await store.select(room: roomB)
        XCTAssertEqual(store.draft, "")

        store.draft = "note for B"
        await store.select(room: roomA)
        XCTAssertEqual(store.draft, "secret for A")

        await store.select(room: roomB)
        XCTAssertEqual(store.draft, "note for B")
    }

    @MainActor
    func testFailedSendDoesNotOverwriteANewerDraftSavedWhileItWasInFlight() async throws {
        let roomA = try decodeRoom(id: "A", name: "A")
        let roomB = try decodeRoom(id: "B", name: "B")
        let connection = MockRoomConnection()
        connection.blocksSend = true
        connection.sendShouldFail = true
        let store = makeStore(connection: connection)
        store.rooms = [roomA, roomB]
        store.connectionStatus = .connected
        await store.select(room: roomA)

        store.draft = "first attempt"
        store.sendDraft()
        while !connection.operations.contains("send:A:first attempt") { await Task.yield() }

        store.draft = "newer replacement"
        await store.select(room: roomB)
        connection.resumeBlockedSend()
        for _ in 0..<100 where store.errorMessage == nil { await Task.yield() }

        await store.select(room: roomA)
        XCTAssertEqual(store.draft, "newer replacement")
    }

    @MainActor
    func testLaterConcurrentFailedSendRemainsRecoverable() async throws {
        let room = try decodeRoom(id: "A", name: "A")
        let connection = MockRoomConnection()
        connection.blocksSend = true
        connection.sendShouldFail = true
        let store = makeStore(connection: connection)
        store.rooms = [room]
        store.connectionStatus = .connected
        await store.select(room: room)

        store.draft = "first attempt"
        store.sendDraft()
        store.draft = "second attempt"
        store.sendDraft()
        while !connection.operations.contains("send:A:first attempt")
            || !connection.operations.contains("send:A:second attempt") {
            await Task.yield()
        }

        connection.resumeBlockedSend(content: "first attempt")
        for _ in 0..<100 where store.draft != "first attempt" { await Task.yield() }
        XCTAssertEqual(store.draft, "first attempt")

        connection.resumeBlockedSend(content: "second attempt")
        for _ in 0..<100 where store.draft != "second attempt" { await Task.yield() }
        XCTAssertEqual(store.draft, "second attempt")
    }

    @MainActor
    func testSetupContinueWaitsForRealCollaboratorBeforeShowingReadyNotice() async throws {
        let suiteName = "RoomTransitionSetupTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("creator-agent", forKey: "CowchatMac.agentID")
        let lobby = try decodeRoom(id: "lobby", name: "Lobby")
        let room = try decodeRoom(id: "design", name: "Design", createdBy: "creator-agent")
        let selfAgent = try decodeAgent(id: "creator-agent", name: "Cowchat Mac")
        let collaborator = try decodeAgent(id: "claude", name: "Claude")
        let connection = MockRoomConnection()
        connection.listedRooms = [lobby]
        connection.roomToCreate = room
        connection.agentsByRoom = [lobby.id: [selfAgent], room.id: [selfAgent]]
        let store = ChatStore(connection: connection, defaults: defaults)
        await store.connect()

        let created = await store.createRoom(
            name: room.name,
            description: "",
            ephemeral: false,
            isPublic: false
        )
        XCTAssertTrue(created)
        XCTAssertTrue(store.setupRoomIDs.contains(room.id))

        connection.onEvent?("message_received", [
            "message_id": "creator-echo",
            "room_id": room.id,
            "agent_id": store.agentID,
            "agent_name": "Cowchat Mac",
            "content": "hello",
            "timestamp": "2026-08-04T21:32:00Z",
            "seq": 1,
        ])
        XCTAssertTrue(store.setupRoomIDs.contains(room.id))
        XCTAssertNil(store.roomReadyNotice)

        // Navigate away from the freshly created room (e.g. back to Lobby)
        // before a real collaborator has joined.
        await store.select(room: lobby)
        XCTAssertEqual(store.selectedRoomID, lobby.id)
        XCTAssertTrue(store.setupRoomIDs.contains(room.id))
        XCTAssertNil(store.roomReadyNotice)

        connection.agentsByRoom[room.id] = [collaborator]
        await store.pollSetupRoomReadiness()
        XCTAssertFalse(store.setupRoomIDs.contains(room.id))
        XCTAssertEqual(store.roomReadyNotice?.id, room.id)
    }

    @MainActor
    func testCollaboratorJoiningWhileSetupIsOpenTransitionsDirectlyToChat() async throws {
        let suiteName = "RoomTransitionOpenSetupTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("creator-agent", forKey: "CowchatMac.agentID")
        let lobby = try decodeRoom(id: "lobby", name: "Lobby")
        let room = try decodeRoom(id: "design", name: "Design", createdBy: "creator-agent")
        let selfAgent = try decodeAgent(id: "creator-agent", name: "Cowchat Mac")
        let collaborator = try decodeAgent(id: "claude", name: "Claude")
        let connection = MockRoomConnection()
        connection.listedRooms = [lobby]
        connection.roomToCreate = room
        connection.agentsByRoom = [lobby.id: [selfAgent], room.id: [selfAgent]]
        let store = ChatStore(connection: connection, defaults: defaults)
        await store.connect()
        _ = await store.createRoom(
            name: room.name,
            description: "",
            ephemeral: false,
            isPublic: false
        )

        connection.agentsByRoom[room.id] = [selfAgent, collaborator]
        await store.pollSetupRoomReadiness()

        XCTAssertEqual(store.selectedRoomID, room.id)
        XCTAssertFalse(store.setupRoomIDs.contains(room.id))
        XCTAssertNil(store.roomReadyNotice)
    }

    @MainActor
    func testFirstCollaboratorInSelectedRoomShowsSecondAgentHintOnce() async throws {
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        let room = try decodeRoom(id: "r1", name: "my-room")
        connection.listedRooms = [try decodeRoom(id: "lobby", name: "lobby"), room]
        await store.connect()
        connection.roomToCreate = room
        _ = await store.createRoom(name: "my-room", description: "", ephemeral: false, isPublic: true)

        // Collaborator appears while the room is selected → in-place flip.
        connection.agentsByRoom["r1"] = [AgentPresence(agentID: "other", name: "codex")]
        await store.pollSetupRoomReadiness()

        XCTAssertEqual(store.secondAgentHintRoom?.id, "r1")
        XCTAssertNil(store.roomReadyNotice)

        // Once per room: re-entering the setup state can't re-trigger it.
        store.secondAgentHintRoom = nil
        connection.agentsByRoom["r1"] = []
        await store.pollSetupRoomReadiness()
        connection.agentsByRoom["r1"] = [AgentPresence(agentID: "other", name: "codex")]
        await store.pollSetupRoomReadiness()
        XCTAssertNil(store.secondAgentHintRoom)
    }

    @MainActor
    func testSelectedAgentlessRoomIsPolledForMembersEvenWhenNotInSetupSet() async throws {
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        let room = try decodeRoom(id: "r-empty", name: "empty-room")
        connection.listedRooms = [room]
        await store.connect()
        await store.select(room: room)
        XCTAssertTrue(store.roomMembers.isEmpty)
        XCTAssertTrue(store.setupRoomIDs.isEmpty)
        XCTAssertTrue(store.selectedRoomAwaitingFirstAgent)

        // An agent connects between event pushes; only polling can see it.
        connection.agentsByRoom["r-empty"] = [
            AgentPresence(agentID: "other-agent", name: "codex")
        ]
        await store.pollSetupRoomReadiness()
        XCTAssertTrue(store.roomMembers.contains { $0.agentID == "other-agent" })
    }

    @MainActor
    func testAgentLeftRestartsSetupReadinessPollingForConnectStateRoom() async throws {
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        let room = try decodeRoom(id: "r-emptied", name: "emptied-room")
        connection.listedRooms = [room]
        connection.agentsByRoom[room.id] = [AgentPresence(agentID: "collaborator", name: "Claude")]
        await store.connect()
        await store.select(room: room)
        // The room already has a collaborator at selection time, so the setup
        // readiness poll loop never starts (its guard requires
        // selectedRoomAwaitingFirstAgent, which is false here).
        XCTAssertFalse(store.selectedRoomAwaitingFirstAgent)

        // The only other member leaves live; refreshMembers empties roomMembers
        // and the room re-enters the connect state.
        connection.agentsByRoom[room.id] = []
        connection.onEvent?("agent_left", ["room_id": room.id])
        for _ in 0..<100 where !store.roomMembers.isEmpty {
            await Task.yield()
        }
        XCTAssertTrue(store.roomMembers.isEmpty)
        XCTAssertTrue(store.selectedRoomAwaitingFirstAgent)

        // A later member appears with no further live event — only a poll
        // loop restarted after agent_left can discover it (bug: the loop
        // already exited when the room first got a collaborator, and nothing
        // restarted it on agent_left, so "waiting…" would be stuck forever).
        connection.agentsByRoom[room.id] = [AgentPresence(agentID: "later-agent", name: "codex")]
        var observedLaterJoiner = false
        for _ in 0..<400 {
            if store.roomMembers.contains(where: { $0.agentID == "later-agent" }) {
                observedLaterJoiner = true
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertTrue(observedLaterJoiner)
    }

    @MainActor
    func testLaunchSelectionPrefersLobbyOverPendingSetupRooms() async throws {
        let connection = MockRoomConnection()
        let suiteName = "RoomTransitionTests.launch.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        // A previous build left a persisted setup-screen room behind.
        defaults.set(["stale-room"], forKey: "CowchatMac.pendingSetupScreenRoomIDs")
        defaults.set(["stale-room"], forKey: RoomLocalPreferences.pendingSetupRoomIDsKey)
        let store = ChatStore(connection: connection, defaults: defaults)

        connection.listedRooms = [
            try decodeRoom(id: "stale-room", name: "stale-room"),
            try decodeRoom(id: "lobby", name: "lobby"),
        ]
        await store.connect()

        XCTAssertEqual(store.selectedRoomID, "lobby")
    }

    @MainActor
    func testInitDeletesThePersistedSetupScreenKey() {
        let connection = MockRoomConnection()
        let suiteName = "RoomTransitionTests.cleanup.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(["r1"], forKey: "CowchatMac.pendingSetupScreenRoomIDs")
        _ = ChatStore(connection: connection, defaults: defaults)
        XCTAssertNil(defaults.object(forKey: "CowchatMac.pendingSetupScreenRoomIDs"))
    }

    @MainActor
    func testFreshInstallAutoCreatesPublicGeneralAndSelectsIt() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [try decodeRoom(id: "lobby", name: "lobby")]
        connection.roomToCreate = try decodeRoom(id: "general-id", name: "General")
        let store = makeFreshInstallStore(connection: connection)

        await store.connect()

        XCTAssertTrue(connection.operations.contains("create:General"))
        XCTAssertEqual(store.selectedRoomID, "general-id")
        XCTAssertTrue(store.setupRoomIDs.contains("general-id"))
    }

    @MainActor
    func testAutoCreateSkipsWhenUserRoomsExist() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [
            try decodeRoom(id: "lobby", name: "lobby"),
            try decodeRoom(id: "mine", name: "my-room"),
        ]
        let store = makeFreshInstallStore(connection: connection)

        await store.connect()

        XCTAssertFalse(connection.operations.contains { $0.hasPrefix("create:") })
        XCTAssertEqual(store.selectedRoomID, "lobby")
    }

    @MainActor
    func testAutoCreateSelectsExistingVisibleGeneralInstead() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [
            try decodeRoom(id: "lobby", name: "lobby"),
            try decodeRoom(id: "existing-general", name: "general"),
        ]
        let store = makeFreshInstallStore(connection: connection)

        await store.connect()

        XCTAssertFalse(connection.operations.contains { $0.hasPrefix("create:") })
        XCTAssertEqual(store.selectedRoomID, "existing-general")
    }

    @MainActor
    func testAutoCreateRunsOnlyOnce() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [try decodeRoom(id: "lobby", name: "lobby")]
        connection.roomToCreate = try decodeRoom(id: "general-id", name: "General")
        let store = makeFreshInstallStore(connection: connection)
        await store.connect()

        // User destroyed General; a later connect must not resurrect it.
        connection.listedRooms = [try decodeRoom(id: "lobby", name: "lobby")]
        await store.connect()

        XCTAssertEqual(connection.operations.filter { $0 == "create:General" }.count, 1)
    }

    @MainActor
    func testNameTakenSetsGuardFlagInsteadOfRetryLooping() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [try decodeRoom(id: "lobby", name: "lobby")]
        connection.createRoomError = CowchatConnectionError.server(
            message: "Room name 'General' already taken", code: "room_name_taken"
        )
        let suiteName = "RoomTransitionTests.taken.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        let store = ChatStore(connection: connection, defaults: defaults)

        await store.connect()

        XCTAssertTrue(defaults.bool(forKey: ChatStore.didCreateDefaultRoomKey))
        XCTAssertNil(store.errorMessage)  // silent fallback, no alert
        XCTAssertEqual(store.selectedRoomID, "lobby")
    }

    func testBackfillSuppressesAutoCreateForExistingInstalls() {
        let suiteName = "RoomTransitionTests.backfill.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        // Existing install: already past onboarding v1.
        defaults.set(CowchatOnboarding.currentVersion, forKey: CowchatOnboarding.completedVersionKey)

        ChatStore.suppressDefaultRoomForExistingInstalls(defaults: defaults)

        XCTAssertTrue(defaults.bool(forKey: ChatStore.didCreateDefaultRoomKey))
        // And it never re-runs: a later reset of the guard flag sticks.
        defaults.removeObject(forKey: ChatStore.didCreateDefaultRoomKey)
        ChatStore.suppressDefaultRoomForExistingInstalls(defaults: defaults)
        XCTAssertFalse(defaults.bool(forKey: ChatStore.didCreateDefaultRoomKey))
    }

    func testBackfillLeavesFreshInstallsEligible() {
        let suiteName = "RoomTransitionTests.backfill2.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)

        ChatStore.suppressDefaultRoomForExistingInstalls(defaults: defaults)

        XCTAssertFalse(defaults.bool(forKey: ChatStore.didCreateDefaultRoomKey))
    }

    @MainActor
    func testCreateAndRenameRejectLobbyNames() async throws {
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        connection.listedRooms = [try decodeRoom(id: "lobby", name: "lobby")]
        await store.connect()

        let created = await store.createRoom(
            name: "Lobby", description: "", ephemeral: false, isPublic: true
        )
        XCTAssertFalse(created)
        XCTAssertNotNil(store.errorMessage)
        XCTAssertFalse(connection.operations.contains("create:Lobby"))
    }

    private func decodeRoom(
        id: String,
        name: String,
        createdBy: String? = nil,
        ephemeral: Bool = false,
        memberCount: Int? = nil
    ) throws -> Room {
        var json: [String: Any] = [
            "room_id": id,
            "name": name,
            "ephemeral": ephemeral,
            "created_at": "2026-08-04T12:00:00Z",
            "visibility": "public",
        ]
        if let createdBy { json["created_by"] = createdBy }
        if let memberCount { json["member_count"] = memberCount }
        return try JSONDecoder().decode(
            Room.self,
            from: JSONSerialization.data(withJSONObject: json)
        )
    }

    private func decodeAgent(id: String, name: String) throws -> AgentPresence {
        try JSONDecoder().decode(
            AgentPresence.self,
            from: JSONSerialization.data(withJSONObject: [
                "agent_id": id,
                "name": name,
                "capabilities": [],
            ])
        )
    }

    private func decodeMessage(
        id: String,
        roomID: String,
        content: String,
        timestamp: String,
        sequence: Int,
        agentID: String = "agent",
        agentName: String = "Agent",
        metadataType: String? = nil
    ) throws -> ChatMessage {
        var json: [String: Any] = [
            "message_id": id,
            "room_id": roomID,
            "agent_id": agentID,
            "agent_name": agentName,
            "content": content,
            "timestamp": timestamp,
            "seq": sequence,
        ]
        if let metadataType {
            json["metadata"] = ["type": metadataType]
        }
        return try JSONDecoder().decode(
            ChatMessage.self,
            from: JSONSerialization.data(withJSONObject: json)
        )
    }
}
