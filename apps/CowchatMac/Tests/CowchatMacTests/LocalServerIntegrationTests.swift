import XCTest
@testable import CowchatMac

final class LocalServerIntegrationTests: XCTestCase {
    @MainActor
    func testReplacingAConnectionIgnoresTheOldCancellation() async throws {
        try requireLocalServer()

        let connection = integrationConnection()
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
    func testRegisterReportsRestoredRoomMembership() async throws {
        try requireLocalServer()

        let connection = integrationConnection()
        let agentID = "cowchat-mac-restoration-tests-\(UUID().uuidString.lowercased())"
        try await connection.connect()
        _ = try await connection.register(name: "Cowchat Mac Restoration Tests", agentID: agentID)
        let room = try await connection.createRoom(
            name: "mac-restoration-\(UUID().uuidString)",
            description: "Reconnect integration test",
            parentID: nil,
            ephemeral: false,
            isPublic: false
        )
        try await connection.join(roomID: room.id)
        connection.disconnect()

        var registration = CowchatRegistration(agentID: agentID)
        for attempt in 0..<10 {
            try await Task.sleep(nanoseconds: 100_000_000)
            try await connection.connect()
            registration = try await connection.register(
                name: "Cowchat Mac Restoration Tests",
                agentID: agentID
            )
            if registration.restoredRoomIDs.contains(room.id) { break }
            if attempt < 9 { connection.disconnect() }
        }

        XCTAssertTrue(registration.restoredRoomIDs.contains(room.id))
        if registration.restoredRoomIDs.contains(room.id) {
            try await connection.leave(roomID: room.id)
        }
        try await connection.destroy(roomID: room.id)
        connection.disconnect()
    }

    @MainActor
    func testCreateJoinAndSendAgainstLocalServer() async throws {
        try requireLocalServer()

        let connection = integrationConnection()
        try await connection.connect()
        _ = try await connection.register(
            name: "Cowchat Mac Tests",
            agentID: "cowchat-mac-tests-\(UUID().uuidString.lowercased())"
        )

        let room = try await connection.createRoom(
            name: "mac-ui-test-\(UUID().uuidString)",
            description: "Temporary integration test",
            parentID: nil,
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

    @MainActor
    func testRenameAndDestroyAgainstLocalServer() async throws {
        try requireLocalServer()

        let connection = integrationConnection()
        try await connection.connect()
        _ = try await connection.register(
            name: "Cowchat Mac Lifecycle Tests",
            agentID: "cowchat-mac-lifecycle-tests-\(UUID().uuidString.lowercased())"
        )

        let room = try await connection.createRoom(
            name: "mac-lifecycle-\(UUID().uuidString)",
            description: "Temporary lifecycle integration test",
            parentID: nil,
            ephemeral: true,
            isPublic: false
        )
        let renamed = try await connection.rename(
            roomID: room.id,
            name: "mac-renamed-\(UUID().uuidString)"
        )
        XCTAssertEqual(renamed.id, room.id)
        let roomsAfterRename = try await connection.listRooms()
        XCTAssertTrue(roomsAfterRename.contains { $0.name == renamed.name })

        try await connection.destroy(roomID: room.id)
        let roomsAfterDestroy = try await connection.listRooms()
        XCTAssertFalse(roomsAfterDestroy.contains { $0.id == room.id })
        connection.disconnect()
    }

    private func requireLocalServer() throws {
        guard ProcessInfo.processInfo.environment["COWCHAT_INTEGRATION_TEST"] == "1" else {
            throw XCTSkip("Set COWCHAT_INTEGRATION_TEST=1 with a local server running.")
        }
    }

    @MainActor
    private func integrationConnection() -> CowchatConnection {
        let port = ProcessInfo.processInfo.environment["COWCHAT_INTEGRATION_PORT"]
            .flatMap(UInt16.init) ?? 9229
        return CowchatConnection(port: port)
    }
}
