import Foundation

struct RoomSidebarGroup: Equatable {
    let title: String
    let rooms: [Room]
}

enum RoomSidebarPresentation {
    static func pinnedRooms(from rooms: [Room], limit: Int = 3) -> [Room] {
        let lobby = rooms.first { $0.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame }
        let remaining = rooms.filter { $0.id != lobby?.id }
        return Array(([lobby].compactMap { $0 } + remaining).prefix(limit))
    }

    static func activeRooms(from rooms: [Room]) -> [Room] {
        rooms.filter { ($0.memberCount ?? 0) > 0 }
    }

    static func filteredRooms(from rooms: [Room], query: String) -> [Room] {
        guard !query.isEmpty else { return rooms }
        return rooms.filter {
            $0.name.localizedCaseInsensitiveContains(query)
                || ($0.description?.localizedCaseInsensitiveContains(query) ?? false)
        }
    }

    static func visiblePinnedRooms(
        from allRooms: [Room],
        among visibleRooms: [Room],
        limit: Int = 3
    ) -> [Room] {
        let pinnedIDs = Set(pinnedRooms(from: allRooms, limit: limit).map(\.id))
        return visibleRooms.filter { pinnedIDs.contains($0.id) }
    }

    static func groups(
        from rooms: [Room],
        now: Date = Date(),
        calendar: Calendar = .current
    ) -> [RoomSidebarGroup] {
        var buckets: [String: [Room]] = [:]
        for room in rooms {
            let title = groupTitle(for: room.activityDate, now: now, calendar: calendar)
            buckets[title, default: []].append(room)
        }

        let order = ["Today", "Yesterday", "This week", "Earlier"]
        return order.compactMap { title in
            guard let rooms = buckets[title], !rooms.isEmpty else { return nil }
            return RoomSidebarGroup(title: title, rooms: rooms)
        }
    }

    private static func groupTitle(for date: Date?, now: Date, calendar: Calendar) -> String {
        guard let date else { return "Earlier" }
        if calendar.isDate(date, inSameDayAs: now) { return "Today" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
           calendar.isDate(date, inSameDayAs: yesterday) {
            return "Yesterday"
        }
        if let weekAgo = calendar.date(byAdding: .day, value: -7, to: now), date >= weekAgo {
            return "This week"
        }
        return "Earlier"
    }
}

extension Room {
    var activityDate: Date? {
        (lastActivity ?? createdAt).cowchatDate
    }
}

extension String {
    var cowchatDate: Date? {
        guard !isEmpty else { return nil }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return fractional.date(from: self) ?? plain.date(from: self)
    }

    var cowchatRelativeTime: String {
        cowchatRelativeTime(relativeTo: Date())
    }

    func cowchatRelativeTime(relativeTo now: Date) -> String {
        guard let date = cowchatDate else { return "" }
        let formatter = RelativeDateTimeFormatter()
        formatter.dateTimeStyle = .numeric
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: date, relativeTo: now)
    }
}
