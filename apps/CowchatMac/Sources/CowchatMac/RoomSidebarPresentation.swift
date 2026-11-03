import Foundation

enum ChatPresencePresentation {
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
            let names = active.prefix(2).map(\.name).joined(separator: ", ")
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
            // copy:
            return "\(recent), \(connectedCount) connected"
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
    /// The rooms a server's list shows before a search narrows it. The local
    /// lobby is an ordinary room in the This Mac list (Patrick removed the Lobby
    /// page, 2 November). A hosted server's lobby stays hidden.
    static func listedRooms(_ rooms: [Room], on server: WorkspaceStore.Server) -> [Room] {
        server == .local
            ? rooms
            : rooms.filter { $0.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame }
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

    /// Flat iMessage-style ordering by recency, with no special case for the
    /// lobby.
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

// MARK: The sidebar's list

/// The sidebar's filter, which its chip shows and whose menu chooses it (navigation note, section 3.3). It is not kept
/// across launches, so the app always starts on All.
enum SidebarFilter: String, CaseIterable, Identifiable {
    case all
    case sessions
    case rooms
    case archived

    var id: String { rawValue }

    // copy:
    var title: String {
        switch self {
        case .all: return "All"
        case .sessions: return "Sessions"
        case .rooms: return "Rooms"
        case .archived: return "Archived"
        }
    }

    /// The values the chip's menu lists. Signed out there are no sessions, so Sessions would be empty and Rooms would
    /// equal All.
    static func offered(signedIn: Bool) -> [SidebarFilter] {
        signedIn ? allCases : [.all, .archived]
    }

    // copy:
    /// The one line an empty list shows under this value. Under All an empty list shows nothing.
    var emptyLine: String? {
        switch self {
        case .all: return nil
        case .sessions: return "No live sessions."
        case .rooms: return "No rooms."
        case .archived: return "No archived rooms."
        }
    }

    /// Whether an unarchived room belongs in the list under this value: a live session's room under Sessions, every
    /// other room under Rooms, and every room under All. Archived holds archived rooms only, which the list takes apart.
    func holds(_ room: SidebarRoom) -> Bool {
        switch self {
        case .all, .archived: return true
        case .sessions: return room.session != nil
        case .rooms: return room.session == nil
        }
    }
}

/// Where a cell says its room is. The room of one of the person's live sessions is on this Mac, since the session runs
/// there, and so is every room on this Mac's server; every other account room and every API-key room is online.
enum SidebarPlace: Equatable {
    case thisMac
    case online
}

/// What the app knows of a server's connection for its cells, once it has been down for a while
/// (`ChatStore.isConnectionDown`), so launch and a quick reconnect change no icon.
enum SidebarConnection: Equatable {
    case up
    case connecting
    case offline
    case failed

    init(status: ConnectionStatus, isDown: Bool) {
        guard isDown else { self = .up; return }
        switch status {
        case .connected: self = .up
        case .connecting: self = .connecting
        case .disconnected: self = .offline
        case .failed: self = .failed
        }
    }
}

/// The icon in a cell's place slot (navigation note, sections 3.4 and 3.11).
enum SidebarPlaceIcon: Equatable {
    case thisMac
    case online
    /// This Mac's server has failed.
    case thisMacFailed
    /// An online room's connection is connecting or disconnected.
    case onlineDown
    /// An online room's connection has failed.
    case onlineFailed

    /// The icon for a room's place on a server whose connection is as given. A live session's room is on this Mac but
    /// on the account's connection, so its icon swaps as every account room's does when that connection is down.
    static func icon(place: SidebarPlace, isLocalServer: Bool, connection: SidebarConnection) -> SidebarPlaceIcon {
        if isLocalServer { return connection == .failed ? .thisMacFailed : .thisMac }
        switch connection {
        case .up: return place == .thisMac ? .thisMac : .online
        case .connecting, .offline: return .onlineDown
        case .failed: return .onlineFailed
        }
    }

    // copy:
    /// The slot's help, which says what is wrong while a connection is down.
    func help(connection: SidebarConnection) -> String {
        switch self {
        case .thisMac: return "On this Mac"
        case .online: return "Online"
        case .thisMacFailed: return "Can't reach this Mac's server"
        case .onlineDown: return connection == .connecting ? "Connecting…" : "Offline"
        case .onlineFailed: return "Offline"
        }
    }

    /// Whether the connection the icon stands for is down, which adds Reconnect to the cell's right-click menu.
    var isDown: Bool { self == .thisMacFailed || self == .onlineDown || self == .onlineFailed }
}

/// One room as the sidebar's list holds it, on whichever of the three connections it is.
struct SidebarRoom: Identifiable, Equatable {
    let server: WorkspaceStore.Server
    /// The room as its feed shows it (`ChatStore.roomAsShown`), so its time is its newest shown message.
    let room: Room
    /// The person's live session the room was made for, while it is live.
    var session: LiveSession?
    var isArchived = false
    var isUnread = false
    /// Whether a message in the room matches the search (`ChatStore.messageSearchRoomIDs`).
    var matchesMessages = false

    var id: String { Self.id(room.id, on: server) }

    /// A room's ID in the list. Room IDs are each server's own, so the server is part of it.
    static func id(_ roomID: String, on server: WorkspaceStore.Server) -> String { "\(server.rawValue)/\(roomID)" }

    var place: SidebarPlace { server == .local || session != nil ? .thisMac : .online }

    /// The list ID of the room it sits inside, on the same server.
    var parentListID: String? { room.parentID.map { Self.id($0, on: server) } }

    /// The time the cell shows and the list orders by: the newest shown message, and for a live session's room also
    /// the session's start and the moment the app saw it stop at a prompt, so a session that starts waiting rises to
    /// the top.
    var activity: Date {
        [room.activityDate, session?.startedAt, session?.waitingSince].compactMap { $0 }.max() ?? .distantPast
    }
}

/// A room the person can join, which the search's To join group lists (navigation note, section 3.7).
struct SidebarJoinable: Identifiable, Equatable {
    enum Source: Equatable {
        /// An open invitation in the Inbox, by its item's ID. Pressing it opens the Join sheet.
        case invitation(itemID: String)
        /// A room on the API-key connection that the person is not in. Pressing it opens the room, which joins it.
        case room(Room)
    }

    let name: String
    var description: String?
    let server: WorkspaceStore.Server
    let at: Date?
    let source: Source

    var id: String {
        switch source {
        case let .invitation(itemID): return "join/invitation/\(itemID)"
        case let .room(room): return "join/" + SidebarRoom.id(room.id, on: server)
        }
    }
}

/// A company room and the rooms inside it, which move through the list as one (`add-new-company.md`, section 3).
struct SidebarBlock: Equatable {
    let parent: SidebarRoom
    var children: [SidebarRoom]
}

/// A row of the sidebar's list.
enum SidebarRow: Identifiable, Equatable {
    enum Disclosure: Equatable {
        case open
        case closed
    }

    /// A room's cell, at depth 1 inside a company room. A company room with rooms inside it has its disclosure, and
    /// `showsDot` is its unread, or a hidden room's while it is closed.
    case cell(SidebarRoom, depth: Int, disclosure: Disclosure?, showsDot: Bool)
    /// The row after a company room's five newest rooms.
    case more(parentID: String, hidden: Int, showsAll: Bool)
    /// A search group's label.
    case label(String)
    /// A room the person can join.
    case joinable(SidebarJoinable)

    var id: String {
        switch self {
        case let .cell(room, _, _, _): return room.id
        case let .more(parentID, _, _): return "more/\(parentID)"
        case let .label(title): return "label/\(title)"
        case let .joinable(item): return item.id
        }
    }
}

extension RoomSidebarPresentation {
    /// How many rooms inside a company room show before its more row, as many as `LiveSessions.sidebarLimit`.
    static let childLimit = 5

    /// Rooms newest first by `activity`, then by name.
    static func ordered(_ rooms: [SidebarRoom], activity: (SidebarRoom) -> Date = { $0.activity }) -> [SidebarRoom] {
        rooms.sorted { lhs, rhs in
            let l = activity(lhs), r = activity(rhs)
            if l != r { return l > r }
            return lhs.room.name.localizedCaseInsensitiveCompare(rhs.room.name) == .orderedAscending
        }
    }

    /// Each company room with the rooms inside it on the same server, as blocks in the list's order. A block's place is
    /// its newest room's, so an active child lifts its parent, and inside a block the children follow their own
    /// recency. A room whose parent has no cell in the list stands at the top level. There is one level, so a room
    /// inside a child sits under the top parent.
    static func blocks(_ rooms: [SidebarRoom],
                       activity: (SidebarRoom) -> Date = { $0.activity }) -> [SidebarBlock] {
        let byID = Dictionary(rooms.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        func top(of room: SidebarRoom) -> String {
            var current = room
            var seen: Set<String> = [room.id]
            while let parentID = current.parentListID, let parent = byID[parentID], seen.insert(parentID).inserted {
                current = parent
            }
            return current.id
        }
        var tops: [SidebarRoom] = []
        var children: [String: [SidebarRoom]] = [:]
        for room in rooms {
            let topID = top(of: room)
            if topID == room.id { tops.append(room) } else { children[topID, default: []].append(room) }
        }
        func latest(_ block: SidebarBlock) -> Date {
            ([block.parent] + block.children).map(activity).max() ?? .distantPast
        }
        return tops
            .map { SidebarBlock(parent: $0, children: ordered(children[$0.id] ?? [], activity: activity)) }
            .sorted { lhs, rhs in
                let l = latest(lhs), r = latest(rhs)
                if l != r { return l > r }
                return lhs.parent.room.name.localizedCaseInsensitiveCompare(rhs.parent.room.name) == .orderedAscending
            }
    }

    /// The blocks as rows. A closed parent hides its rooms and shows the dot when one of them is unread. A parent with
    /// more than five rooms shows the five newest and then its more row, unless `showsAllChildren` holds it. A search
    /// ignores closed parents and shows every match, so its parents draw no disclosure and no more row.
    static func rows(_ blocks: [SidebarBlock], collapsed: Set<String>, showsAllChildren: Set<String>,
                     isSearching: Bool = false) -> [SidebarRow] {
        var rows: [SidebarRow] = []
        for block in blocks {
            let hasChildren = !block.children.isEmpty
            let isOpen = isSearching || !collapsed.contains(block.parent.id)
            let disclosure: SidebarRow.Disclosure? = hasChildren && !isSearching ? (isOpen ? .open : .closed) : nil
            let hiddenUnread = !isOpen && block.children.contains(where: \.isUnread)
            rows.append(.cell(block.parent, depth: 0, disclosure: disclosure,
                              showsDot: block.parent.isUnread || hiddenUnread))
            guard hasChildren, isOpen else { continue }
            let limited = !isSearching && block.children.count > childLimit
            let showsAll = showsAllChildren.contains(block.parent.id)
            let shown = limited && !showsAll ? Array(block.children.prefix(childLimit)) : block.children
            rows += shown.map { .cell($0, depth: 1, disclosure: nil, showsDot: $0.isUnread) }
            if limited {
                rows.append(.more(parentID: block.parent.id, hidden: block.children.count - childLimit,
                                  showsAll: showsAll))
            }
        }
        return rows
    }

    /// The list under the line with no search typed: the filter's rooms in blocks, or under Archived the archived rooms
    /// alone, flat, with no parents grouped.
    static func rows(for filter: SidebarFilter, rooms: [SidebarRoom], collapsed: Set<String>,
                     showsAllChildren: Set<String>,
                     activity: (SidebarRoom) -> Date = { $0.activity }) -> [SidebarRow] {
        if filter == .archived {
            return ordered(rooms.filter(\.isArchived), activity: activity).map(flatCell)
        }
        let listed = rooms.filter { !$0.isArchived && filter.holds($0) }
        return rows(blocks(listed, activity: activity), collapsed: collapsed, showsAllChildren: showsAllChildren)
    }

    /// Whether a room matches the search: its name or description, a live session's folder or branch, or a message.
    static func matches(_ room: SidebarRoom, query: String) -> Bool {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return true }
        return room.room.name.localizedCaseInsensitiveContains(query)
            || (room.room.description?.localizedCaseInsensitiveContains(query) ?? false)
            || (room.session.map {
                $0.folder.localizedCaseInsensitiveContains(query) || $0.branch.localizedCaseInsensitiveContains(query)
            } ?? false)
            || room.matchesMessages
    }

    /// The search's groups under the line (navigation note, section 3.7). First the rooms the person is in that match,
    /// in the list's order with no label, each matching child under its parent; then "To join"; then "Archived". The
    /// filter narrows each group: Sessions shows only the first group's live sessions' rooms, Rooms shows all three
    /// without them, and Archived shows only the archived rooms, without their label. An empty group is not drawn.
    static func searchRows(query: String, filter: SidebarFilter, rooms: [SidebarRoom], joinable: [SidebarJoinable],
                           activity: (SidebarRoom) -> Date = { $0.activity }) -> [SidebarRow] {
        var rows: [SidebarRow] = []
        if filter != .archived {
            let listed = rooms.filter { !$0.isArchived && filter.holds($0) }
            let byID = Dictionary(listed.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            let matching = listed.filter { matches($0, query: query) }
            // A matching child shows under its parent, which shows with it even when it does not match.
            var shown = Set(matching.map(\.id))
            for room in matching {
                var parentID = room.parentListID
                while let id = parentID, let parent = byID[id], shown.insert(id).inserted { parentID = parent.parentListID }
            }
            rows += Self.rows(blocks(listed.filter { shown.contains($0.id) }, activity: activity), collapsed: [],
                              showsAllChildren: [], isSearching: true)
        }
        if filter == .all || filter == .rooms {
            let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
            let join = joinable
                .filter {
                    $0.name.localizedCaseInsensitiveContains(query)
                        || ($0.description?.localizedCaseInsensitiveContains(query) ?? false)
                }
                .sorted { ($0.at ?? .distantPast, $1.name) > ($1.at ?? .distantPast, $0.name) }
            if !join.isEmpty {
                // copy:
                rows.append(.label("To join"))
                rows += join.map(SidebarRow.joinable)
            }
        }
        if filter != .sessions {
            let archived = rooms.filter { $0.isArchived && (filter != .rooms || $0.session == nil) }
                .filter { matches($0, query: query) }
            if !archived.isEmpty {
                // copy:
                if filter != .archived { rows.append(.label("Archived")) }
                rows += ordered(archived, activity: activity).map(flatCell)
            }
        }
        return rows
    }

    // copy:
    /// The one line an empty list shows: while searching, "Searching…" until the message search answers and then
    /// "Nothing matches."; otherwise the filter's line, or nothing under All.
    static func emptyLine(filter: SidebarFilter, isSearching: Bool, isSearchingMessages: Bool) -> String? {
        if isSearching { return isSearchingMessages ? "Searching…" : "Nothing matches." }
        return filter.emptyLine
    }

    private static func flatCell(_ room: SidebarRoom) -> SidebarRow {
        .cell(room, depth: 0, disclosure: nil, showsDot: room.isUnread)
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

    /// The cell's short time for a timestamp from the server (`Date.cowchatShortTime(relativeTo:calendar:)`).
    func cowchatShortTime(relativeTo now: Date, calendar: Calendar = .current) -> String {
        cowchatDate?.cowchatShortTime(relativeTo: now, calendar: calendar) ?? ""
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

extension Date {
    // copy:
    /// The sidebar cell's time (navigation note, section 3.4): "now" under a minute, "9m" under an hour, "3h" under a
    /// day, "2d" under a week, then the day and month ("Sep 3"). A time at or after `now` reads "now".
    func cowchatShortTime(relativeTo now: Date, calendar: Calendar = .current) -> String {
        let seconds = now.timeIntervalSince(self)
        if seconds < 60 { return "now" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        if seconds < 86_400 { return "\(Int(seconds / 3600))h" }
        if seconds < 7 * 86_400 { return "\(Int(seconds / 86_400))d" }
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "MMM d"
        return formatter.string(from: self)
    }
}
