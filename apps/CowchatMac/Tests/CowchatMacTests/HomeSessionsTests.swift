import XCTest
@testable import CowchatMac

/// The live sessions in Home: reading them from Home's agents, their names, the five rows under Home, the row for the
/// rest, and the @ completion's parts.
@MainActor
final class HomeSessionsTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_800_000_000)

    private func agent(_ id: String, _ capabilities: [String], status: String? = nil,
                       startedMinutesIn minutes: Int = 0) -> AgentPresence {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return AgentPresence(agentID: id, name: id, capabilities: capabilities,
                             connectedAt: formatter.string(from: start.addingTimeInterval(TimeInterval(minutes * 60))),
                             status: status)
    }

    private func session(_ id: String, _ branch: String, folder: String = "cowchat", harness: LiveSession.Harness =
                         .claudeCode, state: LiveSession.State = .idle, minute: Int) -> LiveSession {
        LiveSession(id: id, harness: harness, folder: folder, branch: branch, state: state,
                    startedAt: start.addingTimeInterval(TimeInterval(minute * 60)))
    }

    private func home(_ sessions: [LiveSession]) -> HomeSessions {
        let home = HomeSessions()
        home.update(from: sessions.map { session in
            agent(session.id, [
                "harness:\(session.harness.rawValue)", "folder:\(session.folder)", "branch:\(session.branch)",
            ], status: session.state == .working ? "working" : session.state == .prompt ? "prompt" : "idle",
                  startedMinutesIn: Int(session.startedAt.timeIntervalSince(start) / 60))
        })
        return home
    }

    // MARK: Parsing

    func testASessionIsReadFromItsAgentsCapabilitiesAndPresence() throws {
        let parsed = try XCTUnwrap(LiveSession(agent: agent(
            "bot-patrick.p-dash", ["harness:codex", "folder:dashboard", "branch:pmandia/parity-sign-in"],
            status: "prompt", startedMinutesIn: 40)))
        XCTAssertEqual(parsed.id, "bot-patrick.p-dash")
        XCTAssertEqual(parsed.harness, .codex)
        XCTAssertEqual(parsed.folder, "dashboard")
        XCTAssertEqual(parsed.branch, "pmandia/parity-sign-in")
        XCTAssertEqual(parsed.state, .prompt)
        XCTAssertEqual(parsed.startedAt, start.addingTimeInterval(40 * 60))

        let working = LiveSession(agent: agent("s", ["harness:claude-code", "folder:cowchat"], status: "working"))
        XCTAssertEqual(working?.harness, .claudeCode)
        XCTAssertEqual(working?.state, .working)
        XCTAssertEqual(working?.branch, "", "A hook that sends no branch leaves it empty")
        // The CLI's `wait` sets `waiting` while it listens, which is not a prompt.
        XCTAssertEqual(LiveSession(agent: agent("s", ["harness:codex", "folder:x"], status: "waiting"))?.state, .idle)
        XCTAssertEqual(LiveSession(agent: agent("s", ["harness:codex", "folder:x"]))?.state, .idle)
    }

    func testAnAgentThatIsNotASessionIsLeftOut() {
        XCTAssertNil(LiveSession(agent: agent("mac-patrick-account", ["macos-ui"])), "The person's Mac")
        XCTAssertNil(LiveSession(agent: agent("s", ["harness:cursor", "folder:x"])), "A harness the app can't draw")
        XCTAssertNil(LiveSession(agent: agent("s", ["harness:codex"])), "No folder")
        XCTAssertNil(LiveSession(agent: agent("s", ["harness:codex", "folder:"])), "An empty folder")
    }

    func testHomeListsSessionsNewestStartedFirstAndLeavesOutOtherAgents() {
        let home = HomeSessions()
        home.update(from: [
            agent("a", ["harness:claude-code", "folder:prototype-site", "branch:pmandia/cowchat-swift"],
                  startedMinutesIn: 5),
            agent("mac-patrick-account", ["macos-ui"], startedMinutesIn: 90),
            agent("c", ["harness:claude-code", "folder:cowchat", "branch:pmandia/unread-counts"], startedMinutesIn: 70),
            agent("b", ["harness:codex", "folder:dashboard", "branch:pmandia/parity-sign-in"], startedMinutesIn: 40),
        ])
        XCTAssertEqual(home.sessions.map(\.id), ["c", "b", "a"])
        XCTAssertEqual(home.sessions.map(home.name(of:)), ["unread-counts", "parity-sign-in", "cowchat-swift"])
    }

    // MARK: Names

    func testANameIsTheBranchWithoutItsPrefixOrTheFolderOnMain() {
        XCTAssertEqual(session("s", "pmandia/parity-sign-in", minute: 0).baseName, "parity-sign-in")
        XCTAssertEqual(session("s", "fix-it", minute: 0).baseName, "fix-it")
        XCTAssertEqual(session("s", "main", folder: "prototype-site", minute: 0).baseName, "prototype-site")
        XCTAssertEqual(session("s", "master", folder: "infra", minute: 0).baseName, "infra")
        XCTAssertEqual(session("s", "", folder: "infra", minute: 0).baseName, "infra")
        // The second line names the folder, or the branch when the name already says the folder.
        XCTAssertEqual(session("s", "pmandia/parity-sign-in", folder: "dashboard", minute: 0).place, "dashboard")
        XCTAssertEqual(session("s", "main", folder: "prototype-site", minute: 0).place, "main")
    }

    func testTwoSessionsOnOneBranchAreNumberedByStartOrderAndKeepTheirNames() {
        let first = session("first", "pmandia/actor-logs", minute: 0)
        let second = session("second", "pmandia/actor-logs", minute: 10)
        let third = session("third", "pmandia/actor-logs", minute: 20)
        var names = HomeSessions.settledNames(for: [third, first, second], keeping: [:])
        XCTAssertEqual(names, ["first": "actor-logs", "second": "actor-logs-2", "third": "actor-logs-3"])

        // The first ends: the others keep their names, and the next to start takes the lowest free one.
        names = HomeSessions.settledNames(for: [second, third], keeping: names)
        XCTAssertEqual(names, ["second": "actor-logs-2", "third": "actor-logs-3"])
        let fourth = session("fourth", "pmandia/actor-logs", minute: 30)
        names = HomeSessions.settledNames(for: [second, third, fourth], keeping: names)
        XCTAssertEqual(names["fourth"], "actor-logs")
    }

    func testTheModelKeepsANameWhileTheSessionLives() {
        let home = home([session("a", "pmandia/x", minute: 0), session("b", "pmandia/x", minute: 5)])
        XCTAssertEqual(home.sessions.map(home.name(of:)), ["x-2", "x"])
        home.update(from: [agent("b", ["harness:claude-code", "folder:cowchat", "branch:pmandia/x"],
                                 startedMinutesIn: 5)])
        XCTAssertEqual(home.sessions.map(home.name(of:)), ["x-2"], "b stays x-2 after a ends")
    }

    // MARK: The five

    func testFiveOrFewerShowEveryOneWithNoMoreRow() {
        let five = home((0..<5).map { session("s\($0)", "b\($0)", minute: $0) })
        XCTAssertEqual(five.picked.map(\.id), ["s4", "s3", "s2", "s1", "s0"])
        XCTAssertEqual(five.hiddenCount, 0)
        let three = home((0..<3).map { session("s\($0)", "b\($0)", state: $0 == 0 ? .prompt : .idle, minute: $0) })
        XCTAssertEqual(three.picked.map(\.id), ["s2", "s1", "s0"], "Nothing moves when every session shows")
        XCTAssertEqual(three.hiddenCount, 0)
    }

    func testThePickHoldsEveryPromptThenTheNewestInHomesOrder() {
        // Eight sessions, s7 newest. s1 and s5 wait at a prompt.
        let home = home((0..<8).map { index in
            session("s\(index)", "b\(index)", state: [1, 5].contains(index) ? .prompt : .working, minute: index)
        })
        XCTAssertEqual(home.picked.map(\.id), ["s7", "s6", "s5", "s4", "s1"],
                       "Both prompts and the three newest of the rest, newest first")
        XCTAssertEqual(home.hiddenCount, 3)
        XCTAssertEqual(home.shown.map(\.id), home.picked.map(\.id))
        home.showsAll = true
        XCTAssertEqual(home.shown.map(\.id), ["s7", "s6", "s5", "s4", "s3", "s2", "s1", "s0"])
    }

    func testMoreThanFivePromptsAllShow() {
        let home = home((0..<7).map { session("s\($0)", "b\($0)", state: $0 == 3 ? .idle : .prompt, minute: $0) })
        XCTAssertEqual(home.picked.map(\.id), ["s6", "s5", "s4", "s2", "s1", "s0"])
        XCTAssertEqual(home.hiddenCount, 1)
    }

    func testTheMoreRowSaysHowManyItShowsAndThenHides() {
        let home = home((0..<30).map { session("s\($0)", "b\($0)", minute: $0) })
        XCTAssertEqual(home.moreTitle, "Show 25 more")
        home.showsAll = true
        XCTAssertEqual(home.moreTitle, "Hide 25 sessions")
        let six = self.home((0..<6).map { session("s\($0)", "b\($0)", minute: $0) })
        six.showsAll = true
        XCTAssertEqual(six.moreTitle, "Hide 1 session")
    }

    func testTheSecondLineSaysWhereAPromptIsAnswered() {
        let home = home([
            session("a", "pmandia/a", folder: "dashboard", harness: .codex, state: .prompt, minute: 0),
            session("b", "pmandia/b", harness: .claudeCode, state: .prompt, minute: 1),
            session("c", "pmandia/c", folder: "cowchat", state: .working, minute: 2),
        ])
        XCTAssertEqual(home.sessions.map(home.line(for:)), ["cowchat", "Waiting in Claude Code", "Waiting in Codex"])
    }

    // MARK: Mentions

    func testTheMentionBeingTyped() {
        XCTAssertEqual(HomeSessions.mentionQuery(in: "@"), "")
        XCTAssertEqual(HomeSessions.mentionQuery(in: "please ask @par"), "par")
        XCTAssertNil(HomeSessions.mentionQuery(in: "@parity-sign-in "), "A space ends it")
        XCTAssertNil(HomeSessions.mentionQuery(in: "mail me at p@cowboy"), "An @ inside a word is not a mention")
        XCTAssertNil(HomeSessions.mentionQuery(in: "no mention"))
    }

    func testPickingANameReplacesTheTypedMention() {
        XCTAssertEqual(HomeSessions.completing("@par", with: "parity-sign-in"), "@parity-sign-in ")
        XCTAssertEqual(HomeSessions.completing("ask @", with: "unread-counts"), "ask @unread-counts ")
    }

    func testTheCompletionMatchesNameFolderOrBranch() {
        let home = home([
            session("a", "pmandia/parity-sign-in", folder: "dashboard", minute: 0),
            session("b", "pmandia/unread-counts", folder: "cowchat", minute: 1),
        ])
        XCTAssertEqual(home.options(matching: "").map(\.id), ["b", "a"])
        XCTAssertEqual(home.options(matching: "PAR").map(\.id), ["a"])
        XCTAssertEqual(home.options(matching: "dash").map(\.id), ["a"], "By folder")
        XCTAssertEqual(home.options(matching: "pmandia/un").map(\.id), ["b"], "By branch")
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

    // MARK: Home's header

    func testHomesHeaderNamesItsSessionsWithoutTheirHandle() {
        let home = home([session("a", "pmandia/actor-logs", minute: 0), session("b", "pmandia/actor-logs", minute: 5)])
        let members = [
            AgentPresence(agentID: "mac", name: "mac-patrick", capabilities: [], connectedAt: nil, status: nil),
            AgentPresence(agentID: "b", name: "actor-logs-2 · @Patrick-Mandia", capabilities: [], connectedAt: nil,
                          status: "working"),
            AgentPresence(agentID: "a", name: "actor-logs · @Patrick-Mandia", capabilities: [], connectedAt: nil,
                          status: "working"),
        ]
        XCTAssertEqual(ChatPresencePresentation.summary(members: members, currentAgentID: "mac",
                                                        fallbackMemberCount: nil, name: home.headerName(of:),
                                                        nameSeparator: HomeSessions.headerNameSeparator),
                       "actor-logs-2, actor-logs active")
        XCTAssertEqual(ChatPresencePresentation.summary(members: members, currentAgentID: "mac",
                                                        fallbackMemberCount: nil),
                       "actor-logs-2 · @Patrick-Mandia · actor-logs · @Patrick-Mandia active",
                       "Other rooms keep the server's names")
    }

    func testADraftsMentionsAreTheSessionsItNames() {
        let home = home([
            session("bot-patrick.p-dash", "pmandia/parity-sign-in", minute: 0),
            session("bot-patrick.p-cowchat", "pmandia/unread-counts", minute: 1),
        ])
        XCTAssertEqual(home.mentions(in: "@parity-sign-in and @unread-counts, please check. @parity-sign-in."),
                       ["bot-patrick.p-dash", "bot-patrick.p-cowchat"])
        XCTAssertEqual(home.mentions(in: "@Chad and someone@parity-sign-in"), [], "No live session is named")
    }
}
