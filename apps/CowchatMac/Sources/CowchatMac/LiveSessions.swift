import AppKit
import SwiftUI

/// One Claude Code or Codex session running on the person's Mac, with the account room the server made for it when it
/// registered. The room's `session` field says which session it was made for, with its harness, folder and branch,
/// and `session_state` says what it is doing (navigation note, section 4.1; Chad's plan, tasks 1 and 9).
struct LiveSession: Identifiable, Equatable {
    enum Harness: String {
        case claudeCode = "claude-code"
        case codex

        // copy:
        var appName: String {
            switch self {
            case .claudeCode: return "Claude Code"
            case .codex: return "Codex"
            }
        }
    }

    /// What the session is doing. `prompt` is the value the prototype sends for a session stopped at a permission
    /// prompt while Chad decides task 9; the CLI's own `waiting` means listening, so it reads as idle.
    enum State: Equatable {
        case working
        case idle
        case prompt

        init(presence status: String?) {
            switch status?.lowercased() {
            case "working": self = .working
            case "prompt": self = .prompt
            default: self = .idle
            }
        }
    }

    /// The agent ID, which is also what a mention of the session names.
    let id: String
    let harness: Harness
    /// The folder the session works in, which is its repo.
    let folder: String
    /// The branch its worktree has checked out, or empty when the hook sent none.
    let branch: String
    let state: State
    /// When it registered.
    let startedAt: Date
    /// The account room the server made for it.
    let roomID: String
    /// The last moment the app saw it stop at a prompt, which lifts its room's cell to the top of the list.
    let waitingSince: Date?

    init(id: String, harness: Harness, folder: String, branch: String, state: State = .idle, startedAt: Date,
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
    var baseName: String {
        let last = branch.split(separator: "/").last.map(String.init) ?? branch
        return last.isEmpty || last == "main" || last == "master" ? folder : last
    }

    /// Its second fact: its folder, or its branch when the name already says the folder.
    var place: String {
        baseName == folder && !branch.isEmpty ? branch : folder
    }
}

/// A `session_state` frame: one of the person's sessions registered, changed state or ended. The server sends it to the
/// person's own account connections, since the app hears presence only for the room it has joined (navigation note,
/// section 4.1; a question for Chad beside his task 9).
struct SessionStateEvent: Equatable {
    enum Value: String {
        case working, prompt, idle, ended
    }

    let agentID: String
    let roomID: String
    let state: Value

    init(agentID: String, roomID: String, state: Value) {
        self.agentID = agentID
        self.roomID = roomID
        self.state = state
    }

    init?(payload: [String: Any]) {
        guard let agentID = payload["agent_id"] as? String, let roomID = payload["room_id"] as? String,
              let state = (payload["state"] as? String).flatMap(Value.init(rawValue:)) else { return nil }
        self.init(agentID: agentID, roomID: roomID, state: state)
    }
}

/// The person's live sessions, each with its room, read from the account rooms' `session` field and kept current by
/// `session_state`, and what the cells, the composer and the sheets make of them: the names, the pick of five, and the
/// @ mentions.
@MainActor
final class LiveSessions: ObservableObject {
    /// How many sessions a list shows before its more row.
    static let sidebarLimit = 5

    /// Every live session, newest started first.
    @Published private(set) var sessions: [LiveSession] = []
    /// Each live session's room's name, by agent ID, which is the session's one name.
    @Published private(set) var roomNames: [String: String] = [:]
    /// The person's session rooms as the account last listed them, live or ended.
    private var listed: [SocialRoom] = []
    private var viewerID: String?
    /// What `session_state` last said of each session.
    private var states: [String: SessionStateEvent.Value] = [:]
    private var waitingSince: [String: Date] = [:]

    /// Takes the account's rooms as the server listed them. A session is live while it has not ended (`hasEnded`).
    func update(from rooms: [SocialRoom], viewerID: String?) {
        self.viewerID = viewerID
        listed = rooms.filter { $0.session != nil && $0.ownerId == viewerID && viewerID != nil }
        rebuild()
    }

    /// Takes a `session_state`. Returns false when the event names a room the account has not listed yet, which asks
    /// the caller to list the rooms again so the new session's cell appears at once.
    @discardableResult
    func apply(_ event: SessionStateEvent, at now: Date = Date()) -> Bool {
        let previous = states[event.agentID]
        states[event.agentID] = event.state
        // The app can tell when a session stops at a prompt only when it knew what the session did before.
        if event.state == .prompt, let previous, previous != .prompt { waitingSince[event.agentID] = now }
        rebuild()
        return listed.contains { $0.roomId == event.roomID }
    }

    /// Sign out: every session goes with the account.
    func clear() {
        listed = []
        viewerID = nil
        states = [:]
        waitingSince = [:]
        sessions = []
        roomNames = [:]
    }

    /// A session's one name: its room's name, which the cells, the composer and the sheets show and which others write
    /// after @.
    func name(of session: LiveSession) -> String {
        roomNames[session.id] ?? session.baseName
    }

    /// The live session a room was made for, when the room is one of the person's and its session is live.
    func session(inRoom roomID: String) -> LiveSession? {
        sessions.first { $0.roomID == roomID }
    }

