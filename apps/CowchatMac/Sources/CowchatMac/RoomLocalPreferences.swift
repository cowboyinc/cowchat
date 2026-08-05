import Foundation

struct RoomLocalPreferences {
    static let archivedRoomIDsKey = "CowchatMac.archivedRoomIDs"
    static let pinnedRoomIDsKey = "CowchatMac.pinnedRoomIDs"
    static let pinnedRoomsInitializedKey = "CowchatMac.pinnedRoomsInitialized"
    static let pendingSetupRoomIDsKey = "CowchatMac.pendingSetupRoomIDs"
    static let pendingSetupScreenRoomIDsKey = "CowchatMac.pendingSetupScreenRoomIDs"

    private let defaults: UserDefaults
    private let scope: String?

    init(defaults: UserDefaults, scope: String? = nil) {
        self.defaults = defaults
        self.scope = scope?.isEmpty == false ? scope : nil
    }

    var archivedRoomIDs: Set<String> {
        loadIDs(forKey: Self.archivedRoomIDsKey)
    }

    var pinnedRoomIDs: Set<String> {
        loadIDs(forKey: Self.pinnedRoomIDsKey)
    }

    var hasInitializedPinnedRooms: Bool {
        defaults.bool(forKey: scopedKey(Self.pinnedRoomsInitializedKey))
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
        defaults.set(true, forKey: scopedKey(Self.pinnedRoomsInitializedKey))
    }

    func savePendingSetupRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.pendingSetupRoomIDsKey)
    }

    func savePendingSetupScreenRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.pendingSetupScreenRoomIDsKey)
    }

    private func loadIDs(forKey key: String) -> Set<String> {
        Set(defaults.stringArray(forKey: scopedKey(key)) ?? [])
    }

    private func save(_ roomIDs: Set<String>, forKey key: String) {
        defaults.set(roomIDs.sorted(), forKey: scopedKey(key))
    }

    private func scopedKey(_ key: String) -> String {
        guard let scope else { return key }
        return "\(key).\(scope)"
    }
}
