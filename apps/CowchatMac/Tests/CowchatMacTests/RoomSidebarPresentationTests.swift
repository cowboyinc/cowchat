import XCTest
@testable import CowchatMac

final class RoomSidebarPresentationTests: XCTestCase {
    func testRelativeTimeNeverRendersTheFuture() {
        // A TimelineView tick date is captured when the view was last
        // evaluated, so an event that just arrived can be newer than it.
        let tick = Date()
        let arrivedAfterTick = tick.addingTimeInterval(45)
        let formatted = ISO8601DateFormatter().string(from: arrivedAfterTick)

        XCTAssertEqual(formatted.cowchatRelativeTime(relativeTo: tick), "now")
    }

    func testRelativeTimeRendersNowRatherThanInZeroSeconds() {
        // The delta rounding to zero is the case that produced "in 0s" for a
        // room that had just been created.
        let now = Date()
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let justNow = formatter.string(from: now.addingTimeInterval(-0.2))

        XCTAssertEqual(justNow.cowchatRelativeTime(relativeTo: now), "now")
    }

    func testRelativeTimeStillRendersThePast() {
        let now = Date()
        let anHourAgo = ISO8601DateFormatter().string(from: now.addingTimeInterval(-3600))

        XCTAssertFalse(anHourAgo.cowchatRelativeTime(relativeTo: now).isEmpty)
        XCTAssertFalse(anHourAgo.cowchatRelativeTime(relativeTo: now).contains("in "))
    }

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
                fallbackMemberCount: 99
            ),
            "Claude active"
        )
        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [members[0]],
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 1
            ),
            "No agents connected"
        )
    }

    func testChatPresenceUsesNewerRoomCountWhenMemberSnapshotOnlyContainsThisMac() {
        let mac = AgentPresence(agentID: "cowchat-mac", name: "Cowchat Mac")

        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [mac],
                currentAgentID: mac.id,
                fallbackMemberCount: 3
            ),
            "2 agents connected"
        )
    }

    func testChatPresenceDoesNotSubtractThisMacBeforeItHasJoinedTheRoom() {
        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [],
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 1
            ),
            "1 agent connected"
        )
    }

    func testChatPresenceSubtractsThisMacOnlyWhenFallbackSnapshotIncludesIt() {
        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [],
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 1,
                fallbackMemberCountIncludesCurrentAgent: true
            ),
            "No agents connected"
        )
    }

    func testChatPresenceShowsRecentThinkingAfterOneShotAgentDisconnects() {
        let now = Date()

        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [],
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 0,
                recentActivityByAgent: ["claude": now.addingTimeInterval(-30)],
                now: now
            ),
            "1 agent active recently"
        )
        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [],
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 0,
                recentActivityByAgent: ["claude": now.addingTimeInterval(-121)],
                now: now
            ),
            "No agents connected"
        )
    }

    func testChatPresenceCombinesRecentActivityWithConnectedCount() {
        let now = Date()
        let members = [
            AgentPresence(agentID: "claude", name: "Claude"),
            AgentPresence(agentID: "codex", name: "Codex"),
            AgentPresence(agentID: "reviewer", name: "Reviewer"),
        ]

        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: members,
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: nil,
                recentActivityByAgent: ["claude": now.addingTimeInterval(-30)],
                now: now
            ),
            "1 agent active recently · 3 agents connected"
        )
    }

    func testChatPresenceNeverClaimsCachedMembersAreLiveWhileOfflineOrConnecting() {
        let members = [AgentPresence(agentID: "cowchat-mac", name: "Cowchat Mac")]

        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: [],
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 2,
                fallbackMemberCountIncludesCurrentAgent: false,
                connectionStatus: .failed("offline")
            ),
            "Offline"
        )
        XCTAssertEqual(
            ChatPresencePresentation.summary(
                members: members,
                currentAgentID: "cowchat-mac",
                fallbackMemberCount: 1,
                fallbackMemberCountIncludesCurrentAgent: true,
                connectionStatus: .connecting
            ),
            "Connecting…"
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

        XCTAssertEqual(updated["design"]?["claude"], now)
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
        // The unrelated transition must still apply after the prune pass.
        XCTAssertNotNil(updated["other"]?["claude"])
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
            createdAt: lastActivity,
            createdBy: nil,
            visibility: "public",
            lastActivity: lastActivity,
            memberCount: memberCount,
            encrypted: false
        )
    }
}