    /// Whether a room's session has ended. A `session_state` heard since launch is newer than the rooms list, which
    /// refreshes every 30 seconds, so it wins: a session that ended says so at once, and a resumed one is live again
    /// before its room's `ended_at` clears.
    func hasEnded(_ session: SocialRoom.Session) -> Bool {
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
    static func pick(from sessions: [LiveSession]) -> [LiveSession] {
        guard sessions.count > sidebarLimit else { return sessions }
        let prompted = sessions.filter { $0.state == .prompt }
        let rest = sessions.filter { $0.state != .prompt }.prefix(max(0, sidebarLimit - prompted.count))
        let ids = Set(prompted.map(\.id)).union(rest.map(\.id))
        return sessions.filter { ids.contains($0.id) }
    }

    // copy:
    /// The more row's words: how many more it shows, or once open, how many it hides again.
    static func moreTitle(hidden: Int, showsAll: Bool) -> String {
        guard showsAll else { return "Show \(hidden) more" }
        return hidden == 1 ? "Hide 1 session" : "Hide \(hidden) sessions"
    }

    // copy:
    /// What a session's cell says under the pointer: where its person answers it while it is stopped at a prompt, or
    /// else its place.
    func line(for session: LiveSession) -> String {
        session.state == .prompt ? "Waiting in \(session.harness.appName)" : session.place
    }

    // MARK: Mentions

    /// The agent IDs of the live sessions a text mentions by name, in the order it names them.
    func mentions(in text: String) -> [String] {
        var found: [String] = []
        for token in Self.mentionTokens(in: text) {
            guard let session = sessions.first(where: { name(of: $0).lowercased() == token.lowercased() }),
                  !found.contains(session.id) else { continue }
            found.append(session.id)
        }
        return found
    }

    /// The words after each @ that starts a mention, up to the first character a name cannot hold.
    nonisolated static func mentionTokens(in text: String) -> [String] {
        mentionRanges(in: text).map { String(text[text.index(after: $0.lowerBound)..<$0.upperBound]) }
    }

    /// Each @ that starts a mention with the name after it, up to the first character a name cannot hold and without
    /// a full stop at its end.
    nonisolated static func mentionRanges(in text: String) -> [Range<String.Index>] {
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

/// The @names a room's messages draw in the accent, as the approved design draws them: the live sessions of the room's
/// owner, and the people the app knows, by handle or by first name ("@Patrick-Mandia" or "@Patrick"). A mention of the
/// viewer also sits on a fill. Names are kept lowercased, without their @.
struct MentionMarks: Equatable {
    /// One @name to draw in the accent.
    struct Mark: Equatable {
        let range: Range<String.Index>
        let namesViewer: Bool
    }

    private(set) var names: Set<String> = []
    private(set) var viewerNames: Set<String> = []

    init() {}

    init(sessionNames: [String], people: [SocialAccount], viewer: SocialAccount?) {
        viewerNames = Set(viewer.map(Self.names(of:)) ?? [])
        names = Set(sessionNames.map { $0.lowercased() } + people.flatMap(Self.names(of:))).union(viewerNames)
    }

    /// The @names in a text that name a session or person here. With `completedOnly`, a name that runs to the end of
    /// the text, which is still being typed, is left out.
    func marks(in text: String, completedOnly: Bool = false) -> [Mark] {
        guard !names.isEmpty else { return [] }
        return LiveSessions.mentionRanges(in: text).compactMap { range in
            let name = text[text.index(after: range.lowerBound)..<range.upperBound].lowercased()
            guard names.contains(name), !completedOnly || range.upperBound < text.endIndex else { return nil }
            return Mark(range: range, namesViewer: viewerNames.contains(name))
        }
    }

    /// A message's text with its @names in the accent, and a mention of the viewer on its fill.
    func marking(_ attributed: AttributedString) -> AttributedString {
        let text = String(attributed.characters)
        let found = marks(in: text)
        guard !found.isEmpty else { return attributed }
        var result = attributed
        for mark in found {
            let start = result.characters.index(result.startIndex,
                                                offsetBy: text.distance(from: text.startIndex, to: mark.range.lowerBound))
            let end = result.characters.index(start, offsetBy: text.distance(from: mark.range.lowerBound,
                                                                             to: mark.range.upperBound))
            result[start..<end].swiftUI.foregroundColor = SemanticColor.surfaceGlassOnTextDefault
            if mark.namesViewer { result[start..<end].swiftUI.backgroundColor = Self.viewerFill }
        }
        return result
    }

    /// The fill behind a mention of the viewer: Nugget 100 in light mode and Nugget 900 in dark, as the approved design
    /// draws it. The accent keeps 4.5:1 on either.
    static let viewerFill = Color(nsColor: NSColor(name: nil) { appearance in
        HexColor.isDark(appearance) ? NSColor(Palette.nugget900) : NSColor(Palette.nugget100)
    })

    /// The accent, Surface Glass On Text Default, as AppKit draws it in the composer.
    static let accent = NSColor(name: nil) { appearance in
        guard let token = SemanticColor.allTokens.first(where: { $0.name == "surfaceGlassOnTextDefault" }) else {
            return .controlAccentColor
        }
        return HexColor.nsColor(HexColor.isDark(appearance) ? token.dark : token.light)
    }

    /// A person's handle and first name, as others write them after @.
    private static func names(of account: SocialAccount) -> [String] {
        [account.handle, account.displayName.split(separator: " ").first.map(String.init) ?? ""]
            .filter { !$0.isEmpty }
            .map { $0.lowercased() }
    }
}

private struct MentionMarksKey: EnvironmentKey {
    static let defaultValue = MentionMarks()
}

extension EnvironmentValues {
    /// The @names the messages drawn here mark in the accent.
    var mentionMarks: MentionMarks {
        get { self[MentionMarksKey.self] }
        set { self[MentionMarksKey.self] = newValue }
    }
}
