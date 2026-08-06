import Foundation

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

    /// Flat iMessage-style ordering. Lobby stays first — the existing roomSort
    /// invariant (ChatStore.roomSort): it is the home surface, not a "pin".
    static func sortedByRecency(_ rooms: [Room]) -> [Room] {
        rooms.sorted { lhs, rhs in
            let lhsIsLobby = lhs.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame
            let rhsIsLobby = rhs.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame
            if lhsIsLobby != rhsIsLobby { return lhsIsLobby }
            let l = lhs.activityDate ?? .distantPast
            let r = rhs.activityDate ?? .distantPast
            if l != r { return l > r }
            return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
        }
    }

    /// Sidebar working signal: presence is selected-room-only and unattributable
    /// per-room (presence_update has no room_id), so background rooms light up on
    /// thinking-message recency instead — see the spec's §4 validation notes.
    /// Tracked per agent (not per room) so one agent finishing a turn cannot
    /// clear the indicator while another agent in the same room is still
    /// composing.
    static func isWorking(thinkingByAgent: [String: Date]?, now: Date, window: TimeInterval = 120) -> Bool {
        guard let thinkingByAgent else { return false }
        return thinkingByAgent.values.contains { now.timeIntervalSince($0) < window }
    }

    /// Per-agent thinking tracking: a thinking message stamps that agent's entry;
    /// a non-thinking message clears ONLY that agent's entry (another agent may
    /// still be composing). Empty room maps are pruned so `isEmpty` stays a
    /// cheap "anything working?" check.
    static func updatedThinkingByAgent(
        _ current: [String: [String: Date]],
        message: ChatMessage,
        now: Date
    ) -> [String: [String: Date]] {
        var updated = current
        if message.isThinking {
            updated[message.roomID, default: [:]][message.agentID] = message.timestamp.cowchatDate ?? now
        } else {
            updated[message.roomID]?.removeValue(forKey: message.agentID)
            if updated[message.roomID]?.isEmpty == true {
                updated.removeValue(forKey: message.roomID)
            }
        }
        return updated
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
