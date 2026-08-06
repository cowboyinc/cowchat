import XCTest
@testable import CowchatMac

final class RoomSidebarPresentationTests: XCTestCase {
    func testSortedByRecencyOrdersByActivityDescending() {
        // Lobby gets no special treatment — it lives in its own nav row now.
        let lobby = makeRoom(name: "Lobby", lastActivity: "2026-08-01T00:00:00Z")
        let older = makeRoom(name: "alpha", lastActivity: "2026-08-03T00:00:00Z")
        let newer = makeRoom(name: "zulu", lastActivity: "2026-08-04T00:00:00Z")
        let sorted = RoomSidebarPresentation.sortedByRecency([older, lobby, newer])
        XCTAssertEqual(sorted.map(\.name), ["zulu", "alpha", "Lobby"])
    }

    func testSortedByRecencyTiebreaksOnName() {
        let a = makeRoom(name: "beta", lastActivity: "2026-08-04T00:00:00Z")
        let b = makeRoom(name: "Alpha", lastActivity: "2026-08-04T00:00:00Z")
        XCTAssertEqual(RoomSidebarPresentation.sortedByRecency([a, b]).map(\.name), ["Alpha", "beta"])
    }

    func testMessageMatchesCanSurfaceRoomWithoutNameMatch() {
        let design = makeRoom(id: "design", name: "Design")
        let release = makeRoom(id: "release", name: "Release")

        XCTAssertEqual(
            RoomSidebarPresentation.filteredRooms(
                from: [design, release],
                query: "  deployment  ",
                matchingMessageRoomIDs: ["release"]
            ).map(\.id),
            ["release"]
        )
    }

