import XCTest
@testable import CowchatMac

final class RoomSidebarPresentationTests: XCTestCase {
    func testPinnedRoomsPreferLobbyAndRespectLimit() throws {
        let rooms = try [
            room(id: "recent", name: "Recent", activity: "2026-08-04T15:00:00Z"),
            room(id: "lobby", name: "Lobby", activity: "2026-08-03T15:00:00Z"),
            room(id: "assistant", name: "Assistant", activity: "2026-08-02T15:00:00Z"),
            room(id: "demo", name: "Demo", activity: "2026-08-01T15:00:00Z"),
        ]

        XCTAssertEqual(
            RoomSidebarPresentation.pinnedRooms(from: rooms).map(\.id),
            ["lobby", "recent", "assistant"]
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
            RoomSidebarPresentation.visiblePinnedRooms(from: allRooms, among: visibleRooms).map(\.id),
            ["design"]
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
