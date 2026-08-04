import XCTest
@testable import CowchatMac

final class ProtocolModelsTests: XCTestCase {
    func testRoomDecodesServerPayloadAndDefaultsEncryption() throws {
        let data = Data(#"""
        {
          "room_id":"lobby",
          "name":"lobby",
          "ephemeral":false,
          "created_at":"2026-07-11T12:00:00Z",
          "visibility":"public",
          "member_count":3
        }
        """#.utf8)

        let room = try JSONDecoder().decode(Room.self, from: data)
        XCTAssertEqual(room.id, "lobby")
        XCTAssertEqual(room.memberCount, 3)
        XCTAssertFalse(room.encrypted)
    }

    func testMessageDecodesCurrentProtocolShape() throws {
        let data = Data(#"""
        {
          "message_id":"message-1",
          "room_id":"lobby",
          "agent_id":"agent-1",
          "agent_name":"Codex",
          "content":"hello",
          "metadata":{},
          "timestamp":"2026-07-11T12:00:00.123Z",
          "seq":42
        }
        """#.utf8)

        let message = try JSONDecoder().decode(ChatMessage.self, from: data)
        XCTAssertEqual(message.content, "hello")
        XCTAssertEqual(message.seq, 42)
        XCTAssertNil(message.replyToMessage)
    }

    func testAgentPresenceDecodesThinkingStatus() throws {
        let data = Data(#"""
        {
          "agent_id":"agent-1",
          "name":"Claude",
          "capabilities":["review"],
          "status":"thinking",
          "status_detail":"reading the spec",
          "progress":40
        }
        """#.utf8)

        let agent = try JSONDecoder().decode(AgentPresence.self, from: data)
        XCTAssertEqual(agent.id, "agent-1")
        XCTAssertEqual(agent.status, "thinking")
        XCTAssertEqual(agent.statusDetail, "reading the spec")
        XCTAssertEqual(agent.progress, 40)
    }

    func testRoomActivityCanAdvanceWithoutDroppingRoomMetadata() throws {
        let data = Data(#"""
        {
          "room_id":"room-1",
          "name":"Design",
          "description":"UI work",
          "ephemeral":true,
          "created_at":"2026-07-11T12:00:00Z",
          "visibility":"private",
          "member_count":2,
          "encrypted":true
        }
        """#.utf8)

        let room = try JSONDecoder().decode(Room.self, from: data)
            .updating(lastActivity: "2026-08-04T21:30:00Z")

        XCTAssertEqual(room.lastActivity, "2026-08-04T21:30:00Z")
        XCTAssertEqual(room.description, "UI work")
        XCTAssertEqual(room.memberCount, 2)
        XCTAssertTrue(room.ephemeral)
        XCTAssertTrue(room.encrypted)
    }

    @MainActor
    func testHistoryMergePreservesLiveMessagesAndDeduplicates() throws {
        func message(_ id: String, _ seq: Int, _ content: String) throws -> ChatMessage {
            let json: [String: Any] = [
                "message_id": id,
                "room_id": "room",
                "agent_id": "agent",
                "agent_name": "Agent",
                "content": content,
                "timestamp": "2026-07-11T12:00:00Z",
                "seq": seq,
            ]
            return try JSONDecoder().decode(
                ChatMessage.self,
                from: JSONSerialization.data(withJSONObject: json)
            )
        }

        let historical = try message("m1", 1, "history")
        let duplicateLive = try message("m1", 1, "history")
        let arrivedDuringLoad = try message("m2", 2, "live")
        let merged = ChatStore.merging(
            history: [historical],
            live: [duplicateLive, arrivedDuringLoad]
        )
        XCTAssertEqual(merged.map(\.id), ["m1", "m2"])
        XCTAssertEqual(merged.map(\.content), ["history", "live"])
    }
}
