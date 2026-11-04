import XCTest
@testable import CowchatMac

/// First run's steps, the agents and Settings' agents, the last step's wait for the first session, and New room's
/// friend search with the invitations it sends.
@MainActor
final class FirstRunTests: XCTestCase {
    /// An installer that records each call and says where the agent stood while its install ran.
    private final class RecordingInstaller: AgentHookInstaller {
        var states: [LiveSession.Harness: AgentHookState] = [:]
        private(set) var calls: [String] = []
        weak var connections: AgentConnections?
        private(set) var whileConnecting: [AgentHookState] = []

        func state(of harness: LiveSession.Harness) async -> AgentHookState { states[harness] ?? .notConnected }

        func connect(_ harness: LiveSession.Harness) async throws -> AgentHookState {
            calls.append("connect \(harness.rawValue)")
            if let connections { whileConnecting.append(connections.state(of: harness)) }
            let state: AgentHookState = harness == .codex ? .waitingForTrust : .connected
            states[harness] = state
            return state
        }
    }

    private let start = Date(timeIntervalSince1970: 1_800_000_000)

    private func agent(_ id: String, minute: Int) -> AgentPresence {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return AgentPresence(agentID: id, name: id, capabilities: ["harness:claude-code", "folder:cowboy", "branch:\(id)"],
                             connectedAt: formatter.string(from: start.addingTimeInterval(TimeInterval(minute * 60))),
                             status: "idle")
    }

    private func friend(_ id: String, _ name: String, agents: [String] = []) -> SocialFriend {
        SocialFriend(account: SocialAccount(id: id, handle: name.replacingOccurrences(of: " ", with: "-"),
                                            displayName: name),
                     agents: agents.map { SocialAgent(id: $0, ownerId: id, name: $0, discoverable: true) })
    }

    // MARK: Steps

    func testTheStepsRunInTheAppsOrderWithTheAgentsAfterTheName() {
        XCTAssertEqual(WizardStep.allCases, [.login, .name, .agents, .ready])

        let firstRun = FirstRun()
        XCTAssertEqual(firstRun.step, .login)
        XCTAssertFalse(firstRun.recovered(hasSetup: false))
        XCTAssertEqual(firstRun.step, .name)
        firstRun.homeCreated()
        XCTAssertEqual(firstRun.step, .agents)
        firstRun.agentsDone()
        XCTAssertEqual(firstRun.step, .ready)
        XCTAssertFalse(firstRun.skippedAgents)
        XCTAssertTrue(firstRun.waitsForFirstSession)

        firstRun.reset()
        XCTAssertEqual(firstRun.step, .login, "Signing out goes back to the sign-in step")
        XCTAssertFalse(firstRun.waitsForFirstSession)
    }

    func testSkipGoesOnToTheWaitAndLeavesBothAgentsUnconnected() async {
        let installer = RecordingInstaller()
        let agents = AgentConnections(installer: installer)
        let firstRun = FirstRun(step: .agents)

        firstRun.skipAgents()
        XCTAssertEqual(firstRun.step, .ready)
        XCTAssertTrue(firstRun.skippedAgents)
        XCTAssertTrue(firstRun.waitsForFirstSession)
        await agents.refresh()
        XCTAssertFalse(agents.hasConnected)
        XCTAssertEqual(AgentConnections.harnesses.map(agents.state(of:)), [.notConnected, .notConnected])
        XCTAssertTrue(installer.calls.isEmpty)
    }

    /// An account that signs in with its Home already made skips first run, and the wizard opens Home. One that made
    /// its Home in this first run stays where it is.
    func testAnAccountThatComesBackWithAHomeSkipsFirstRun() {
        let returning = FirstRun()
        XCTAssertTrue(returning.recovered(hasSetup: true))
        XCTAssertEqual(returning.step, .login, "The signed-out page finds Home while it opens")
        XCTAssertFalse(returning.waitsForFirstSession)

        let onTheName = FirstRun(step: .name)
        XCTAssertTrue(onTheName.recovered(hasSetup: true))

        for step in [WizardStep.agents, .ready] {
            let partWay = FirstRun(step: step)
            XCTAssertFalse(partWay.recovered(hasSetup: true), "\(step)")
            XCTAssertEqual(partWay.step, step)
        }
    }

    // MARK: Agents

    func testEachConnectInstallsItsAgentAndCodexWaitsForTrust() async {
        let installer = RecordingInstaller()
        let agents = AgentConnections(installer: installer)
        installer.connections = agents
        XCTAssertFalse(agents.hasConnected)

        await agents.connect(.claudeCode)
        await agents.connect(.codex)
        XCTAssertEqual(installer.calls, ["connect claude-code", "connect codex"])
        XCTAssertEqual(installer.whileConnecting, [.connecting, .connecting])
        XCTAssertEqual(agents.state(of: .claudeCode), .connected)
        XCTAssertEqual(agents.state(of: .codex), .waitingForTrust)
        XCTAssertTrue(agents.hasConnected)
        XCTAssertTrue(agents.isWaitingForTrust)

        // The person trusts the hooks in Codex, and the list asks again.
        installer.states[.codex] = .connected
        await agents.refresh()
        XCTAssertEqual(agents.state(of: .codex), .connected)
        XCTAssertFalse(agents.isWaitingForTrust)

        await agents.connect(.claudeCode)
        XCTAssertEqual(installer.calls.count, 2, "Nothing connected is installed again")
    }