    func testLobbyAvailableAgentCountExcludesThisMacClientAndDeduplicates() throws {
        func agent(_ id: String) throws -> AgentPresence {
            try JSONDecoder().decode(
                AgentPresence.self,
                from: Data(#"{"agent_id":"\#(id)","name":"Agent","capabilities":[]}"#.utf8)
            )
        }
        let members = try [agent("cowchat-mac"), agent("bot-a"), agent("bot-a"), agent("bot-b")]

        XCTAssertEqual(
            LobbyPresentation.availableAgentCount(from: members, excluding: "cowchat-mac"),
            2
        )
    }

    func testChatPresenceNamesOnlyActiveCollaboratorsAndNeverThisMacClient() throws {
        func agent(_ id: String, name: String, status: String?) throws -> AgentPresence {
            var json: [String: Any] = [
                "agent_id": id,
                "name": name,
                "capabilities": [],
            ]
            if let status { json["status"] = status }
            return try JSONDecoder().decode(
                AgentPresence.self,
                from: JSONSerialization.data(withJSONObject: json)
            )
        }
        let members = try [
            agent("cowchat-mac", name: "Cowchat Mac", status: "working"),
            agent("claude", name: "Claude", status: "thinking"),
            agent("codex", name: "Codex", status: nil),
        ]

        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: members,
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 99,
                isConnected: true
            ),
            "Claude active"
        )
        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [members[0]],
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 1,
                isConnected: true
            ),
            "No collaborators"
        )
    }

    func testWorkingPredicateHonorsWindow() {
        let now = Date()
        XCTAssertFalse(RoomSidebarPresentation.isWorking(thinkingByAgent: nil, now: now, window: 120))
        XCTAssertTrue(RoomSidebarPresentation.isWorking(
            thinkingByAgent: ["claude": now.addingTimeInterval(-30)], now: now, window: 120))
        XCTAssertFalse(RoomSidebarPresentation.isWorking(
            thinkingByAgent: ["claude": now.addingTimeInterval(-121)], now: now, window: 120))
    }

    func testWorkingPredicateTrueWhenAnyAgentIsFresh() {
        let now = Date()
        XCTAssertTrue(RoomSidebarPresentation.isWorking(
            thinkingByAgent: [
                "claude": now.addingTimeInterval(-200),
                "codex": now.addingTimeInterval(-10),
            ],
            now: now,
            window: 120
        ))
    }

    func testWorkingPredicateFalseWhenAllAgentsExpired() {
        let now = Date()
        XCTAssertFalse(RoomSidebarPresentation.isWorking(
            thinkingByAgent: [
                "claude": now.addingTimeInterval(-200),
                "codex": now.addingTimeInterval(-150),
            ],
            now: now,
            window: 120
        ))
    }

    func testUpdatedThinkingByAgentStampsThinkingAgent() throws {
        let now = Date()
        let message = try makeMessage(roomID: "design", agentID: "claude", isThinking: true)

        let updated = RoomSidebarPresentation.updatedThinkingByAgent([:], message: message, now: now)

        XCTAssertEqual(updated["design"]?["claude"], message.timestamp.cowchatDate)
    }

    func testUpdatedThinkingByAgentClearsOnlyThatAgentsEntry() throws {
        let now = Date()
        let existing: [String: [String: Date]] = [
            "design": [
                "claude": now.addingTimeInterval(-10),
                "codex": now.addingTimeInterval(-5),
            ],
        ]
        let message = try makeMessage(roomID: "design", agentID: "claude", isThinking: false)

        let updated = RoomSidebarPresentation.updatedThinkingByAgent(existing, message: message, now: now)

        XCTAssertNil(updated["design"]?["claude"])
        XCTAssertNotNil(updated["design"]?["codex"])
    }

    func testUpdatedThinkingByAgentPrunesLongExpiredEntries() throws {
        let now = Date()
        let existing: [String: [String: Date]] = [
            "stale": ["ghost": now.addingTimeInterval(-700)],
            "mixed": [
                "ghost": now.addingTimeInterval(-700),
                "fresh": now.addingTimeInterval(-10),
            ],
        ]
        let message = try makeMessage(roomID: "other", agentID: "claude", isThinking: true)

        let updated = RoomSidebarPresentation.updatedThinkingByAgent(existing, message: message, now: now)

        XCTAssertNil(updated["stale"])
        XCTAssertNil(updated["mixed"]?["ghost"])
        XCTAssertNotNil(updated["mixed"]?["fresh"])
    }

    func testUpdatedThinkingByAgentPrunesRoomWhenLastAgentClears() throws {
        let now = Date()
        let existing: [String: [String: Date]] = [
            "design": ["claude": now.addingTimeInterval(-10)],
        ]
        let message = try makeMessage(roomID: "design", agentID: "claude", isThinking: false)

        let updated = RoomSidebarPresentation.updatedThinkingByAgent(existing, message: message, now: now)

        XCTAssertNil(updated["design"])
    }

    /// `ChatMessage` decodes only (its `init(from:)` suppresses the memberwise
    /// initializer), so fixtures go through JSONDecoder like the AgentPresence
    /// helpers above rather than direct construction.
    private func makeMessage(
        roomID: String,
        agentID: String,
        isThinking: Bool,
        timestamp: String = "2026-08-04T12:00:00Z"
    ) throws -> ChatMessage {
        var json: [String: Any] = [
            "message_id": UUID().uuidString,
            "room_id": roomID,
            "agent_id": agentID,
            "agent_name": agentID,
            "content": "hello",
            "timestamp": timestamp,
            "seq": 1,
        ]
        if isThinking {
            json["metadata"] = ["type": "thinking"]
        }
        return try JSONDecoder().decode(
            ChatMessage.self,
            from: JSONSerialization.data(withJSONObject: json)
        )
    }

    private func makeRoom(
        id: String? = nil,
        name: String,
        lastActivity: String = "2026-08-04T12:00:00Z",
        memberCount: Int? = 1
    ) -> Room {
        Room(
            roomID: id ?? name,
            name: name,
            description: nil,
            parentID: nil,
            ephemeral: false,
            createdAt: lastActivity,
            createdBy: nil,
            visibility: "public",
            lastActivity: lastActivity,
            memberCount: memberCount,
            encrypted: false
        )
    }
}
