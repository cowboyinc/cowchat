import XCTest
@testable import CowchatMac

final class LocalServerIntegrationTests: XCTestCase {
    @MainActor
    func testReplacingAConnectionIgnoresTheOldCancellation() async throws {
        try requireLocalServer()

        let connection = CowchatConnection()
        let agentID = "cowchat-mac-reconnect-tests-\(UUID().uuidString.lowercased())"
        try await connection.connect()
        _ = try await connection.register(name: "Cowchat Mac Reconnect Tests", agentID: agentID)

        try await connection.connect()
        _ = try await connection.register(name: "Cowchat Mac Reconnect Tests", agentID: agentID)
        let rooms = try await connection.listRooms()
        XCTAssertFalse(rooms.isEmpty)
        connection.disconnect()
    }

    @MainActor
    func testCreateJoinAndSendAgainstLocalServer() async throws {
        try requireLocalServer()

        let connection = CowchatConnection()
        try await connection.connect()
        _ = try await connection.register(
            name: "Cowchat Mac Tests",
            agentID: "cowchat-mac-tests-\(UUID().uuidString.lowercased())"
        )

        let room = try await connection.createRoom(
            name: "mac-ui-test-\(UUID().uuidString)",
            description: "Temporary integration test",
            ephemeral: true,
            isPublic: false
        )
        try await connection.join(roomID: room.roomID)
        let members = try await connection.listAgents(roomID: room.roomID)
        let message = try await connection.send(roomID: room.roomID, content: "hello from SwiftUI")

        XCTAssertEqual(members.count, 1)
        XCTAssertEqual(members.first?.name, "Cowchat Mac Tests")
        XCTAssertEqual(message.roomID, room.roomID)
        XCTAssertEqual(message.content, "hello from SwiftUI")
        XCTAssertEqual(message.seq, 1)
        connection.disconnect()
    }

    private func requireLocalServer() throws {
        guard ProcessInfo.processInfo.environment["COWCHAT_INTEGRATION_TEST"] == "1" else {
            throw XCTSkip("Set COWCHAT_INTEGRATION_TEST=1 with a local server running.")
        }
    }
}
