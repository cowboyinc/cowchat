import XCTest
@testable import CowchatMac

final class RoomLocalPreferencesTests: XCTestCase {
    func testArchiveAndPendingSetupSelectionsRoundTripLocally() {
        let suiteName = "RoomLocalPreferencesTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = RoomLocalPreferences(defaults: defaults)

        XCTAssertEqual(preferences.archivedRoomIDs, [])
        XCTAssertEqual(preferences.pendingSetupRoomIDs, [])
        XCTAssertEqual(preferences.pendingSetupScreenRoomIDs, [])

        preferences.saveArchivedRoomIDs(["room-b", "room-a"])
        preferences.savePendingSetupRoomIDs(["room-b"])
        preferences.savePendingSetupScreenRoomIDs(["room-b"])

        let reloaded = RoomLocalPreferences(defaults: defaults)
        XCTAssertEqual(reloaded.archivedRoomIDs, ["room-a", "room-b"])
        XCTAssertEqual(reloaded.pendingSetupRoomIDs, ["room-b"])
        XCTAssertEqual(reloaded.pendingSetupScreenRoomIDs, ["room-b"])
    }

    func testConnectionScopesDoNotLeakRoomSelections() {
        let suiteName = "RoomLocalPreferencesTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let local = RoomLocalPreferences(defaults: defaults)
        let cloudA = RoomLocalPreferences(defaults: defaults, scope: "cloud-a")
        let cloudB = RoomLocalPreferences(defaults: defaults, scope: "cloud-b")

        local.saveArchivedRoomIDs(["lobby"])
        cloudA.saveArchivedRoomIDs(["cloud-room"])

        XCTAssertEqual(local.archivedRoomIDs, ["lobby"])
        XCTAssertEqual(cloudA.archivedRoomIDs, ["cloud-room"])
        XCTAssertEqual(cloudB.archivedRoomIDs, [])
    }
}
