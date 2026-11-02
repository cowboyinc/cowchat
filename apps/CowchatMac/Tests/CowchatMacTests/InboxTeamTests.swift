import Foundation
import XCTest
@testable import CowchatMac

/// The Inbox's and Team's store logic, on the replies `GET /social/v1/inbox` and `GET /social/v1/team` send (Chad's
/// plan, tasks 5 and 12).
@MainActor
final class InboxTeamTests: XCTestCase {
    private let chad = SocialAccount(id: "chad", handle: "Chad-DePue", displayName: "Chad DePue")
    private let patrick = SocialAccount(id: "patrick", handle: "Patrick-Mandia", displayName: "Patrick Mandia")
    private var suites: [String] = []

    override func tearDown() async throws {
        for suite in suites { UserDefaults().removePersistentDomain(forName: suite) }
        suites = []
        try await super.tearDown()
    }

    private func defaults() -> UserDefaults {
        let suite = "CowchatInboxTests.\(UUID().uuidString)"
        suites.append(suite)
        return UserDefaults(suiteName: suite)!
    }

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(type, from: Data(json.utf8))
    }

    private func account(_ account: SocialAccount) -> String {
        #"{"id":"\#(account.id)","handle":"\#(account.handle)","display_name":"\#(account.displayName)"}"#
    }

    private func invitation(_ id: String, status: String, at: String = "2026-11-01T18:52:00.000Z") -> String {
        #"""
        {"id":"invitation.\#(id)","kind":"invitation","room_id":"room-\#(id)","room_name":"unread-counts",
         "at":"\#(at)","invitation":{"id":"\#(id)","room_id":"room-\#(id)","room_name":"unread-counts",
         "from":\#(account(patrick)),"to":\#(account(chad)),"proposed_agents":[],"members":[\#(account(patrick))],
         "agents":[],"history_policy":"full","encrypted":false,"slack_connected":false,"status":"\#(status)"}}
        """#
    }

    private func message(_ id: String, kind: String, sender: String, at: String, content: String,
                         room: String = "home-chad", roomName: String = "Chad's Home") -> String {
        #"""
        {"id":"message.\#(id)","kind":"\#(kind)","room_id":"\#(room)","room_name":"\#(roomName)","at":"\#(at)",
         "content":"\#(content)","sender":\#(sender)}
        """#
    }

    private func items(_ wire: [String], viewer: SocialAccount? = nil) throws -> [InboxItem] {
        let reply = try decode(InboxReply.self, #"{"items":[\#(wire.joined(separator: ","))]}"#)
        return reply.items.compactMap { InboxItem(wire: $0, viewer: viewer ?? chad) }
    }

    /// A poll's reply, as `InboxStore.refresh` hands it over: the items and the invitations they carry.
    private func receive(_ wire: [String], into inbox: InboxStore) throws {
        let reply = try decode(InboxReply.self, #"{"items":[\#(wire.joined(separator: ","))]}"#)
        inbox.receive(reply.items.compactMap { InboxItem(wire: $0, viewer: chad) },
                      invitations: reply.items.compactMap(\.invitation))
    }

    private var chadsWire: [String] {
        [
            invitation("invitation-1", status: "pending"),
            invitation("invitation-2", status: "declined"),
            message("district-3", kind: "mention",
                    sender: #"{"kind":"person","account":\#(account(SocialAccount(id: "theo", handle: "Theo-Park", displayName: "Theo Park"))),"agent_id":"mac-theo-account","name":"Theo Park"}"#,
                    at: "2026-11-01T18:12:00.000Z", content: "@Chad can you confirm which price District should use?",
                    room: "district", roomName: "District"),
            message("home-c-4", kind: "mention",
                    sender: #"{"kind":"agent","account":\#(account(chad)),"agent_id":"bot-chad.c-store","name":"hire-fees"}"#,
                    at: "2026-11-01T18:40:00.000Z", content: "@Chad should the ledger show cents?"),
            message("ledger-5", kind: "marked",
                    sender: #"{"kind":"agent","account":{"id":"nico","handle":"Nico","display_name":"Nico"},"agent_id":"bot-nico.n-ledger","name":"fee-ledger"}"#,
                    at: "2026-11-01T19:11:00.000Z", content: "The ledger is ready. @Patrick please look too.",
                    room: "fee-ledger", roomName: "fee-ledger"),
        ]
    }

    // MARK: What reaches the Inbox

    func testEachKindFromTheWire() throws {
        let items = try items(chadsWire)
        XCTAssertEqual(items.map(\.id), ["invitation.invitation-1", "message.district-3", "message.home-c-4",
                                         "message.ledger-5"], "A declined invitation is left out")
        let invitation = items[0]
        XCTAssertTrue(invitation.isOpenInvitation)
        XCTAssertEqual(invitation.title { _ in nil }, "Patrick Mandia")
        XCTAssertEqual(invitation.message, "Invited you to join.")
        XCTAssertEqual(invitation.invitationID, "invitation-1")

        XCTAssertEqual(items[1].title { _ in nil }, "Theo Park")
        XCTAssertEqual(items[1].message, "can you confirm which price District should use?")
        XCTAssertEqual(items[2].sender, .ownSession(agentID: "bot-chad.c-store", name: "hire-fees"))
        XCTAssertEqual(items[2].title { $0 == "bot-chad.c-store" ? "hire-fees-2" : nil }, "hire-fees-2",
                       "One of his own sessions, as his Home names it")
        XCTAssertEqual(items[3].title { _ in nil }, "Nico's fee-ledger")
        XCTAssertEqual(items[3].message, "The ledger is ready. @Patrick please look too.", "Only a leading mention goes")
        XCTAssertTrue(items.allSatisfy { $0.server == .accountGlobal })
    }

    func testALeadingMentionOfTheViewerIsDropped() {
        XCTAssertEqual(InboxItem.line(for: "@Chad could you review", viewer: chad), "could you review")
        XCTAssertEqual(InboxItem.line(for: "@chad-depue, could you", viewer: chad), "could you")
        XCTAssertEqual(InboxItem.line(for: "@Theo could you", viewer: chad), "@Theo could you")
        XCTAssertEqual(InboxItem.line(for: "  Two\n\nlines  ", viewer: chad), "Two lines")
    }

    func testTheOrder() throws {
        let accepted = invitation("invitation-3", status: "accepted", at: "2026-11-01T19:30:00.000Z")
        let ordered = InboxItem.ordered(try items(chadsWire + [accepted]))
        XCTAssertEqual(ordered.map(\.id), [
            "invitation.invitation-1", "invitation.invitation-3", "message.ledger-5", "message.home-c-4",
            "message.district-3",
        ], "Open invitations first, then everything newest first")
    }

    /// A community room's message on the API key's connection: a session is named with its person from Team, a
    /// person is found by name among the people.
    func testCommunitySenders() throws {
        func chatMessage(_ agentID: String, _ name: String, kind: String) throws -> ChatMessage {
            try JSONDecoder().decode(ChatMessage.self, from: Data(#"""
            {"message_id":"m","room_id":"junction","agent_id":"\#(agentID)","agent_name":"\#(name)","content":"@Chad hi",
             "metadata":{"sender_kind":"\#(kind)","mentions":["chad"]},"timestamp":"2026-11-01T18:41:00.000Z","seq":1}
            """#.utf8))
        }
        let logan = SocialAccount(id: "logan", handle: "Logan-Lamb", displayName: "Logan Lamb")
        let agents = [AgentPresence(agentID: "l-auth-community", name: "sign-in-field",
                                    capabilities: ["harness:codex", "folder:dashboard", "branch:logan/sign-in-field"])]
        let owner: (LiveSession) -> SocialAccount? = { $0.branch == "logan/sign-in-field" ? logan : nil }
        let session = try chatMessage("l-auth-community", "sign-in-field", kind: "agent")
        XCTAssertEqual(InboxStore.communitySender(of: session, agents: agents, people: [logan], viewer: chad,
                                                  owner: owner), .session(owner: "Logan", name: "sign-in-field"))
        XCTAssertEqual(InboxStore.communitySender(of: session, agents: [], people: [logan], viewer: chad,
                                                  owner: owner), .session(owner: nil, name: "sign-in-field"))
        let person = try chatMessage("martin-community", "Martin", kind: "person")
        let martin = SocialAccount(id: "martin", handle: "Martin", displayName: "Martin Kay")
        XCTAssertEqual(InboxStore.communitySender(of: person, agents: [], people: [martin], viewer: chad,
                                                  owner: owner), .person("Martin Kay"))
        XCTAssertTrue(try chatMessage("x", "x", kind: "person").metadata.mentions == ["chad"])
    }

    // MARK: Read state

    func testReadStateIsKeptOnTheMacForEachAccount() throws {
        let defaults = defaults()
        let inbox = InboxStore(defaults: defaults)
        inbox.use(account: chad, scope: "chad-scope")
        try receive(chadsWire, into: inbox)
        XCTAssertTrue(inbox.isLoaded)
        XCTAssertEqual(inbox.unreadCount, 4)
        let mention = try XCTUnwrap(inbox.items.first { $0.id == "message.district-3" })
        inbox.markRead(mention)
        XCTAssertFalse(inbox.isUnread(mention))

        let again = InboxStore(defaults: defaults)
        again.use(account: chad, scope: "chad-scope")
        XCTAssertTrue(again.readIDs.contains(mention.id), "Kept on this Mac")
        again.use(account: patrick, scope: "patrick-scope")
        XCTAssertEqual(again.readIDs, [], "Another account's Inbox starts unread")
        XCTAssertFalse(again.isLoaded)
    }

    func testMarkAllReadLeavesOpenInvitationsUnread() throws {
        let inbox = InboxStore(defaults: defaults())
        inbox.use(account: chad, scope: "scope")
        try receive(chadsWire, into: inbox)
        XCTAssertTrue(inbox.hasUnreadMessages)
        inbox.markAllRead()
        XCTAssertEqual(inbox.items.filter(inbox.isUnread).map(\.id), ["invitation.invitation-1"])
        XCTAssertFalse(inbox.hasUnreadMessages)
        XCTAssertEqual(inbox.unreadCount, 1, "The sidebar counts the open invitation")
        inbox.markUnread(inbox.items[2])
        XCTAssertTrue(inbox.hasUnreadMessages)
    }

    /// Joining makes the invitation read on the next poll; declining takes it out at once.
    func testAnsweredInvitations() throws {
        let inbox = InboxStore(defaults: defaults())
        inbox.use(account: chad, scope: "scope")
        try receive(chadsWire, into: inbox)
        let open = try XCTUnwrap(inbox.items.first)
        XCTAssertNotNil(inbox.invitation(for: open))

        var wire = chadsWire
        wire[0] = invitation("invitation-1", status: "accepted")
        try receive(wire, into: inbox)
        let answered = try XCTUnwrap(inbox.items.first { $0.id == open.id })
        XCTAssertFalse(answered.isOpenInvitation)
        XCTAssertFalse(inbox.isUnread(answered))

        inbox.remove(answered)
        XCTAssertNil(inbox.items.first { $0.id == open.id })
    }

    func testThePanelShowsTheFourNewestUnread() throws {
        let inbox = InboxStore(defaults: defaults())
        inbox.use(account: chad, scope: "scope")
        let more = (0..<4).map { index in
            message("extra-\(index)", kind: "marked",
                    sender: #"{"kind":"agent","account":\#(account(chad)),"agent_id":"a","name":"x"}"#,
                    at: "2026-11-01T17:0\(index):00.000Z", content: "Older \(index)")
        }
        try receive(chadsWire + more, into: inbox)
        XCTAssertEqual(inbox.newestUnread.map(\.id), [
            "message.ledger-5", "invitation.invitation-1", "message.home-c-4", "message.district-3",
        ])
        inbox.markRead(try XCTUnwrap(inbox.items.first { $0.id == "message.ledger-5" }))
        XCTAssertEqual(inbox.newestUnread.last?.id, "message.extra-3")
    }

    // MARK: Team

    private func session(_ id: String, _ folder: String, _ branch: String, started: String, state: String = "idle",
                         room: String? = nil, pullRequest: (Int, String, String)? = nil) -> String {
        var fields = [#""id":"\#(id)""#, #""folder":"\#(folder)""#, #""branch":"\#(branch)""#,
                      #""state":"\#(state)""#, #""started_at":"2026-11-01T\#(started):00.000Z""#]
        if let room { fields.append(#""room":{"room_id":"\#(room)","name":"\#(room)"}"#) }
        if let (number, checks, at) = pullRequest {
            fields.append(#"""
            "pull_request":{"repository":"cowboyinc/\#(folder)","number":\#(number),"title":"Fee ledger \#(number)",
             "checks":"\#(checks)","additions":1,"deletions":1,"address":"https://github.com/cowboyinc/\#(folder)/pull/\#(number)"},
            "pull_request_at":"2026-11-01T\#(at):00.000Z"
            """#)
        }
        return "{" + fields.joined(separator: ",") + "}"
    }

    private func person(_ id: String, _ name: String, _ sessions: [String]) -> String {
        #"{"account":{"id":"\#(id)","handle":"\#(id)","display_name":"\#(name)"},"sessions":[\#(sessions.joined(separator: ","))]}"#
    }

    private func reply(sharing: Bool = true) throws -> TeamReply {
        let nico = person("nico", "Nico", [
            session("n1", "dashboard", "nico/fee-ledger", started: "16:50", room: "fee-ledger",
                    pullRequest: (1806, "running", "19:11")),
            session("n2", "dashboard", "nico/fee-ledger", started: "18:10", state: "working",
                    pullRequest: (1799, "passing", "17:00")),
            session("n3", "cowboy", "nico/store-feed", started: "18:40", state: "prompt"),
            session("n4", "cowboy", "main", started: "15:00"),
        ] + (0..<4).map { session("nx\($0)", "infra", "nico/extra-\($0)", started: "14:0\($0)") })
        let adrian = person("adrian", "Adrian Kenny", [
            session("a1", "prototype-site", "adrian/onboarding", started: "15:50", pullRequest: (480, "passing", "18:10")),
        ])
        let patrick = person("patrick", "Patrick Mandia", [
            session("p1", "cowchat", "pmandia/unread-counts", started: "17:10", state: "working"),
        ])
        let martin = person("martin", "Martin", [])
        return try decode(TeamReply.self,
                          #"{"sharing":\#(sharing),"people":[\#([patrick, nico, martin, adrian].joined(separator: ","))]}"#)
    }

    /// Sessions on one branch are one row: working when any works, ordered by the latest start, with the newest pull
    /// request and the newest session's room. Teammates come in order of name and the viewer last; people with no
    /// live session are left out.
    func testTeamGroupsByBranch() throws {
        let groups = TeamList.groups(from: try reply().people, viewer: "patrick")
        XCTAssertEqual(groups.map(\.person.id), ["adrian", "nico", "patrick"])
        let nico = groups[1]
        XCTAssertEqual(nico.branches.map(\.name), ["store-feed", "fee-ledger", "cowboy", "extra-3", "extra-2",
                                                   "extra-1", "extra-0"])
        let ledger = nico.branches[1]
        XCTAssertTrue(ledger.isWorking)
        XCTAssertEqual(ledger.pullRequest?.number, 1806, "The newest posted")
        XCTAssertEqual(ledger.room?.roomId, "fee-ledger", "Only the older session is in a room")
        XCTAssertEqual(ledger.repo, "dashboard")
        XCTAssertFalse(nico.branches[0].isWorking, "Stopped at a prompt draws nothing")
        XCTAssertEqual(nico.branches[2].repo, "", "On main the name is the folder")
        XCTAssertTrue(groups[2].isViewer)
    }

    func testFiveAndTheMoreRow() throws {
        let team = TeamStore()
        team.use(accountID: "patrick")
        team.receive(try reply())
        let nico = try XCTUnwrap(team.groups.first { $0.person.id == "nico" })
        XCTAssertEqual(team.rows(of: nico).count, 5)
        XCTAssertEqual(team.hiddenCount(of: nico), 2)
        XCTAssertEqual(team.moreTitle(of: nico), "Show 2 more")
        team.toggleMore(of: nico)
        XCTAssertEqual(team.rows(of: nico).count, 7)
        XCTAssertEqual(team.moreTitle(of: nico), "Hide 2")
        team.use(accountID: "chad")
        XCTAssertEqual(team.expanded, [], "Not kept for another account")
    }

    func testSearchAndItsCount() throws {
        let team = TeamStore()
        team.use(accountID: "patrick")
        team.receive(try reply())
        XCTAssertEqual(team.countLine, "9 branches")
        team.query = "fee-ledger"
        XCTAssertEqual(team.shownGroups.flatMap(\.branches).map(\.name), ["fee-ledger"])
        XCTAssertEqual(team.countLine, "1 of 9 branches")
        team.query = "#480"
        XCTAssertEqual(team.shownGroups.map(\.person.id), ["adrian"])
        team.query = "1806"
        XCTAssertEqual(team.shownGroups.flatMap(\.branches).map(\.name), ["fee-ledger"])
        team.query = "Fee ledger 480"
        XCTAssertEqual(team.shownGroups.flatMap(\.branches).map(\.name), ["onboarding"], "A pull request's title")
        team.query = "nico"
        XCTAssertEqual(team.rows(of: try XCTUnwrap(team.shownGroups.first)).count, 7, "Every match, no more row")
        XCTAssertEqual(team.hiddenCount(of: try XCTUnwrap(team.shownGroups.first)), 0)
        team.query = "infra"
        XCTAssertEqual(team.countLine, "4 of 9 branches", "A repo")
        team.query = "zzz"
        XCTAssertEqual(team.shownGroups.count, 0)
        XCTAssertEqual(team.countLine, "0 of 9 branches")
        XCTAssertEqual(TeamList.count(shown: 1, total: 1, narrowed: false), "1 branch")
    }

    func testWhichRowsOpenARoomAndThePullRequestHelp() throws {
        let groups = TeamList.groups(from: try reply().people, viewer: "patrick")
        let ledger = try XCTUnwrap(groups[1].branches.first { $0.name == "fee-ledger" })
        XCTAssertTrue(TeamList.opensRoom(ledger, accountRoomIDs: ["fee-ledger"]))
        XCTAssertFalse(TeamList.opensRoom(ledger, accountRoomIDs: ["home-patrick"]), "Not one of his rooms")
        XCTAssertFalse(TeamList.opensRoom(groups[1].branches[0], accountRoomIDs: ["fee-ledger"]), "No room")
        XCTAssertEqual(TeamList.help(for: try XCTUnwrap(ledger.pullRequest)),
                       "dashboard #1806, Fee ledger 1806, checks running")
    }

    func testTheSharingSwitchComesFromTheServer() throws {
        let team = TeamStore()
        team.use(accountID: "patrick")
        XCTAssertNil(team.sharing, "Unknown until the server says")
        team.receive(try reply(sharing: false))
        XCTAssertEqual(team.sharing, false)
        team.receive(try reply(sharing: true))
        XCTAssertEqual(team.sharing, true)
    }
}
