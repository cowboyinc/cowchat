import Foundation

/// Per-room last-seen state, modeled on the cowboy app's ConversationReadState:
/// epoch-milliseconds map, seeded all-read on first run, marked read on selection.
struct RoomReadState: Codable, Equatable {
    private(set) var hasSeeded = false
    private(set) var entries: [String: Double] = [:]

    static func milliseconds(from date: Date) -> Double {
        (date.timeIntervalSince1970 * 1000).rounded()
    }

    mutating func seed(rooms: [Room]) {
        guard !hasSeeded else { return }
        for room in rooms {
            if let activity = room.activityDate {
                entries[room.id] = Self.milliseconds(from: activity)
            } else {
                entries[room.id] = 0
            }
        }
        hasSeeded = true
    }

    mutating func markRead(roomID: String, activityDate: Date?) {
        let stamp = activityDate.map(Self.milliseconds(from:)) ?? 0
        if stamp >= (entries[roomID] ?? -1) { entries[roomID] = stamp }
    }

    func isUnread(_ room: Room, selectedRoomID: String?) -> Bool {
        guard hasSeeded, room.id != selectedRoomID,
              let activity = room.activityDate else { return false }
        guard let seen = entries[room.id] else { return true }
        return Self.milliseconds(from: activity) > seen
    }

    mutating func reconcile(validRoomIDs: Set<String>) {
        entries = entries.filter { validRoomIDs.contains($0.key) }
    }
}
