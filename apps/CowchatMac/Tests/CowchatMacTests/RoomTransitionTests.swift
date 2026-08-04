import XCTest
@testable import CowchatMac

@MainActor
private final class MockRoomConnection: CowchatConnectionProtocol {
    var onEvent: ((String, [String: Any]) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?
    var operations: [String] = []
    var blockedRoomID: String?
    var agentsByRoom: [String: [AgentPresence]] = [:]
    var listedRooms: [Room] = []
    var blocksListRooms = false
    private var joinContinuation: CheckedContinuation<Void, Never>?
    private var listRoomsContinuation: CheckedContinuation<Void, Never>?

    func connect() async throws {}
    func register(name: String, agentID: String) async throws -> String { agentID }
    func listRooms() async throws -> [Room] {
        operations.append("listRooms")
        if blocksListRooms {
            await withCheckedContinuation { listRoomsContinuation = $0 }
        }
        return listedRooms
    }
    func listAgents(roomID: String) async throws -> [AgentPresence] { agentsByRoom[roomID] ?? [] }
    func createRoom(name: String, description: String?, ephemeral: Bool, isPublic: Bool) async throws -> Room {
        fatalError("not used")
    }
    func join(roomID: String) async throws {
        operations.append("join:\(roomID)")
        if roomID == blockedRoomID {
            await withCheckedContinuation { joinContinuation = $0 }
        }
    }
    func leave(roomID: String) async throws { operations.append("leave:\(roomID)") }
    func history(roomID: String, limit: Int) async throws -> [ChatMessage] { [] }
    func send(roomID: String, content: String) async throws -> ChatMessage { fatalError("not used") }
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
}

final class RoomTransitionTests: XCTestCase {
    @MainActor
    func testRoomRefreshPreservesRoomCreatedWhileRequestIsInFlight() async throws {
        let existingData = Data(#"""
        {"room_id":"existing","name":"existing","created_at":"2026-07-11T12:00:00Z","visibility":"public"}
        """#.utf8)
        let existing = try JSONDecoder().decode(Room.self, from: existingData)
        let connection = MockRoomConnection()
        connection.listedRooms = [existing]
        connection.blocksListRooms = true
        let store = ChatStore(connection: connection)
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
        let store = ChatStore(connection: connection)
        store.rooms = [stale]

        try await store.refreshRooms()

        XCTAssertEqual(store.rooms.first?.lastActivity, "2026-08-04T21:45:00Z")
        XCTAssertEqual(store.rooms.first?.memberCount, 3)
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
        let store = ChatStore(connection: connection)
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
        let store = ChatStore(connection: connection)
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
        let store = ChatStore(connection: connection)
        store.rooms = [room]

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
        let store = ChatStore(connection: connection)
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
}
