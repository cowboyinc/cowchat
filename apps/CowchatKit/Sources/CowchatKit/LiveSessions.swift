import Foundation
import SwiftUI

/// One Claude Code or Codex session running on the person's Mac, with the account room the server made for it when it
/// registered. The room's `session` field says which session it was made for, with its harness, folder and branch,
/// and `session_state` says what it is doing (navigation note, section 4.1; Chad's plan, tasks 1 and 9).
public struct LiveSession: Identifiable, Equatable {
    public enum Harness: String {
        case claudeCode = "claude-code"
        case codex

        // copy:
        public var appName: String {
            switch self {
            case .claudeCode: return "Claude Code"
            case .codex: return "Codex"
            }
        }
    }

    /// What the session is doing. `prompt` is the value the prototype sends for a session stopped at a permission
    /// prompt while Chad decides task 9; the CLI's own `waiting` means listening, so it reads as idle.
    public enum State: Equatable {
        case working
        case idle
        case prompt

        public init(presence status: String?) {
            switch status?.lowercased() {
            case "working": self = .working
            case "prompt": self = .prompt
            default: self = .idle
            }
        }
    }

    /// The agent ID, which is also what a mention of the session names.
    public let id: String
    public let harness: Harness
    /// The folder the session works in, which is its repo.
    public let folder: String
    /// The branch its worktree has checked out, or empty when the hook sent none.
    public let branch: String
    public let state: State
    /// When it registered.
    public let startedAt: Date
    /// The account room the server made for it.
    public let roomID: String
    /// The last moment the app saw it stop at a prompt, which lifts its room's cell to the top of the list.
    public let waitingSince: Date?

    public init(id: String, harness: Harness, folder: String, branch: String, state: State = .idle, startedAt: Date,
         roomID: String = "", waitingSince: Date? = nil) {
        self.id = id
        self.harness = harness
        self.folder = folder
        self.branch = branch
        self.state = state
        self.startedAt = startedAt
        self.roomID = roomID
        self.waitingSince = waitingSince
    }

    /// The name before any number: its branch without the person's prefix, or its folder on main, as the plugin names
    /// a session after its folder and branch.
    public var baseName: String {
        let last = branch.split(separator: "/").last.map(String.init) ?? branch
        return last.isEmpty || last == "main" || last == "master" ? folder : last
    }

    /// Its second fact: its folder, or its branch when the name already says the folder.
    public var place: String {
        baseName == folder && !branch.isEmpty ? branch : folder
    }
}

/// A `session_state` frame: one of the person's sessions registered, changed state or ended. The server sends it to the
/// person's own account connections, since the app hears presence only for the room it has joined (navigation note,
/// section 4.1; a question for Chad beside his task 9).
public struct SessionStateEvent: Equatable {
    public enum Value: String {
        case working, prompt, idle, ended
    }

    public let agentID: String
    public let roomID: String
    public let state: Value

    public init(agentID: String, roomID: String, state: Value) {
        self.agentID = agentID
        self.roomID = roomID
        self.state = state
    }

    public init?(payload: [String: Any]) {
        guard let agentID = payload["agent_id"] as? String, let roomID = payload["room_id"] as? String,
              let state = (payload["state"] as? String).flatMap(Value.init(rawValue:)) else { return nil }
        self.init(agentID: agentID, roomID: roomID, state: state)
    }
}

/// The person's live sessions, each with its room, read from the account rooms' `session` field and kept current by
/// `session_state`, and what the cells, the composer and the sheets make of them: the names, the pick of five, and the
/// @ mentions.
@MainActor
public final class LiveSessions: ObservableObject {
    /// How many sessions a list shows before its more row.
    public static let sidebarLimit = 5

    public init() {}

    /// Every live session, newest started first.
    @Published public private(set) var sessions: [LiveSession] = []
    /// Each live session's room's name, by agent ID, which is the session's one name.
    @Published public private(set) var roomNames: [String: String] = [:]
    /// The person's session rooms as the account last listed them, live or ended.
    private var listed: [SocialRoom] = []
    private var viewerID: String?
    /// What `session_state` last said of each session.
    private var states: [String: SessionStateEvent.Value] = [:]
    private var waitingSince: [String: Date] = [:]

    /// Takes the account's rooms as the server listed them. A session is live while it has not ended (`hasEnded`).
    public func update(from rooms: [SocialRoom], viewerID: String?) {
        self.viewerID = viewerID
        listed = rooms.filter { $0.session != nil && $0.ownerId == viewerID && viewerID != nil }
        rebuild()
    }

    /// Takes a `session_state`. Returns false when the event names a room the account has not listed yet, which asks
    /// the caller to list the rooms again so the new session's cell appears at once.
    @discardableResult
    public func apply(_ event: SessionStateEvent, at now: Date = Date()) -> Bool {
        let previous = states[event.agentID]
        states[event.agentID] = event.state
        // The app can tell when a session stops at a prompt only when it knew what the session did before.
        if event.state == .prompt, let previous, previous != .prompt { waitingSince[event.agentID] = now }
        rebuild()
        return listed.contains { $0.roomId == event.roomID }
    }

