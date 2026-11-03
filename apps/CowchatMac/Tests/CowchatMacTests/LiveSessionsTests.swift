import XCTest
@testable import CowchatMac

/// The person's live sessions: reading them from the account rooms' `session` field, keeping them current with
/// `session_state`, their names, the pick of five the sheets use, and the @ mentions.
@MainActor
final class LiveSessionsTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_800_000_000)

    private func stamp(_ minutes: Int) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: start.addingTimeInterval(TimeInterval(minutes * 60)))
    }

    private func room(_ id: String, name: String? = nil, owner: String = "patrick", agent: String? = nil,
                      harness: String = "claude-code", folder: String = "cowchat", branch: String = "",
                      minute: Int = 0, ended: Bool = false) -> SocialRoom {
        SocialRoom(roomId: id, name: name ?? id, ownerId: owner, kind: "session",
                   session: SocialRoom.Session(agentId: agent ?? "agent-\(id)", harness: harness, folder: folder,
                                               branch: branch, startedAt: stamp(minute),
                                               endedAt: ended ? stamp(minute + 30) : nil))
    }

    private func session(_ id: String, _ branch: String, folder: String = "cowchat", harness: LiveSession.Harness =
                         .claudeCode, state: LiveSession.State = .idle, minute: Int) -> LiveSession {
        LiveSession(id: id, harness: harness, folder: folder, branch: branch, state: state,
                    startedAt: start.addingTimeInterval(TimeInterval(minute * 60)))
    }

    private func sessions(_ rooms: [SocialRoom]) -> LiveSessions {
        let sessions = LiveSessions()
        sessions.update(from: rooms, viewerID: "patrick")
        return sessions
    }

    private func event(_ agent: String, _ room: String, _ state: SessionStateEvent.Value) -> SessionStateEvent {
        SessionStateEvent(agentID: agent, roomID: room, state: state)
    }

    // MARK: Read from the rooms

    func testSessionsAreReadFromTheRoomsTheServerMadeForThem() throws {
        let live = sessions([
            room("r-a", name: "cowchat-swift", agent: "a", folder: "prototype-site", branch: "pmandia/cowchat-swift",
                 minute: 5),
            room("r-c", name: "unread-counts", agent: "c", branch: "pmandia/unread-counts", minute: 70),
            room("r-b", name: "parity-sign-in", agent: "b", harness: "codex", folder: "dashboard",
                 branch: "pmandia/parity-sign-in", minute: 40),
        ])
        XCTAssertEqual(live.sessions.map(\.id), ["c", "b", "a"], "Newest started first")
        XCTAssertEqual(live.sessions.map(live.name(of:)), ["unread-counts", "parity-sign-in", "cowchat-swift"])
        let parity = try XCTUnwrap(live.session(inRoom: "r-b"))
        XCTAssertEqual(parity.harness, .codex)
        XCTAssertEqual(parity.folder, "dashboard")
        XCTAssertEqual(parity.branch, "pmandia/parity-sign-in")
        XCTAssertEqual(parity.roomID, "r-b")
        XCTAssertEqual(parity.startedAt, start.addingTimeInterval(40 * 60))
        XCTAssertEqual(parity.state, .idle, "Idle until a session_state says otherwise")
    }

    func testRoomsThatAreNotTheViewersLiveSessionsAreLeftOut() {
        let live = sessions([
            room("ended", ended: true),
            room("chads", owner: "chad"),
            SocialRoom(roomId: "district", name: "District", ownerId: "patrick"),
            room("cursor", harness: "cursor"),
            room("mine", agent: "mine"),
        ])
        XCTAssertEqual(live.sessions.map(\.id), ["mine"])
        XCTAssertNil(live.session(inRoom: "ended"))
    }

    // MARK: session_state

    func testEachStateTheServerSendsReachesTheSession() {
        let live = sessions([room("r", agent: "s")])
        XCTAssertTrue(live.apply(event("s", "r", .working)))
        XCTAssertEqual(live.sessions.first?.state, .working)
        live.apply(event("s", "r", .prompt))
        XCTAssertEqual(live.sessions.first?.state, .prompt)
        live.apply(event("s", "r", .idle))
        XCTAssertEqual(live.sessions.first?.state, .idle)
        live.apply(event("s", "r", .ended))
        XCTAssertTrue(live.sessions.isEmpty, "An ended session goes at once, before the rooms list says so")
    }

    func testAnEventForARoomNotListedYetAsksForTheRooms() {
        let live = sessions([room("r", agent: "s")])
        XCTAssertFalse(live.apply(event("new", "r-new", .working)), "The caller lists the rooms again")
        XCTAssertEqual(live.sessions.map(\.id), ["s"])
        live.update(from: [room("r", agent: "s"), room("r-new", agent: "new", minute: 9)], viewerID: "patrick")
        XCTAssertEqual(live.sessions.map(\.id), ["new", "s"])
        XCTAssertEqual(live.sessions.first?.state, .working, "The event heard before the room was listed still counts")
    }

    func testWaitingSinceIsTheMomentASessionWasSeenStoppingAtAPrompt() {
        let live = sessions([room("r", agent: "s")])
        let now = start.addingTimeInterval(3600)
        live.apply(event("s", "r", .prompt), at: now)
        XCTAssertNil(live.sessions.first?.waitingSince, "Its state before the prompt is unknown")
        live.apply(event("s", "r", .working), at: now)
        live.apply(event("s", "r", .prompt), at: now.addingTimeInterval(60))
        XCTAssertEqual(live.sessions.first?.waitingSince, now.addingTimeInterval(60))
        live.apply(event("s", "r", .prompt), at: now.addingTimeInterval(120))
        XCTAssertEqual(live.sessions.first?.waitingSince, now.addingTimeInterval(60), "A repeated prompt is no change")
    }

    func testAResumedSessionIsLiveAgainBeforeItsRoomSaysSo() {
        let live = sessions([room("r", agent: "s", ended: true)])
        XCTAssertTrue(live.sessions.isEmpty)
        XCTAssertTrue(live.hasEnded(room("r", agent: "s", ended: true).session!))
        live.apply(event("s", "r", .working))
        XCTAssertEqual(live.sessions.map(\.id), ["s"])
        XCTAssertFalse(live.hasEnded(room("r", agent: "s", ended: true).session!), "The event is newer")
    }

    func testSigningOutClearsEverySession() {
        let live = sessions([room("r", agent: "s")])
        live.apply(event("s", "r", .working))
        live.clear()
        XCTAssertTrue(live.sessions.isEmpty)
        XCTAssertTrue(live.roomNames.isEmpty)
        live.update(from: [room("r", agent: "s")], viewerID: "patrick")
        XCTAssertEqual(live.sessions.first?.state, .idle, "What the last account heard is gone")
    }

    // MARK: A session read from an agent

    private func agent(_ id: String, _ capabilities: [String], status: String? = nil) -> AgentPresence {
        AgentPresence(agentID: id, name: id, capabilities: capabilities, connectedAt: stamp(40), status: status)
    }

    /// The Inbox still reads a community sender's session from its capabilities.
    func testASessionIsReadFromAnAgentsCapabilitiesAndPresence() throws {
        let parsed = try XCTUnwrap(InboxStore.session(of: agent(
            "bot-patrick.p-dash", ["harness:codex", "folder:dashboard", "branch:pmandia/parity-sign-in"],
            status: "prompt")))
        XCTAssertEqual(parsed.id, "bot-patrick.p-dash")
        XCTAssertEqual(parsed.harness, .codex)
        XCTAssertEqual(parsed.folder, "dashboard")
        XCTAssertEqual(parsed.branch, "pmandia/parity-sign-in")
        XCTAssertEqual(parsed.state, .prompt)
        XCTAssertEqual(parsed.startedAt, start.addingTimeInterval(40 * 60))
        let working = InboxStore.session(of: agent("s", ["harness:claude-code", "folder:cowchat"], status: "working"))
        XCTAssertEqual(working?.state, .working)
        XCTAssertEqual(working?.branch, "", "A hook that sends no branch leaves it empty")
        // The CLI's `wait` sets `waiting` while it listens, which is not a prompt.
        XCTAssertEqual(InboxStore.session(of: agent("s", ["harness:codex", "folder:x"], status: "waiting"))?.state,
                       .idle)
        XCTAssertNil(InboxStore.session(of: agent("mac-patrick-account", ["macos-ui"])), "The person's Mac")
        XCTAssertNil(InboxStore.session(of: agent("s", ["harness:cursor", "folder:x"])), "A harness the app can't draw")
        XCTAssertNil(InboxStore.session(of: agent("s", ["harness:codex"])), "No folder")
    }

    // MARK: Names

    func testANameIsTheBranchWithoutItsPrefixOrTheFolderOnMain() {
        XCTAssertEqual(session("s", "pmandia/parity-sign-in", minute: 0).baseName, "parity-sign-in")
        XCTAssertEqual(session("s", "fix-it", minute: 0).baseName, "fix-it")
        XCTAssertEqual(session("s", "main", folder: "prototype-site", minute: 0).baseName, "prototype-site")
        XCTAssertEqual(session("s", "master", folder: "infra", minute: 0).baseName, "infra")
        XCTAssertEqual(session("s", "", folder: "infra", minute: 0).baseName, "infra")
        // The second fact names the folder, or the branch when the name already says the folder.
        XCTAssertEqual(session("s", "pmandia/parity-sign-in", folder: "dashboard", minute: 0).place, "dashboard")
        XCTAssertEqual(session("s", "main", folder: "prototype-site", minute: 0).place, "main")
    }

    func testASessionsNameIsItsRoomsName() {
        let live = sessions([room("r", name: "actor-logs-2", agent: "s", branch: "pmandia/actor-logs")])
        XCTAssertEqual(live.sessions.map(live.name(of:)), ["actor-logs-2"])
        XCTAssertEqual(live.name(of: session("unknown", "pmandia/other", minute: 0)), "other",
                       "A session with no room yet falls back to its branch")
    }

    // MARK: The five

    func testFiveOrFewerShowEveryOne() {
        let five = (0..<5).reversed().map { session("s\($0)", "b\($0)", minute: $0) }
        XCTAssertEqual(LiveSessions.pick(from: five).map(\.id), ["s4", "s3", "s2", "s1", "s0"])
        let three = (0..<3).reversed().map {
            session("s\($0)", "b\($0)", state: $0 == 0 ? .prompt : .idle, minute: $0)
        }
        XCTAssertEqual(LiveSessions.pick(from: three).map(\.id), ["s2", "s1", "s0"], "Nothing moves")
    }

    func testThePickHoldsEveryPromptThenTheNewest() {
        // Eight sessions, s7 newest. s1 and s5 wait at a prompt.
        let eight = (0..<8).reversed().map { index in
            session("s\(index)", "b\(index)", state: [1, 5].contains(index) ? .prompt : .working, minute: index)
        }
        XCTAssertEqual(LiveSessions.pick(from: eight).map(\.id), ["s7", "s6", "s5", "s4", "s1"],
                       "Both prompts and the three newest of the rest, newest first")
        let seven = (0..<7).reversed().map { session("s\($0)", "b\($0)", state: $0 == 3 ? .idle : .prompt, minute: $0) }
        XCTAssertEqual(LiveSessions.pick(from: seven).map(\.id), ["s6", "s5", "s4", "s2", "s1", "s0"],
                       "More than five at a prompt all show")
    }

    func testTheMoreRowSaysHowManyItShowsAndThenHides() {
        XCTAssertEqual(LiveSessions.moreTitle(hidden: 25, showsAll: false), "Show 25 more")
        XCTAssertEqual(LiveSessions.moreTitle(hidden: 25, showsAll: true), "Hide 25 sessions")
        XCTAssertEqual(LiveSessions.moreTitle(hidden: 1, showsAll: true), "Hide 1 session")
    }

    func testTheLineSaysWhereAPromptIsAnswered() {
        let live = LiveSessions()
        XCTAssertEqual(live.line(for: session("a", "pmandia/a", folder: "dashboard", harness: .codex, state: .prompt,
                                              minute: 0)), "Waiting in Codex")
        XCTAssertEqual(live.line(for: session("b", "pmandia/b", state: .prompt, minute: 1)), "Waiting in Claude Code")
        XCTAssertEqual(live.line(for: session("c", "pmandia/c", folder: "cowchat", state: .working, minute: 2)),
                       "cowchat")
    }

    // MARK: Mentions

    func testADraftsMentionsAreTheLiveSessionsItNames() {
        let live = sessions([
            room("r-dash", name: "parity-sign-in", agent: "bot-patrick.p-dash", minute: 0),
            room("r-chat", name: "unread-counts", agent: "bot-patrick.p-cowchat", minute: 1),
        ])
        XCTAssertEqual(live.mentions(in: "@parity-sign-in and @unread-counts, please check. @parity-sign-in."),
                       ["bot-patrick.p-dash", "bot-patrick.p-cowchat"])
        XCTAssertEqual(live.mentions(in: "@Chad and someone@parity-sign-in"), [], "No live session is named")
    }

    // MARK: Mentions drawn in the accent

    private let patrick = SocialAccount(id: "patrick", handle: "Patrick-Mandia", displayName: "Patrick Mandia")
    private let chad = SocialAccount(id: "chad", handle: "Chad-DePue", displayName: "Chad DePue")

    private func marked(_ marks: MentionMarks, _ text: String, completedOnly: Bool = false) -> [String] {
        marks.marks(in: text, completedOnly: completedOnly).map { "\(text[$0.range])\($0.namesViewer ? " (viewer)" : "")" }
    }

    func testAMessageMarksTheOwnersSessionsAndKnownPeople() {
        let marks = MentionMarks(sessionNames: ["parity-sign-in", "unread-counts-2"], people: [chad], viewer: patrick)
        XCTAssertEqual(marked(marks, "@parity-sign-in wait for @Chad, then ask @Unread-Counts-2."),
                       ["@parity-sign-in", "@Chad", "@Unread-Counts-2"])
        XCTAssertEqual(marked(marks, "@Patrick should we use it? @Patrick-Mandia, @Chad-DePue"),
                       ["@Patrick (viewer)", "@Patrick-Mandia (viewer)", "@Chad-DePue"])
        XCTAssertEqual(marked(marks, "@nobody and mail p@parity-sign-in and @parity"), [],
                       "Only a known name after an @ that starts a word")
        XCTAssertEqual(marked(MentionMarks(), "@parity-sign-in @Patrick"), [], "No names, no marks")
    }

    func testTheComposerMarksAMentionOnceItIsTypedOut() {
        let marks = MentionMarks(sessionNames: ["parity-sign-in"], people: [], viewer: nil)
        XCTAssertEqual(marked(marks, "@parity-sign-in", completedOnly: true), [], "Still being typed")
        XCTAssertEqual(marked(marks, "@parity-sign-in ", completedOnly: true), ["@parity-sign-in"])
        XCTAssertEqual(marked(marks, "ask @parity-sign-in.", completedOnly: true), ["@parity-sign-in"])
    }

    func testMarkingColoursOnlyTheMentionsAndFillsTheViewers() {
        let marks = MentionMarks(sessionNames: ["unread-counts"], people: [], viewer: patrick)
        let source = (try? AttributedString(markdown: "**Ready**: @unread-counts asks @Patrick to merge"))
            ?? AttributedString()
        let result = marks.marking(source)
        let coloured = result.runs.filter { $0.swiftUI.foregroundColor != nil }
            .map { String(result[$0.range].characters) }
        XCTAssertEqual(coloured, ["@unread-counts", "@Patrick"])
        let filled = result.runs.filter { $0.swiftUI.backgroundColor != nil }.map { String(result[$0.range].characters) }
        XCTAssertEqual(filled, ["@Patrick"])
        XCTAssertEqual(String(result.characters), String(source.characters), "The text itself is unchanged")
        XCTAssertTrue(result.runs.contains { $0.inlinePresentationIntent == .stronglyEmphasized },
                      "Markdown's own styling stays")
    }
}
