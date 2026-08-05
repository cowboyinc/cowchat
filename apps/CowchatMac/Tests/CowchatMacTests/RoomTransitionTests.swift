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
    var registration = CowchatRegistration(agentID: "")
    var destroyedRoomIDs: [String] = []
    var emitsDestroyEventBeforeReply = false
    var destroyShouldFailAfterEvent = false
    var renamedRooms: [(String, String)] = []
    var blocksListRooms = false
    var blockedHistoryLimit: Int?
    var blocksSend = false
    var sendShouldFail = false
    private var joinContinuation: CheckedContinuation<Void, Never>?
    private var listRoomsContinuation: CheckedContinuation<Void, Never>?
    private var historyContinuation: CheckedContinuation<Void, Never>?
    private var sendContinuations: [String: CheckedContinuation<Void, Never>] = [:]

    func connect() async throws {}
    func register(name: String, agentID: String) async throws -> CowchatRegistration {
        CowchatRegistration(
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
    func listAgents(roomID: String) async throws -> [AgentPresence] { agentsByRoom[roomID] ?? [] }
    func createRoom(
        name: String,
        description: String?,
        parentID: String?,
        ephemeral: Bool,
        isPublic: Bool
    ) async throws -> Room {
        guard let roomToCreate else { fatalError("set roomToCreate before creating") }
        return roomToCreate
    }
    func destroy(roomID: String) async throws {
        operations.append("destroy:\(roomID)")
        destroyedRoomIDs.append(roomID)
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

    func resumeBlockedListRooms() {
        blocksListRooms = false
        listRoomsContinuation?.resume()
        listRoomsContinuation = nil
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

final class RoomTransitionTests: XCTestCase {
    @MainActor
    private func makeStore(connection: MockRoomConnection) -> ChatStore {
        let suiteName = "RoomTransitionTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        return ChatStore(connection: connection, defaults: defaults)
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
        XCTAssertEqual(RoomSidebarPresentation.activeRooms(from: store.rooms).map(\.id), ["room"])
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
        connection.agentsByRoom = ["A": [agent], "B": [agent]]
        let store = makeStore(connection: connection)
        store.rooms = [roomA, roomB]
        store.connectionStatus = .connected

        await store.select(room: roomA)
        await store.select(room: roomB)

        XCTAssertEqual(store.rooms.first(where: { $0.id == "A" })?.memberCount, 0)
        XCTAssertEqual(store.rooms.first(where: { $0.id == "B" })?.memberCount, 1)
        XCTAssertEqual(RoomSidebarPresentation.activeRooms(from: store.rooms).map(\.id), ["B"])
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
    func testArchiveAndPinPreferencesPersistWithoutMutatingServer() async throws {
        let suiteName = "RoomTransitionPreferencesTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let connection = MockRoomConnection()
        let room = try decodeRoom(id: "design", name: "Design")
        let store = ChatStore(connection: connection, defaults: defaults)
        store.rooms = [room]

        store.togglePinned(room)
        XCTAssertTrue(store.isPinned(room))
        await store.archive(room)

        XCTAssertTrue(store.isArchived(room))
        XCTAssertFalse(store.isPinned(room))
        XCTAssertTrue(connection.operations.isEmpty)

        let reloaded = ChatStore(connection: connection, defaults: defaults)
        XCTAssertTrue(reloaded.isArchived(room))
        XCTAssertFalse(reloaded.isPinned(room))
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
        XCTAssertTrue(store.roomSetupScreenIDs.contains(room.id))

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
        XCTAssertTrue(store.roomSetupScreenIDs.contains(room.id))
        XCTAssertNil(store.roomReadyNotice)

        await store.completeRoomSetup(room)
        XCTAssertEqual(store.selectedRoomID, lobby.id)
        XCTAssertTrue(store.setupRoomIDs.contains(room.id))
        XCTAssertFalse(store.roomSetupScreenIDs.contains(room.id))
        XCTAssertNil(store.roomReadyNotice)

        connection.agentsByRoom[room.id] = [collaborator]
        await store.pollSetupRoomReadiness()
        XCTAssertFalse(store.setupRoomIDs.contains(room.id))
        XCTAssertEqual(store.roomReadyNotice?.id, room.id)
    }

    @MainActor
    func testContinuingTemporaryRoomSetupKeepsTheCreatorJoined() async throws {
        let suiteName = "RoomTransitionTemporarySetupTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("creator-agent", forKey: "CowchatMac.agentID")
        let lobby = try decodeRoom(id: "lobby", name: "Lobby")
        let room = try decodeRoom(
            id: "temporary",
            name: "Temporary",
            createdBy: "creator-agent",
            ephemeral: true
        )
        let selfAgent = try decodeAgent(id: "creator-agent", name: "Cowchat Mac")
        let connection = MockRoomConnection()
        connection.listedRooms = [lobby]
        connection.roomToCreate = room
        connection.agentsByRoom = [lobby.id: [selfAgent], room.id: [selfAgent]]
        let store = ChatStore(connection: connection, defaults: defaults)
        await store.connect()
        _ = await store.createRoom(
            name: room.name,
            description: "",
            ephemeral: true,
            isPublic: false
        )

        await store.completeRoomSetup(room)

        XCTAssertEqual(store.selectedRoomID, room.id)
        XCTAssertFalse(connection.operations.contains("leave:\(room.id)"))
        XCTAssertTrue(store.setupRoomIDs.contains(room.id))
        XCTAssertFalse(store.roomSetupScreenIDs.contains(room.id))
    }

    @MainActor
    func testPendingSetupScreenResumesAcrossRelaunchAndDismissalPersists() async throws {
        let suiteName = "RoomTransitionSetupPersistenceTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("creator-agent", forKey: "CowchatMac.agentID")
        let lobby = try decodeRoom(id: "lobby", name: "Lobby")
        let room = try decodeRoom(id: "design", name: "Design", createdBy: "creator-agent")
        let selfAgent = try decodeAgent(id: "creator-agent", name: "Cowchat Mac")

        let creatingConnection = MockRoomConnection()
        creatingConnection.listedRooms = [lobby]
        creatingConnection.roomToCreate = room
        creatingConnection.agentsByRoom = [lobby.id: [selfAgent], room.id: [selfAgent]]
        let creatingStore = ChatStore(connection: creatingConnection, defaults: defaults)
        await creatingStore.connect()
        _ = await creatingStore.createRoom(
            name: room.name,
            description: "",
            ephemeral: false,
            isPublic: false
        )
        XCTAssertTrue(creatingStore.roomSetupScreenIDs.contains(room.id))

        let relaunchedConnection = MockRoomConnection()
        relaunchedConnection.listedRooms = [lobby, room]
        relaunchedConnection.agentsByRoom = [lobby.id: [selfAgent], room.id: [selfAgent]]
        let relaunchedStore = ChatStore(connection: relaunchedConnection, defaults: defaults)
        await relaunchedStore.connect()

        XCTAssertEqual(relaunchedStore.selectedRoomID, room.id)
        XCTAssertTrue(relaunchedStore.roomSetupScreenIDs.contains(room.id))

        await relaunchedStore.completeRoomSetup(room)
        let dismissedStore = ChatStore(connection: MockRoomConnection(), defaults: defaults)

        XCTAssertFalse(dismissedStore.roomSetupScreenIDs.contains(room.id))
        XCTAssertTrue(dismissedStore.setupRoomIDs.contains(room.id))
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
        XCTAssertFalse(store.roomSetupScreenIDs.contains(room.id))
        XCTAssertNil(store.roomReadyNotice)
    }

    private func decodeRoom(
        id: String,
        name: String,
        createdBy: String? = nil,
        ephemeral: Bool = false
    ) throws -> Room {
        var json: [String: Any] = [
            "room_id": id,
            "name": name,
            "ephemeral": ephemeral,
            "created_at": "2026-08-04T12:00:00Z",
            "visibility": "public",
        ]
        if let createdBy { json["created_by"] = createdBy }
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
        sequence: Int
    ) throws -> ChatMessage {
        try JSONDecoder().decode(
            ChatMessage.self,
            from: JSONSerialization.data(withJSONObject: [
                "message_id": id,
                "room_id": roomID,
                "agent_id": "agent",
                "agent_name": "Agent",
                "content": content,
                "timestamp": timestamp,
                "seq": sequence,
            ])
        )
    }
}
