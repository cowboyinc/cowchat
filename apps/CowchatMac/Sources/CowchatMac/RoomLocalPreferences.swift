import Foundation

struct RoomLocalPreferences {
    static let archivedRoomIDsKey = "CowchatMac.archivedRoomIDs"
    static let pinnedRoomIDsKey = "CowchatMac.pinnedRoomIDs"
    static let pinnedRoomsInitializedKey = "CowchatMac.pinnedRoomsInitialized"
    static let pendingSetupRoomIDsKey = "CowchatMac.pendingSetupRoomIDs"
    static let pendingSetupScreenRoomIDsKey = "CowchatMac.pendingSetupScreenRoomIDs"

    private let defaults: UserDefaults

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    var archivedRoomIDs: Set<String> {
        loadIDs(forKey: Self.archivedRoomIDsKey)
    }

    var pinnedRoomIDs: Set<String> {
        loadIDs(forKey: Self.pinnedRoomIDsKey)
    }

    var hasInitializedPinnedRooms: Bool {
        defaults.bool(forKey: Self.pinnedRoomsInitializedKey)
    }

    var pendingSetupRoomIDs: Set<String> {
        loadIDs(forKey: Self.pendingSetupRoomIDsKey)
    }

    var pendingSetupScreenRoomIDs: Set<String> {
        loadIDs(forKey: Self.pendingSetupScreenRoomIDsKey)
    }

    func saveArchivedRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.archivedRoomIDsKey)
    }

    func savePinnedRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.pinnedRoomIDsKey)
        defaults.set(true, forKey: Self.pinnedRoomsInitializedKey)
    }

    func savePendingSetupRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.pendingSetupRoomIDsKey)
    }

    func savePendingSetupScreenRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.pendingSetupScreenRoomIDsKey)
    }

    private func loadIDs(forKey key: String) -> Set<String> {
        Set(defaults.stringArray(forKey: key) ?? [])
    }

    private func save(_ roomIDs: Set<String>, forKey key: String) {
        defaults.set(roomIDs.sorted(), forKey: key)
    }
}
