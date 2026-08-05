import XCTest
@testable import CowchatMac

final class RoomLocalPreferencesTests: XCTestCase {
    func testArchiveAndPinSelectionsRoundTripLocally() {
        let suiteName = "RoomLocalPreferencesTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = RoomLocalPreferences(defaults: defaults)

        XCTAssertEqual(preferences.archivedRoomIDs, [])
        XCTAssertEqual(preferences.pinnedRoomIDs, [])
        XCTAssertFalse(preferences.hasInitializedPinnedRooms)
        XCTAssertEqual(preferences.pendingSetupRoomIDs, [])
        XCTAssertEqual(preferences.pendingSetupScreenRoomIDs, [])

        preferences.saveArchivedRoomIDs(["room-b", "room-a"])
        preferences.savePinnedRoomIDs(["lobby", "room-a"])
        preferences.savePendingSetupRoomIDs(["room-b"])
        preferences.savePendingSetupScreenRoomIDs(["room-b"])

        let reloaded = RoomLocalPreferences(defaults: defaults)
        XCTAssertEqual(reloaded.archivedRoomIDs, ["room-a", "room-b"])
        XCTAssertEqual(reloaded.pinnedRoomIDs, ["lobby", "room-a"])
        XCTAssertTrue(reloaded.hasInitializedPinnedRooms)
        XCTAssertEqual(reloaded.pendingSetupRoomIDs, ["room-b"])
        XCTAssertEqual(reloaded.pendingSetupScreenRoomIDs, ["room-b"])
    }
}