    /// An agent this Mac doesn't have is installed from its page first; Connect does nothing for it.
    func testAnAgentNotInstalledIsNotConnectedThroughTheHooks() async {
        let installer = RecordingInstaller()
        installer.states = [.codex: .notInstalled]
        let agents = AgentConnections(installer: installer)
        await agents.refresh()
        XCTAssertEqual(agents.state(of: .codex), .notInstalled)
        XCTAssertFalse(agents.hasConnected)

        await agents.connect(.codex)
        XCTAssertTrue(installer.calls.isEmpty)
        XCTAssertEqual(agents.state(of: .codex), .notInstalled)
        XCTAssertEqual(LiveSession.Harness.codex.installPage.scheme, "https")
        XCTAssertEqual(LiveSession.Harness.claudeCode.installPage.scheme, "https")
    }

    /// Settings' Connect connects the one agent its row is for.
    func testSettingsConnectsOneAgent() async {
        let installer = RecordingInstaller()
        installer.states = [.claudeCode: .connected]
        let agents = AgentConnections(installer: installer)
        await agents.refresh()
        XCTAssertEqual(agents.state(of: .codex), .notConnected)

        await agents.connect(.codex)
        XCTAssertEqual(installer.calls, ["connect codex"])
        XCTAssertEqual(agents.state(of: .codex), .waitingForTrust)
        XCTAssertEqual(agents.state(of: .claudeCode), .connected)
    }

    /// Until task 13 lands, the app's own installer connects nothing and Connect says so.
    func testTheUnavailableInstallerSaysItCannotConnectYet() async {
        let agents = AgentConnections()
        await agents.connect(.claudeCode)
        XCTAssertEqual(agents.state(of: .claudeCode), .notConnected)
        XCTAssertEqual(agents.errorMessage, "This version of Cowchat can't connect Claude Code yet.")
    }

    // MARK: The first session

    func testTheLastStepOpensOnTheFirstSessionToJoinAfterItStartsWaiting() {
        let sessions = HomeSessions()
        sessions.update(from: [agent("already-here", minute: 0)])
        let firstRun = FirstRun(step: .agents, sessions: sessions)
        var opened: [String] = []
        firstRun.onFirstSession = { opened.append($0.id) }

        sessions.update(from: [agent("already-here", minute: 0), agent("before-ready", minute: 1)])
        XCTAssertTrue(opened.isEmpty, "Nothing opens before the last step")

        firstRun.agentsDone()
        sessions.update(from: [agent("already-here", minute: 0), agent("before-ready", minute: 1)])
        XCTAssertTrue(opened.isEmpty, "Sessions already in Home do not count")

        sessions.update(from: [agent("already-here", minute: 0), agent("before-ready", minute: 1),
                               agent("first-look", minute: 2)])
        XCTAssertEqual(opened, ["first-look"])
        XCTAssertFalse(firstRun.waitsForFirstSession)

        sessions.update(from: [agent("first-look", minute: 2), agent("second", minute: 3)])
        XCTAssertEqual(opened, ["first-look"], "Only the first one opens")
    }

    // MARK: New room

    func testTheFriendSearchMatchesNameOrHandleAndAnEmptySearchMatchesNone() {
        let friends = [friend("chad", "Chad DePue"), friend("logan", "Logan Lamb"), friend("nico", "Nico")]
        XCTAssertEqual(FriendSearch.matches(friends, query: "").map(\.id), [])
        XCTAssertEqual(FriendSearch.matches(friends, query: "  ").map(\.id), [])
        XCTAssertEqual(FriendSearch.matches(friends, query: "chad").map(\.id), ["chad"])
        XCTAssertEqual(FriendSearch.matches(friends, query: "@Logan-L").map(\.id), ["logan"], "A handle, with its @")
        XCTAssertEqual(FriendSearch.matches(friends, query: "N").map(\.id), ["logan", "nico"],
                       "Any part of a name, in the friends list's order")
        XCTAssertEqual(FriendSearch.matches(friends, query: "dana").map(\.id), [])
    }

    func testRoomCreationInvitesTheFirstFriendAndEachOtherGetsAnInvitation() {
        let chad = friend("chad", "Chad DePue", agents: ["chad-bot"])
        let logan = friend("logan", "Logan Lamb", agents: ["logan-bot"])
        var draft = SocialRoomDraft(name: " pricing ", friendIDs: ["chad", "logan"])
        draft.proposedAgentIDs = ["chad-bot", "logan-bot"]

        let create = draft.requestBody(inviting: chad)
        XCTAssertEqual(create["name"] as? String, "pricing")
        XCTAssertEqual(create["friend_id"] as? String, "chad")
        XCTAssertEqual(create["proposed_agent_ids"] as? [String], ["chad-bot"], "Only the first friend's agents")

        let invite = draft.invitationBody(for: logan)
        XCTAssertEqual(invite["friend_id"] as? String, "logan")
        XCTAssertEqual(invite["proposed_agent_ids"] as? [String], ["logan-bot"])

        draft.friendID = "logan"
        XCTAssertEqual(draft.friendIDs, ["logan"], "Setting the one friend replaces the list")
    }
}
