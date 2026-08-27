import XCTest
@testable import CowchatMac

final class ProtocolModelsTests: XCTestCase {
    func testMacClientAdvertisesLifecycleCapableProtocolVersion() {
        XCTAssertEqual(CowchatWireProtocol.currentVersion, 2)
    }

    func testRoomDecodesServerPayloadAndDefaultsEncryption() throws {
        let data = Data(#"""
        {
          "room_id":"lobby",
          "name":"lobby",
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
        XCTAssertFalse(message.isThinking)
    }

    func testMessageToleratesArbitraryMetadataShapes() throws {
        let metadataValues: [Any] = [42, ["custom", true], ["type": 1]]

        for (index, metadata) in metadataValues.enumerated() {
            let data = try JSONSerialization.data(withJSONObject: [
                "message_id": "message-\(index)",
                "room_id": "lobby",
                "agent_id": "agent-1",
                "agent_name": "Codex",
                "content": "still visible",
                "metadata": metadata,
                "timestamp": "2026-07-11T12:00:00.123Z",
                "seq": index + 1,
            ])

            let message = try JSONDecoder().decode(ChatMessage.self, from: data)
            XCTAssertEqual(message.content, "still visible")
            XCTAssertFalse(message.isThinking)
        }
    }

    func testMessageDecodesValidStructuredHandoff() throws {
        let data = Data(#"""
        {
          "message_id":"handoff-1",
          "room_id":"handoffs",
          "agent_id":"builder",
          "agent_name":"Builder",
          "content":"Handoff ready",
          "metadata":{
            "kind":"handoff.ready",
            "handoff":{
              "version":1,
              "summary":"Auth change complete",
              "next":"Review expiry tests",
              "risks":["Coverage is incomplete"],
              "refs":["git:abc123"]
            }
          },
          "timestamp":"2026-07-11T12:00:00.123Z",
          "seq":42
        }
        """#.utf8)

        let message = try JSONDecoder().decode(ChatMessage.self, from: data)
        XCTAssertEqual(message.handoff?.summary, "Auth change complete")
        XCTAssertEqual(message.handoff?.next, "Review expiry tests")
        XCTAssertEqual(message.handoff?.refs, ["git:abc123"])
    }

    func testMalformedHandoffFallsBackToNormalMessage() throws {
        let data = Data(#"""
        {
          "message_id":"handoff-malformed",
          "room_id":"handoffs",
          "agent_id":"builder",
          "agent_name":"Builder",
          "content":"Keep this visible",
          "metadata":{"kind":"handoff.ready","handoff":{"version":1,"summary":"missing next"}},
          "timestamp":"2026-07-11T12:00:00.123Z",
          "seq":43
        }
        """#.utf8)

        let message = try JSONDecoder().decode(ChatMessage.self, from: data)
        XCTAssertNil(message.handoff)
        XCTAssertEqual(message.content, "Keep this visible")
    }

    func testMessageArrivalIdentityChangesWhenTheCappedWindowAdvances() throws {
        func message(_ sequence: Int) throws -> ChatMessage {
            try JSONDecoder().decode(
                ChatMessage.self,
                from: JSONSerialization.data(withJSONObject: [
                    "message_id": "message-\(sequence)",
                    "room_id": "lobby",
                    "agent_id": "agent-1",
                    "agent_name": "Codex",
                    "content": "message \(sequence)",
                    "timestamp": "2026-07-11T12:00:00.123Z",
                    "seq": sequence,
                ])
            )
        }

        let before = try (1...200).map(message)
        let after = try (2...201).map(message)

        XCTAssertEqual(before.count, after.count)
        XCTAssertNotEqual(
            MessageArrivalIdentity.latest(in: before),
            MessageArrivalIdentity.latest(in: after)
        )
        XCTAssertEqual(MessageArrivalIdentity.latest(in: after)?.sequence, 201)
    }

    @MainActor
    func testThinkingHistoryRowsAreIdentifiedAndFilteredFromChat() throws {
        let data = Data(#"""
        {
          "message_id":"thinking-1",
          "room_id":"lobby",
          "agent_id":"agent-1",
          "agent_name":"Codex",
          "content":"private reasoning pulse",
          "metadata":{"type":"thinking"},
          "timestamp":"2026-07-11T12:00:00.123Z",
          "seq":42
        }
        """#.utf8)

        let message = try JSONDecoder().decode(ChatMessage.self, from: data)

        XCTAssertTrue(message.isThinking)
        XCTAssertTrue(ChatStore.visibleMessages(in: [message]).isEmpty)
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

    func testServerErrorCarriesCodeAndMessage() {
        let error = CowchatConnectionError.server(message: "Room name 'General' already taken", code: "room_name_taken")
        guard case .server(let message, let code) = error else {
            return XCTFail("expected .server")
        }
        XCTAssertEqual(message, "Room name 'General' already taken")
        XCTAssertEqual(code, "room_name_taken")
        XCTAssertEqual(error.errorDescription, "Room name 'General' already taken")
    }
}
