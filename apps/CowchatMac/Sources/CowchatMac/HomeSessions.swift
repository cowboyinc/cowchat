import AppKit
import SwiftUI

/// One Claude Code or Codex session running on the person's Mac, as the server lists it among the agents in their
/// Home. Its hook registers it under the person's bot as `<bot id>.<session>` and puts `harness:`, `folder:` and
/// `branch:` in its capabilities; its presence status is what its hooks last reported. (Chad's plan, tasks 1 and 9.)
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
    /// When it registered, which orders Home newest first.
    let startedAt: Date

    /// A session from one of Home's agents, or nil for an agent that is not a session, such as the person's Mac.
    init?(agent: AgentPresence) {
        var fields: [String: String] = [:]
        for capability in agent.capabilities {
            guard let colon = capability.firstIndex(of: ":") else { continue }
            let key = String(capability[..<colon])
            if fields[key] == nil { fields[key] = String(capability[capability.index(after: colon)...]) }
        }
        guard let harness = fields["harness"].flatMap(Harness.init(rawValue:)),
              let folder = fields["folder"], !folder.isEmpty else { return nil }
        id = agent.agentID
        self.harness = harness
        self.folder = folder
        branch = fields["branch"] ?? ""
        state = State(presence: agent.status)
        startedAt = agent.connectedAt?.cowchatDate ?? .distantPast
    }

    init(id: String, harness: Harness, folder: String, branch: String, state: State = .idle, startedAt: Date) {
        self.id = id
        self.harness = harness
        self.folder = folder
        self.branch = branch
        self.state = state
        self.startedAt = startedAt
    }

    /// The name before any number: its branch without the person's prefix, or its folder on main, as the plugin names
    /// a session after its folder and branch.
    var baseName: String {
        let last = branch.split(separator: "/").last.map(String.init) ?? branch
        return last.isEmpty || last == "main" || last == "master" ? folder : last
    }

    /// Its second fact in a row: its folder, or its branch when the name already says the folder.
    var place: String {
        baseName == folder && !branch.isEmpty ? branch : folder
    }
}

/// The person's live sessions, read from their Home's agents, and what the sidebar and composer make of them: the
/// names, the five rows under Home, the row for the rest, and the @ completion.
@MainActor
final class HomeSessions: ObservableObject {
    /// How many of Home's sessions the sidebar shows before its more row.
    static let sidebarLimit = 5

    /// The account's Home room, from its setup.
    @Published var roomID: String?
    /// Every live session, newest started first, as Home lists them.
    @Published private(set) var sessions: [LiveSession] = []
    /// Whether the more row is open, showing every session.
    @Published var showsAll = false
    /// Each session's name, settled when it was first seen.
    @Published private(set) var names: [String: String] = [:]

    /// Takes Home's agents as the server listed them.
    func update(from agents: [AgentPresence]) {
        let live = agents.compactMap(LiveSession.init(agent:))
            .sorted { ($0.startedAt, $0.id) > ($1.startedAt, $1.id) }
        let settled = Self.settledNames(for: live, keeping: names)
        if settled != names { names = settled }
        if live != sessions { sessions = live }
    }

    func clear() {
        sessions = []
        names = [:]
        showsAll = false
    }

    /// A session's one name, which its row and the @ completion show and which others write after @.
    func name(of session: LiveSession) -> String {
        names[session.id] ?? session.baseName
    }

    /// Names new sessions in the order they started and keeps every name already given, so a name never changes while
    /// people use it. A session takes its base name, or when a live one holds it, the lowest free number after it
    /// ("unread-counts-2"). An ended session frees its name for the next.
    nonisolated static func settledNames(for sessions: [LiveSession], keeping previous: [String: String])
        -> [String: String] {
        let live = Set(sessions.map(\.id))
        var names = previous.filter { live.contains($0.key) }
        for session in sessions.sorted(by: { ($0.startedAt, $0.id) < ($1.startedAt, $1.id) })
        where names[session.id] == nil {
            let taken = Set(names.values)
            let base = session.baseName
            var number = 1
            while taken.contains(number == 1 ? base : "\(base)-\(number)") { number += 1 }
            names[session.id] = number == 1 ? base : "\(base)-\(number)"
        }
        return names
    }