    /// Sign out: every session goes with the account.
    public func clear() {
        listed = []
        viewerID = nil
        states = [:]
        waitingSince = [:]
        sessions = []
        roomNames = [:]
    }

    /// A session's one name: its room's name, which the cells, the composer and the sheets show and which others write
    /// after @.
    public func name(of session: LiveSession) -> String {
        roomNames[session.id] ?? session.baseName
    }

    /// The live session a room was made for, when the room is one of the person's and its session is live.
    public func session(inRoom roomID: String) -> LiveSession? {
        sessions.first { $0.roomID == roomID }
    }

    /// Whether a room's session has ended. A `session_state` heard since launch is newer than the rooms list, which
    /// refreshes every 30 seconds, so it wins: a session that ended says so at once, and a resumed one is live again
    /// before its room's `ended_at` clears.
    public func hasEnded(_ session: SocialRoom.Session) -> Bool {
        if let state = states[session.agentId] { return state == .ended }
        return session.endedAt != nil
    }

    private func rebuild() {
        var live: [LiveSession] = []
        var names: [String: String] = [:]
        for room in listed {
            guard let session = room.session, !hasEnded(session),
                  let harness = LiveSession.Harness(rawValue: session.harness) else { continue }
            let state: LiveSession.State
            switch states[session.agentId] {
            case .working: state = .working
            case .prompt: state = .prompt
            default: state = .idle
            }
            live.append(LiveSession(id: session.agentId, harness: harness, folder: session.folder,
                                    branch: session.branch, state: state,
                                    startedAt: session.startedAt.cowchatDate ?? .distantPast, roomID: room.roomId,
                                    waitingSince: waitingSince[session.agentId]))
            names[session.agentId] = room.name
        }
        live.sort { ($0.startedAt, $0.id) > ($1.startedAt, $1.id) }
        if names != roomNames { roomNames = names }
        if live != sessions { sessions = live }
    }

    // MARK: Lists

    /// The rows a list shows while its more row is closed: every session stopped at a prompt, since its person must
    /// answer it, then the most recently started of the rest, up to five rows in all, kept in their order so a row
    /// never jumps when it is picked. With five or fewer, all of them. More than five at a prompt all show. The Join
    /// sheet and the Team page use it.
    public static func pick(from sessions: [LiveSession]) -> [LiveSession] {
        guard sessions.count > sidebarLimit else { return sessions }
        let prompted = sessions.filter { $0.state == .prompt }
        let rest = sessions.filter { $0.state != .prompt }.prefix(max(0, sidebarLimit - prompted.count))
        let ids = Set(prompted.map(\.id)).union(rest.map(\.id))
        return sessions.filter { ids.contains($0.id) }
    }

    // copy:
    /// The more row's words: how many more it shows, or once open, how many it hides again.
    public static func moreTitle(hidden: Int, showsAll: Bool) -> String {
        guard showsAll else { return "Show \(hidden) more" }
        return hidden == 1 ? "Hide 1 session" : "Hide \(hidden) sessions"
    }

    // copy:
    /// What a session's cell says under the pointer: where its person answers it while it is stopped at a prompt, or
    /// else its place.
    public func line(for session: LiveSession) -> String {
        session.state == .prompt ? "Waiting in \(session.harness.appName)" : session.place
    }

    // MARK: Mentions

    /// The agent IDs of the live sessions a text mentions by name, in the order it names them.
    public func mentions(in text: String) -> [String] {
        var found: [String] = []
        for token in Self.mentionTokens(in: text) {
            guard let session = sessions.first(where: { name(of: $0).lowercased() == token.lowercased() }),
                  !found.contains(session.id) else { continue }
            found.append(session.id)
        }
        return found
    }

    /// The words after each @ that starts a mention, up to the first character a name cannot hold.
    public nonisolated static func mentionTokens(in text: String) -> [String] {
        mentionRanges(in: text).map { String(text[text.index(after: $0.lowerBound)..<$0.upperBound]) }
    }

    /// Each @ that starts a mention with the name after it, up to the first character a name cannot hold and without
    /// a full stop at its end.
    public nonisolated static func mentionRanges(in text: String) -> [Range<String.Index>] {
        var ranges: [Range<String.Index>] = []
        var index = text.startIndex
        while let at = text[index...].firstIndex(of: "@") {
            let start = text.index(after: at)
            let end = text[start...].firstIndex { !isNameCharacter($0) } ?? text.endIndex
            var nameEnd = end
            while nameEnd > start, text[text.index(before: nameEnd)] == "." { nameEnd = text.index(before: nameEnd) }
            if nameEnd > start, startsMention(text, at: at) { ranges.append(at..<nameEnd) }
            index = end
        }
        return ranges
    }

    /// Whether the @ at this index starts a mention: it begins the text or follows a space, as in "@Chad" but not in
    /// "foo@unread-counts".
    private nonisolated static func startsMention(_ text: String, at: String.Index) -> Bool {
        at == text.startIndex || text[text.index(before: at)].isWhitespace
    }

    private nonisolated static func isNameCharacter(_ character: Character) -> Bool {
        character.isLetter || character.isNumber || "-_.".contains(character)
    }
}
