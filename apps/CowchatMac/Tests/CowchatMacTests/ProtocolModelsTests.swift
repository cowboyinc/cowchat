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
    func testRoomInviteDecodesListPayloadShape() throws {
        let data = Data("""
        {
          "invite_id":"a3f1c2d4e5f60718293a4b5c6d7e8f901234567890abcdef1234567890abcdef",
          "room_id":"room-9",
          "single_use":false,
          "redeemed_count":3,
          "revoked":false,
          "created_at":"2026-08-13T18:00:00Z",
          "mine":true
        }
        """.utf8)

        let invite = try JSONDecoder().decode(RoomInvite.self, from: data)
        XCTAssertEqual(invite.id.prefix(8), "a3f1c2d4")
        XCTAssertEqual(invite.roomID, "room-9")
        XCTAssertFalse(invite.singleUse)
        XCTAssertEqual(invite.redeemedCount, 3)
        XCTAssertFalse(invite.revoked)
        XCTAssertTrue(invite.mine)
    }
    func testFileAttachmentParsesFromMessageMetadata() throws {
        let data = Data("""
        {
          "message_id":"m-file",
          "room_id":"r1",
          "agent_id":"claude-b",
          "agent_name":"claude-b",
          "content":"budget model attached",
          "timestamp":"2026-08-20T18:00:00Z",
          "seq":9,
          "metadata":{"type":"file","blob_id":"b-123","name":"budget-levers.html","sha256":"abc","size":163618}
        }
        """.utf8)

        let message = try JSONDecoder().decode(ChatMessage.self, from: data)
        let attachment = try XCTUnwrap(message.fileAttachment)
        XCTAssertEqual(attachment.blobID, "b-123")
        XCTAssertEqual(attachment.name, "budget-levers.html")
        XCTAssertEqual(attachment.size, 163618)
        XCTAssertFalse(message.isThinking)
    }

    func testPlainMessagesHaveNoAttachment() throws {
        let data = Data("""
        {
          "message_id":"m1","room_id":"r1","agent_id":"a","agent_name":"a",
          "content":"hi","timestamp":"2026-08-20T18:00:00Z","seq":1
        }
        """.utf8)
        XCTAssertNil(try JSONDecoder().decode(ChatMessage.self, from: data).fileAttachment)
    }
}
