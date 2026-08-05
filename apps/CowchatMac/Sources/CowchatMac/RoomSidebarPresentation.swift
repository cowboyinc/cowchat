import Foundation

struct RoomSidebarGroup: Equatable {
    let title: String
    let rooms: [Room]
}

enum LobbyPresentation {
    static func availableAgentCount(
        from members: [AgentPresence],
        excluding currentAgentID: String
    ) -> Int {
        Set(members.lazy.filter { $0.id != currentAgentID }.map(\.id)).count
    }
}

enum ChatPresencePresentation {
    static func summary(
        members: [AgentPresence],
        currentAgentID: String,
        fallbackMemberCount: Int?,
        isConnected: Bool
    ) -> String {
        let collaborators = members.filter { $0.id != currentAgentID }
        let active = collaborators.filter {
            guard let status = $0.status?.lowercased() else { return false }
            return status == "working" || status == "thinking"
        }
        if !active.isEmpty {
            let names = active.prefix(2).map(\.name).joined(separator: " · ")
            return "\(names) active"
        }

        let count: Int
        if !members.isEmpty {
            count = Set(collaborators.map(\.id)).count
        } else {
            count = max((fallbackMemberCount ?? 0) - (isConnected ? 1 : 0), 0)
        }
        switch count {
        case 0: return "No collaborators"
        case 1: return "1 collaborator"
        default: return "\(count) collaborators"
        }
    }
}

enum RoomSidebarPresentation {
    static func pinnedRooms(
        from rooms: [Room],
        pinnedRoomIDs: Set<String>,
        limit: Int = 3
    ) -> [Room] {
        Array(rooms.filter { pinnedRoomIDs.contains($0.id) }.prefix(limit))
    }

    static func activeRooms(
        from rooms: [Room],
        excludingCurrentClientFrom selectedRoomID: String? = nil
    ) -> [Room] {
        rooms.filter { room in
            let currentClientCount = room.id == selectedRoomID ? 1 : 0
            return (room.memberCount ?? 0) - currentClientCount > 0
        }
    }

    static func filteredRooms(
        from rooms: [Room],
        query: String,
        matchingMessageRoomIDs: Set<String> = []
    ) -> [Room] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return rooms }
        return rooms.filter {
            $0.name.localizedCaseInsensitiveContains(query)
                || ($0.description?.localizedCaseInsensitiveContains(query) ?? false)
                || matchingMessageRoomIDs.contains($0.id)
        }
    }

    static func visiblePinnedRooms(
        from allRooms: [Room],
        among visibleRooms: [Room],
        pinnedRoomIDs: Set<String>,
        limit: Int = 3
    ) -> [Room] {
        let pinnedIDs = Set(
            pinnedRooms(
                from: allRooms,
                pinnedRoomIDs: pinnedRoomIDs,
                limit: limit
            ).map(\.id)
        )
        return visibleRooms.filter { pinnedIDs.contains($0.id) }
    }

    static func roomsForRecencyGroups(
        from visibleRooms: [Room],
        allRooms: [Room],
        pinnedRoomIDs: Set<String>,
        pinnedLimit: Int = 3
    ) -> [Room] {
        let displayedPinnedIDs = Set(
            visiblePinnedRooms(
                from: allRooms,
                among: visibleRooms,
                pinnedRoomIDs: pinnedRoomIDs,
                limit: pinnedLimit
            ).map(\.id)
        )
        return visibleRooms.filter { !displayedPinnedIDs.contains($0.id) }
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
