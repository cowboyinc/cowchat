import XCTest
@testable import CowchatMac

final class RoomReadStateTests: XCTestCase {
    /// Memberwise fixture (Room's custom Codable init is bypassed), matching
    /// the construction pattern in RoomSidebarPresentationTests.makeRoom.
    private func room(_ id: String, name: String = "room", activity: String) -> Room {
        Room(
            roomID: id,
            name: name,
            description: nil,
            parentID: nil,
            createdAt: activity,
            createdBy: nil,
            visibility: "public",
            lastActivity: activity,
            memberCount: 1,
            encrypted: false
        )
    }

    func testSeedMarksEverythingRead() {
        var state = RoomReadState()
        let r = room("a", activity: "2026-08-05T10:00:00Z")
        state.seed(rooms: [r])
        XCTAssertTrue(state.hasSeeded)
        XCTAssertFalse(state.isUnread(r, selectedRoomID: nil))
    }

    func testNewerActivityIsUnreadUntilMarkedRead() {
        var state = RoomReadState()
        state.seed(rooms: [room("a", activity: "2026-08-05T10:00:00Z")])
        let updated = room("a", activity: "2026-08-05T11:00:00Z")
        XCTAssertTrue(state.isUnread(updated, selectedRoomID: nil))
        state.markRead(roomID: "a", activityDate: updated.activityDate)
        XCTAssertFalse(state.isUnread(updated, selectedRoomID: nil))
    }

    func testSelectedRoomIsNeverUnread() {
        var state = RoomReadState()
        state.seed(rooms: [room("a", activity: "2026-08-05T10:00:00Z")])
        let updated = room("a", activity: "2026-08-05T11:00:00Z")
        XCTAssertFalse(state.isUnread(updated, selectedRoomID: "a"))
    }

    func testUnseededOrUnknownRoomIsNotUnreadBeforeSeedButUnknownAfter() {
        var state = RoomReadState()
        let r = room("a", activity: "2026-08-05T10:00:00Z")
        XCTAssertFalse(state.isUnread(r, selectedRoomID: nil)) // pre-seed: quiet
        state.seed(rooms: [])
        XCTAssertTrue(state.isUnread(r, selectedRoomID: nil))  // post-seed unknown: unread
    }

    func testReconcilePrunesAndRoundTripsThroughCodable() throws {
        var state = RoomReadState()
        state.seed(rooms: [room("a", activity: "2026-08-05T10:00:00Z"),
                           room("b", activity: "2026-08-05T10:00:00Z")])
        state.reconcile(validRoomIDs: ["a"])
        XCTAssertNil(state.entries["b"])
        let decoded = try JSONDecoder().decode(
            RoomReadState.self, from: JSONEncoder().encode(state)
        )
        XCTAssertEqual(decoded, state)
    }
}
