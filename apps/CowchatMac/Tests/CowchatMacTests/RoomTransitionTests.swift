import XCTest
@testable import CowchatMac

@MainActor
private final class MockRoomConnection: CowchatConnectionProtocol {
    var onEvent: ((String, [String: Any]) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?
    var operations: [String] = []
    var blockedRoomID: String?
    private var joinContinuation: CheckedContinuation<Void, Never>?

    func connect() async throws {}
    func register(name: String, agentID: String) async throws -> String { agentID }
    func listRooms() async throws -> [Room] { [] }
    func listAgents(roomID: String) async throws -> [AgentPresence] { [] }
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
}

final class RoomTransitionTests: XCTestCase {
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
