import XCTest
@testable import CowchatMac

final class RoomSidebarPresentationTests: XCTestCase {
    func testPinnedRoomsUseExplicitLocalPreferenceAndRespectLimit() throws {
        let rooms = try [
            room(id: "recent", name: "Recent", activity: "2026-08-04T15:00:00Z"),
            room(id: "lobby", name: "Lobby", activity: "2026-08-03T15:00:00Z"),
            room(id: "assistant", name: "Assistant", activity: "2026-08-02T15:00:00Z"),
            room(id: "demo", name: "Demo", activity: "2026-08-01T15:00:00Z"),
        ]

        XCTAssertEqual(
            RoomSidebarPresentation.pinnedRooms(
                from: rooms,
                pinnedRoomIDs: ["lobby", "assistant", "demo"]
            ).map(\.id),
            ["lobby", "assistant", "demo"]
        )
    }

    func testGroupsRoomsByCalendarRecency() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        let now = try XCTUnwrap("2026-08-04T17:00:00Z".cowchatDate)
        let rooms = try [
            room(id: "today", name: "Today", activity: "2026-08-04T12:00:00Z"),
            room(id: "yesterday", name: "Yesterday", activity: "2026-08-03T12:00:00Z"),
            room(id: "week", name: "Week", activity: "2026-07-31T12:00:00Z"),
            room(id: "earlier", name: "Earlier", activity: "2026-07-01T12:00:00Z"),
        ]

        let groups = RoomSidebarPresentation.groups(from: rooms, now: now, calendar: calendar)

        XCTAssertEqual(groups.map(\.title), ["Today", "Yesterday", "This week", "Earlier"])
        XCTAssertEqual(groups.flatMap(\.rooms).map(\.id), ["today", "yesterday", "week", "earlier"])
    }

    func testActiveRoomsRequireAtLeastOneReportedMember() throws {
        let active = try room(id: "active", name: "Active", memberCount: 2)
        let quiet = try room(id: "quiet", name: "Quiet", memberCount: 0)
        let unknown = try room(id: "unknown", name: "Unknown", memberCount: nil)

        XCTAssertEqual(
            RoomSidebarPresentation.activeRooms(from: [active, quiet, unknown]).map(\.id),
            ["active"]
        )
    }

    func testActiveRoomsDoNotCountThisMacClientInSelectedRoom() throws {
        let selected = try room(id: "selected", name: "Selected", memberCount: 1)
        let collaborator = try room(id: "other", name: "Other", memberCount: 1)

        XCTAssertEqual(
            RoomSidebarPresentation.activeRooms(
                from: [selected, collaborator],
                excludingCurrentClientFrom: selected.id
            ).map(\.id),
            ["other"]
        )
    }

    func testVisiblePinnedRoomsRespectSearchAndScopeResults() throws {
        let lobby = try room(id: "lobby", name: "Lobby", memberCount: 0)
        let design = try room(id: "design", name: "Design", memberCount: 2)
        let release = try room(id: "release", name: "Release", memberCount: 1)
        let support = try room(id: "support", name: "Support", memberCount: 3)
        let allRooms = [lobby, design, release, support]
        let visibleRooms = RoomSidebarPresentation.filteredRooms(
            from: RoomSidebarPresentation.activeRooms(from: allRooms),
            query: "design"
        )

        XCTAssertEqual(
            RoomSidebarPresentation.visiblePinnedRooms(
                from: allRooms,
                among: visibleRooms,
                pinnedRoomIDs: ["lobby", "design", "release"]
            ).map(\.id),
            ["design"]
        )
    }

    func testFourthPinnedRoomRemainsInRecencyGroups() throws {
        let rooms = try [
            room(id: "one", name: "One"),
            room(id: "two", name: "Two"),
            room(id: "three", name: "Three"),
            room(id: "four", name: "Four"),
        ]

        XCTAssertEqual(
            RoomSidebarPresentation.roomsForRecencyGroups(
                from: rooms,
                allRooms: rooms,
                pinnedRoomIDs: Set(rooms.map(\.id))
            ).map(\.id),
            ["four"]
        )
    }

    func testMessageMatchesCanSurfaceRoomWithoutNameMatch() throws {
        let design = try room(id: "design", name: "Design")
        let release = try room(id: "release", name: "Release")

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

    private func room(
        id: String,
        name: String,
        activity: String = "2026-08-04T12:00:00Z",
        memberCount: Int? = 1
    ) throws -> Room {
        var json: [String: Any] = [
            "room_id": id,
            "name": name,
            "ephemeral": false,
            "created_at": activity,
            "last_activity": activity,
            "visibility": "public",
        ]
        if let memberCount { json["member_count"] = memberCount }
        return try JSONDecoder().decode(
            Room.self,
            from: JSONSerialization.data(withJSONObject: json)
        )
    }
}
