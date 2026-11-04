import Foundation

struct RoomLocalPreferences {
    static let archivedRoomIDsKey = "CowchatMac.archivedRoomIDs"
    // Older builds also wrote per-scope UserDefaults entries for a since-removed
    // pin-state feature (key names dropped here); those entries are intentionally
    // left orphaned, not migrated or cleared.
    static let pendingSetupRoomIDsKey = "CowchatMac.pendingSetupRoomIDs"
    // The setup-screen takeover was removed in the onboarding redesign
    // (spec 2026-08-07). This key is kept only so ChatStore.init can clear
    // any value a previous build left behind; nothing reads or writes it now.
    static let pendingSetupScreenRoomIDsKey = "CowchatMac.pendingSetupScreenRoomIDs"
    static let roomReadStateKey = "CowchatMac.roomReadState"
    static let showsAllMessagesRoomIDsKey = "CowchatMac.showsAllMessagesRoomIDs"
    static let sessionRoomArchiveKey = "CowchatMac.sessionRoomArchive"

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

    /// Rooms whose switch shows every message.
    var showsAllMessagesRoomIDs: Set<String> {
        loadIDs(forKey: Self.showsAllMessagesRoomIDsKey)
    }

    /// What the archive rule for ended sessions did with each session room, so it handles each ended session once.
    var sessionRoomArchiveOutcomes: [String: SessionRoomArchive.Outcome] {
        guard let stored = defaults.dictionary(forKey: scopedKey(Self.sessionRoomArchiveKey)) as? [String: String]
        else { return [:] }
        return stored.compactMapValues(SessionRoomArchive.Outcome.init(rawValue:))
    }

    func saveSessionRoomArchiveOutcomes(_ outcomes: [String: SessionRoomArchive.Outcome]) {
        defaults.set(outcomes.mapValues(\.rawValue), forKey: scopedKey(Self.sessionRoomArchiveKey))
    }

    var roomReadState: RoomReadState? {
        guard let data = defaults.data(forKey: scopedKey(Self.roomReadStateKey)) else { return nil }
        return try? JSONDecoder().decode(RoomReadState.self, from: data)
    }

    func saveRoomReadState(_ state: RoomReadState) {
        guard let data = try? JSONEncoder().encode(state) else { return }
        defaults.set(data, forKey: scopedKey(Self.roomReadStateKey))
    }

    func saveArchivedRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.archivedRoomIDsKey)
    }

    func savePendingSetupRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.pendingSetupRoomIDsKey)
    }

    func saveShowsAllMessagesRoomIDs(_ roomIDs: Set<String>) {
        save(roomIDs, forKey: Self.showsAllMessagesRoomIDsKey)
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
