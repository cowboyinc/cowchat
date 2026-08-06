import Foundation

struct RoomLocalPreferences {
    static let archivedRoomIDsKey = "CowchatMac.archivedRoomIDs"
    // Older builds also wrote per-scope UserDefaults entries for a since-removed
    // pin-state feature (key names dropped here); those entries are intentionally
    // left orphaned, not migrated or cleared.
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

    var pendingSetupRoomIDs: Set<String> {
        loadIDs(forKey: Self.pendingSetupRoomIDsKey)
    }

    var pendingSetupScreenRoomIDs: Set<String> {
        loadIDs(forKey: Self.pendingSetupScreenRoomIDsKey)
    }

    func saveArchivedRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.archivedRoomIDsKey)
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
