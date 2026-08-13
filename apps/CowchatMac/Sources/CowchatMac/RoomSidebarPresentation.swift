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
    struct Agent: Identifiable, Equatable {
        enum Activity: Equatable {
            case active
            case recent
            case connected

            var label: String {
                switch self {
                case .active: return "Active now"
                case .recent: return "Active recently"
                case .connected: return "Connected"
                }
            }

            fileprivate var sortOrder: Int {
                switch self {
                case .active: return 0
                case .recent: return 1
                case .connected: return 2
                }
            }
        }

        let id: String
        let name: String
        let activity: Activity
        let statusDetail: String?
        let progress: Int?

        var statusText: String {
            var parts = [activity.label]
            if let statusDetail = statusDetail?.trimmingCharacters(in: .whitespacesAndNewlines),
               !statusDetail.isEmpty {
                parts.append(statusDetail)
            }
            if let progress {
                parts.append("\(min(max(progress, 0), 100))%")
            }
            return parts.joined(separator: " · ")
        }
    }

    static func agents(
        members: [AgentPresence],
        currentAgentID: String,
        recentActivityByAgent: [String: Date]? = nil,
        recentAgentNames: [String: String]? = nil,
        now: Date = Date(),
        recentWindow: TimeInterval = 120,
        connectionStatus: ConnectionStatus = .connected
    ) -> [Agent] {
        guard connectionStatus.isConnected else { return [] }

        let recentActivityByAgent = recentActivityByAgent ?? [:]
        let recentAgentNames = recentAgentNames ?? [:]
        var agentsByID: [String: Agent] = [:]

        for member in members where member.id != currentAgentID {
            let status = member.status?.lowercased()
            let isActive = status == "working" || status == "thinking"
            let isRecent = recentActivityByAgent[member.id].map {
                now.timeIntervalSince($0) < recentWindow
            } ?? false
            let activity: Agent.Activity = isActive ? .active : (isRecent ? .recent : .connected)
            let preferredName = member.name.trimmingCharacters(in: .whitespacesAndNewlines)
            agentsByID[member.id] = Agent(
                id: member.id,
                name: preferredName.isEmpty ? (recentAgentNames[member.id] ?? member.id) : preferredName,
                activity: activity,
                statusDetail: member.statusDetail,
                progress: member.progress
            )
        }

        for (agentID, timestamp) in recentActivityByAgent
        where agentID != currentAgentID
            && now.timeIntervalSince(timestamp) < recentWindow
            && agentsByID[agentID] == nil {
            let knownName = recentAgentNames[agentID]?.trimmingCharacters(in: .whitespacesAndNewlines)
            let displayName = knownName.flatMap { $0.isEmpty ? nil : $0 } ?? agentID
            agentsByID[agentID] = Agent(
                id: agentID,
                name: displayName,
                activity: .recent,
                statusDetail: nil,
                progress: nil
            )
        }

        return agentsByID.values.sorted { lhs, rhs in
            if lhs.activity.sortOrder != rhs.activity.sortOrder {
                return lhs.activity.sortOrder < rhs.activity.sortOrder
            }
            return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
        }
    }

    static func summary(
        members: [AgentPresence],
        currentAgentID: String,
        fallbackMemberCount: Int?,
        fallbackMemberCountIncludesCurrentAgent: Bool = false,
        recentActivityByAgent: [String: Date]? = nil,
        now: Date = Date(),
        recentWindow: TimeInterval = 120,
        connectionStatus: ConnectionStatus = .connected
    ) -> String {
        switch connectionStatus {
        case .connecting:
            return "Connecting…"
        case .disconnected, .failed:
            return "Offline"
        case .connected:
            break
        }

        let collaborators = members.filter { $0.id != currentAgentID }
        let active = collaborators.filter {
            guard let status = $0.status?.lowercased() else { return false }
            return status == "working" || status == "thinking"
        }
        if !active.isEmpty {
            let names = active.prefix(2).map(\.name).joined(separator: " · ")
            return "\(names) active"
        }

        let recentCount = recentCollaboratorCount(
            recentActivityByAgent: recentActivityByAgent,
            currentAgentID: currentAgentID,
            now: now,
            recentWindow: recentWindow
        )
        let connectedCount = connectedCollaboratorCount(
            members: members,
            currentAgentID: currentAgentID,
            fallbackMemberCount: fallbackMemberCount,
            fallbackMemberCountIncludesCurrentAgent: fallbackMemberCountIncludesCurrentAgent
        )
        if recentCount > 0 {
            let recent = recentCount == 1
                ? "1 agent active recently"
                : "\(recentCount) agents active recently"
            guard connectedCount > 0 else { return recent }
            return "\(recent) · \(connectedSummary(count: connectedCount))"
        }

        return connectedSummary(count: connectedCount)
    }

    static func hasCollaboratorSignal(
        members: [AgentPresence],
        currentAgentID: String,
        fallbackMemberCount: Int?,
        fallbackMemberCountIncludesCurrentAgent: Bool = false,
        recentActivityByAgent: [String: Date]? = nil,
        now: Date = Date(),
        recentWindow: TimeInterval = 120
    ) -> Bool {
        connectedCollaboratorCount(
            members: members,
            currentAgentID: currentAgentID,
            fallbackMemberCount: fallbackMemberCount,
            fallbackMemberCountIncludesCurrentAgent: fallbackMemberCountIncludesCurrentAgent
        ) > 0 || recentCollaboratorCount(
            recentActivityByAgent: recentActivityByAgent,
            currentAgentID: currentAgentID,
            now: now,
            recentWindow: recentWindow
        ) > 0
    }

    private static func connectedCollaboratorCount(
        members: [AgentPresence],
        currentAgentID: String,
        fallbackMemberCount: Int?,
        fallbackMemberCountIncludesCurrentAgent: Bool
    ) -> Int {
        let observedCount = Set(members.lazy.filter { $0.id != currentAgentID }.map(\.id)).count
        let currentAgentIsIncluded = fallbackMemberCountIncludesCurrentAgent
            || members.contains { $0.id == currentAgentID }
        let fallbackCount = max(
            (fallbackMemberCount ?? 0) - (currentAgentIsIncluded ? 1 : 0),
            0
        )
        return max(observedCount, fallbackCount)
    }

    private static func recentCollaboratorCount(
        recentActivityByAgent: [String: Date]?,
        currentAgentID: String,
        now: Date,
        recentWindow: TimeInterval
    ) -> Int {
        Set<String>(
            (recentActivityByAgent ?? [:]).compactMap { agentID, timestamp in
                guard agentID != currentAgentID,
                      now.timeIntervalSince(timestamp) < recentWindow else { return nil }
                return agentID
            }
        ).count
    }

    private static func connectedSummary(count: Int) -> String {
        switch count {
        case 0: return "No agents connected"
        case 1: return "1 agent connected"
        default: return "\(count) agents connected"
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

    /// Flat iMessage-style ordering by recency. Lobby lives in its own nav
    /// row above the table (Patrick, 2026-08-06), so no special-casing here.
    static func sortedByRecency(_ rooms: [Room]) -> [Room] {
        rooms.sorted { lhs, rhs in
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
        // Opportunistic prune: entries far past the working window would
        // otherwise linger forever (an agent that pulses once and vanishes),
        // pinning the sidebar's fast refresh cadence indefinitely.
        for (roomID, agents) in updated {
            let live = agents.filter { now.timeIntervalSince($0.value) < 600 }
            if live.isEmpty {
                updated.removeValue(forKey: roomID)
            } else if live.count != agents.count {
                updated[roomID] = live
            }
        }
        if message.isThinking {
            // Live working state is about when this client observed the pulse.
            // Agent clocks can be skewed far enough that the message timestamp
            // would otherwise make a just-received pulse look expired.
            updated[message.roomID, default: [:]][message.agentID] = now
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
        // Callers pass a TimelineView tick date, captured when the view was
        // last evaluated, so an event that just arrived can equal or exceed
        // it. RelativeDateTimeFormatter renders any delta that rounds to zero
        // as "in 0s" — a future reading for something that just happened.
        guard now.timeIntervalSince(date) >= 1 else { return "now" }
        let formatter = RelativeDateTimeFormatter()
        formatter.dateTimeStyle = .numeric
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: date, relativeTo: now)
    }
}
