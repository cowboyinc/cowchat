import XCTest
@testable import CowchatMac

final class RoomSidebarPresentationTests: XCTestCase {
    func testSortedByRecencyKeepsLobbyFirstThenRecency() {
        let lobby = makeRoom(name: "Lobby", lastActivity: "2026-08-01T00:00:00Z")
        let older = makeRoom(name: "alpha", lastActivity: "2026-08-03T00:00:00Z")
        let newer = makeRoom(name: "zulu", lastActivity: "2026-08-04T00:00:00Z")
        let sorted = RoomSidebarPresentation.sortedByRecency([older, lobby, newer])
        XCTAssertEqual(sorted.map(\.name), ["Lobby", "zulu", "alpha"])
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
        XCTAssertFalse(RoomSidebarPresentation.isWorking(lastThinkingAt: nil, now: now, window: 120))
        XCTAssertTrue(RoomSidebarPresentation.isWorking(
            lastThinkingAt: now.addingTimeInterval(-30), now: now, window: 120))
        XCTAssertFalse(RoomSidebarPresentation.isWorking(
            lastThinkingAt: now.addingTimeInterval(-121), now: now, window: 120))
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