    // MARK: The sidebar

    /// The rows under Home while the more row is closed: every session stopped at a prompt, since its person must
    /// answer it there, then the most recently started of the rest, up to five rows in all, in Home's order so a row
    /// never jumps when it is picked. With five or fewer live, all of them. More than five at a prompt all show.
    var picked: [LiveSession] { Self.pick(from: sessions) }

    /// The pick from `sessions`, kept in their order. The Join sheet's rows use it too.
    static func pick(from sessions: [LiveSession]) -> [LiveSession] {
        guard sessions.count > sidebarLimit else { return sessions }
        let prompted = sessions.filter { $0.state == .prompt }
        let rest = sessions.filter { $0.state != .prompt }.prefix(max(0, sidebarLimit - prompted.count))
        let ids = Set(prompted.map(\.id)).union(rest.map(\.id))
        return sessions.filter { ids.contains($0.id) }
    }

    /// The rows under Home: the pick, or every session once the more row is pressed.
    var shown: [LiveSession] { showsAll ? sessions : picked }

    /// How many sessions the pick leaves out. The more row shows only when it is more than none.
    var hiddenCount: Int { sessions.count - picked.count }

    var moreTitle: String { Self.moreTitle(hidden: hiddenCount, showsAll: showsAll) }

    // copy:
    /// The more row's words: how many more it shows, or once open, how many it hides again.
    static func moreTitle(hidden: Int, showsAll: Bool) -> String {
        guard showsAll else { return "Show \(hidden) more" }
        return hidden == 1 ? "Hide 1 session" : "Hide \(hidden) sessions"
    }

    // copy:
    /// A session's second line: where its person answers it while it is stopped at a prompt, or else its place.
    func line(for session: LiveSession) -> String {
        session.state == .prompt ? "Waiting in \(session.harness.appName)" : session.place
    }

    /// How Home's header names an agent at work: a live session by its name alone, since every session in Home is
    /// its person's, and any other agent as the server names it.
    func headerName(of agent: AgentPresence) -> String {
        sessions.first { $0.id == agent.agentID }.map(name(of:)) ?? agent.name
    }

    // copy:
    /// What Home's header puts between the names of the sessions at work.
    static let headerNameSeparator = ", "

    // MARK: Mentions

    /// What the @ completion offers for what is typed after the @: every live session in Home's order whose name,
    /// folder or branch holds it.
    func options(matching query: String) -> [LiveSession] {
        let query = query.lowercased()
        return sessions.filter { session in
            query.isEmpty || [name(of: session), session.folder, session.branch]
                .contains { $0.lowercased().contains(query) }
        }
    }

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

    /// What follows the @ being typed at the end of a draft, or nil when its last word is not a mention being typed.
    /// The @ starts the text or follows a space, and what follows holds only the characters a name can.
    nonisolated static func mentionQuery(in text: String) -> String? {
        guard let at = text.lastIndex(of: "@"), startsMention(text, at: at) else { return nil }
        let typed = text[text.index(after: at)...]
        guard typed.allSatisfy(isNameCharacter) else { return nil }
        return String(typed)
    }

    /// The draft with its last @ word replaced by a session's name and a space, as picking it in the completion does.
    nonisolated static func completing(_ text: String, with name: String) -> String {
        var text = text
        if let at = text.lastIndex(of: "@"), !text[at...].contains(" ") { text.removeSubrange(at...) }
        return text + "@\(name) "
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

/// The @names a room's messages draw in the accent, as the approved Home draws them: the live sessions of the room's
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
        return HomeSessions.mentionRanges(in: text).compactMap { range in
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

    /// The fill behind a mention of the viewer: Nugget 100 in light mode and Nugget 900 in dark, as the approved Home
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
