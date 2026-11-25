import XCTest
@testable import CowchatiOS
import CowchatKit

@MainActor
final class FakeAccountAPI: AccountAPI {
    var account = SocialAccount(id: "me", handle: "ryan", displayName: "Ryan Chan")
    var wire: [InboxWireItem] = []
    var friendList: [SocialFriend] = []
    var requestList: [SocialFriendRequest] = []
    var roomList: [SocialRoom] = []
    var teamReply: TeamReply?
    var calls: [String] = []
    var failing: Set<String> = []
    /// Calls that fail as the network does, not as a server that answered.
    var unreachable: Set<String> = []
    var invitationList: [SocialRoomInvitation] = []

    private func record(_ call: String) throws {
        calls.append(call)
        if unreachable.contains(call.split(separator: " ").first.map(String.init) ?? call) { throw URLError(.notConnectedToInternet) }
        if failing.contains(call.split(separator: " ").first.map(String.init) ?? call) {
            throw CowchatConnectionError.server(message: "refused", code: nil)
        }
    }

    func me() async throws -> SocialMe { try record("me"); return SocialMe(account: account, identities: []) }
    func updateMe(displayName: String, handle: String) async throws -> SocialMe {
        try record("updateMe \(displayName) \(handle)")
        account = SocialAccount(id: "me", handle: handle, displayName: displayName)
        return SocialMe(account: account, identities: [])
    }
    func inbox() async throws -> [InboxWireItem] { try record("inbox"); return wire }
    func acceptInvitation(id: String) async throws { try record("accept \(id)") }
    func declineInvitation(id: String) async throws { try record("decline \(id)") }
    func friends() async throws -> [SocialFriend] { try record("friends"); return friendList }
    func friendRequests() async throws -> [SocialFriendRequest] { try record("requests"); return requestList }
    func addFriend(handle: String) async throws { try record("addFriend \(handle)") }
    func answerFriendRequest(id: String, accept: Bool) async throws { try record("answer \(id) \(accept)") }
    func removeFriend(id: String) async throws { try record("removeFriend \(id)") }
    func rooms() async throws -> [SocialRoom] { try record("rooms"); return roomList }
    var capabilityReply = SocialCapabilities(providers: ["google"], setupVersion: 1, handleAvailability: true, friendLinks: true, setupHandle: true)
    var setupReply: SocialSetup? = SocialSetup(agentId: "bot", agentName: "ryan_bot", agent: nil, botAdmitted: true)
    var takenHandles: Set<String> = []

    func capabilities() async throws -> SocialCapabilities { try record("capabilities"); return capabilityReply }
    func setup() async throws -> SocialSetup? { try record("setup"); return setupReply }
    func establishSetup(displayName: String, handle: String) async throws -> SocialSetup? {
        try record("establishSetup \(displayName) \(handle)")
        account = SocialAccount(id: "me", handle: handle, displayName: displayName)
        setupReply = SocialSetup(agentId: "bot", agentName: "\(handle)_bot", agent: nil, botAdmitted: true)
        return setupReply
    }
    func checkHandle(_ handle: String) async throws -> SocialHandleAvailability {
        try record("checkHandle \(handle)")
        return SocialHandleAvailability(handle: handle, available: !takenHandles.contains(handle), valid: handle.count >= 3)
    }
    func friendLink() async throws -> SocialFriendLink {
        try record("friendLink")
        return SocialFriendLink(slug: "ryan", url: URL(string: "https://chat.example.test/friends/ryan")!)
    }
    func previewFriendLink(slug: String) async throws -> SocialFriendLinkPreview {
        try record("previewFriendLink \(slug)")
        return SocialFriendLinkPreview(account: SocialAccount(id: "a2", handle: slug, displayName: "Cal Fisher"), relationship: .none)
    }
    func requestFriendLink(slug: String) async throws { try record("requestFriendLink \(slug)") }
    var participantReply: SocialParticipants?
    var ownAgents: [SocialAgent] = []

    func participants(roomID: String) async throws -> SocialParticipants {
        try record("participants \(roomID)")
        guard let participantReply else { throw CowchatConnectionError.server(message: "not found", code: nil) }
        return participantReply
    }
    func agents() async throws -> [SocialAgent] { try record("agents"); return ownAgents }
    func removeAgent(roomID: String, agentID: String) async throws { try record("removeAgent \(roomID) \(agentID)") }
    func addAgents(roomID: String, agentIDs: [String]) async throws { try record("addAgents \(roomID) \(agentIDs.joined(separator: ","))") }
    func createRoom(name: String, firstFriendID: String?) async throws -> String {
        try record("createRoom \(name) \(firstFriendID ?? "-")")
        return "new-room"
    }
    func invitations() async throws -> [SocialRoomInvitation] { try record("invitations"); return invitationList }
    func logout() async throws { try record("logout") }
    func leaveRoom(roomID: String) async throws { try record("leave \(roomID)") }
    func invite(friendID: String, roomID: String) async throws { try record("invite \(friendID) \(roomID)") }
    func team() async throws -> TeamReply {
        try record("team")
        guard let teamReply else { throw CowchatConnectionError.server(message: "no route", code: nil) }
        return teamReply
    }
}

func decodeSocial<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return try decoder.decode(type, from: Data(json.utf8))
}

@MainActor
final class AccountStoreTests: XCTestCase {
    private var api: FakeAccountAPI!
    private var defaults: UserDefaults!
    private var suiteName: String!

    private let inboxJSON = """
    {"items":[
      {"id":"i1","kind":"mention","room_id":"r1","room_name":"design","at":"2026-11-18T10:00:00Z",
       "content":"@ryan could you review the plan","sender":{"kind":"person","account":{"id":"a2","handle":"cal","display_name":"Cal Fisher"},"name":"Cal Fisher"}},
      {"id":"i2","kind":"marked","room_id":"r2","room_name":"cowchat-main","at":"2026-11-18T11:00:00Z",
       "content":"Tests pass.\\nReady for review.","sender":{"kind":"agent","account":{"id":"a2","handle":"cal","display_name":"Cal Fisher"},"agent_id":"s1","name":"cowchat-main"}},
      {"id":"i3","kind":"invitation","room_id":"r3","room_name":"launch","at":"2026-11-18T09:00:00Z",
       "invitation":{"id":"inv1","room_id":"r3","room_name":"launch","from":{"id":"a2","handle":"cal","display_name":"Cal Fisher"},
         "to":{"id":"me","handle":"ryan","display_name":"Ryan Chan"},"proposed_agents":[],"members":[],"agents":[],
         "history_policy":"full","encrypted":false,"slack_connected":false,"status":"pending"}},
      {"id":"i4","kind":"invitation","room_id":"r4","room_name":"old","at":"2026-11-18T12:00:00Z",
       "invitation":{"id":"inv2","room_id":"r4","room_name":"old","from":{"id":"a2","handle":"cal","display_name":"Cal Fisher"},
         "to":{"id":"me","handle":"ryan","display_name":"Ryan Chan"},"proposed_agents":[],"members":[],"agents":[],
         "history_policy":"full","encrypted":false,"slack_connected":false,"status":"declined"}}
    ],"next_cursor":null}
    """

    override func setUp() async throws {
        api = FakeAccountAPI()
        suiteName = "AccountStoreTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: suiteName)
    }

    private let endpoint = URL(string: "wss://chat.example.test/ws")!

    private func makeStore() -> AccountStore {
        AccountStore(connectionID: "c1", api: api, defaults: defaults, endpoint: endpoint)
    }

    private func loadInbox() throws { api.wire = try decodeSocial(InboxReply.self, inboxJSON).items }

    func testInboxListsOpenInvitationsFirstThenNewestAndDropsDeclinedOnes() async throws {
        try loadInbox()
        let store = makeStore()
        await store.refresh()

        XCTAssertEqual(store.inbox.map(\.id), ["i3", "i2", "i1"])
        XCTAssertEqual(store.inbox[0].message, "Invited you to join.")
        XCTAssertEqual(store.inbox[0].title, "Cal Fisher")
        XCTAssertTrue(store.inbox[0].isOpenInvitation)
        XCTAssertEqual(store.inbox[1].title, "Cal's cowchat-main")
        XCTAssertEqual(store.inbox[1].message, "Tests pass. Ready for review.")
        XCTAssertEqual(store.inbox[2].message, "could you review the plan")
    }

    func testReadMarksPersistAndMarkAllReadLeavesOpenInvitationsUnread() async throws {
        try loadInbox()
        let store = makeStore()
        await store.refresh()
        XCTAssertEqual(store.unreadCount, 3)

        store.markRead(store.inbox[2])
        XCTAssertEqual(store.unreadCount, 2)
        let relaunched = makeStore()
        await relaunched.refresh()
        XCTAssertEqual(relaunched.unreadCount, 2)

        relaunched.markAllRead()
        XCTAssertEqual(relaunched.inbox.filter(relaunched.isUnread).map(\.id), ["i3"])
        relaunched.markUnread(relaunched.inbox[1])
        XCTAssertEqual(relaunched.unreadCount, 2)
    }

    func testAcceptingAndDecliningAnInvitationTellTheServer() async throws {
        try loadInbox()
        let store = makeStore()
        await store.refresh()
        let invitation = store.inbox[0]

        let accepted = await store.accept(invitation)
        XCTAssertTrue(accepted)
        XCTAssertTrue(api.calls.contains("accept inv1"))
        XCTAssertFalse(store.isUnread(invitation))

        api.wire.removeAll { $0.id == "i3" }
        await store.decline(invitation)
        XCTAssertTrue(api.calls.contains("decline inv1"))
        XCTAssertFalse(store.inbox.contains { $0.id == "i3" })
    }

    func testARefusedAnswerReportsAndChangesNothing() async throws {
        try loadInbox()
        api.failing = ["accept"]
        let store = makeStore()
        await store.refresh()

        let accepted = await store.accept(store.inbox[0])

        XCTAssertFalse(accepted)
        XCTAssertEqual(store.errorMessage, "refused")
        XCTAssertTrue(store.isUnread(store.inbox[0]))
    }

    func testFriendRequestsSplitIntoIncomingAndSent() async throws {
        let me = api.account
        let cal = SocialAccount(id: "a2", handle: "cal", displayName: "Cal Fisher")
        api.requestList = [
            SocialFriendRequest(id: "q1", from: cal, to: me, status: "pending"),
            SocialFriendRequest(id: "q2", from: me, to: cal, status: "pending"),
            SocialFriendRequest(id: "q3", from: cal, to: me, status: "declined"),
        ]
        api.friendList = [SocialFriend(account: cal, agents: [])]
        let store = makeStore()
        await store.refresh()

        XCTAssertEqual(store.incomingRequests.map(\.id), ["q1"])
        XCTAssertEqual(store.sentRequests.map(\.id), ["q2"])
        XCTAssertEqual(store.friends.map(\.id), ["a2"])

        await store.answer(store.incomingRequests[0], accept: true)
        await store.addFriend(handle: "@Sam")
        await store.removeFriend(store.friends[0])
        XCTAssertTrue(api.calls.contains("answer q1 true"))
        XCTAssertTrue(api.calls.contains("addFriend @Sam"))
        XCTAssertTrue(api.calls.contains("removeFriend a2"))
    }

    func testSavingTheProfileUpdatesTheAccount() async {
        let store = makeStore()
        await store.refresh()

        let saved = await store.saveProfile(displayName: "Ryan C", handle: "ryanc")

        XCTAssertTrue(saved)
        XCTAssertEqual(store.account, SocialAccount(id: "me", handle: "ryanc", displayName: "Ryan C"))
    }

    func testAServerWithoutOneRouteStillShowsTheRest() async throws {
        try loadInbox()
        api.failing = ["friends"]
        api.teamReply = nil
        let store = makeStore()
        await store.refresh()

        XCTAssertEqual(store.inbox.count, 3)
        XCTAssertTrue(store.team.isEmpty)
        XCTAssertNil(store.errorMessage)
    }

    func testAPartThatFailedToLoadIsMarkedAndClearsWhenItLoads() async throws {
        try loadInbox()
        api.failing = ["inbox", "invitations", "requests"]
        api.teamReply = nil
        let store = makeStore()
        await store.refresh()

        XCTAssertEqual(store.failedParts, [.inbox, .friends, .team])
        XCTAssertTrue(store.inbox.isEmpty)

        api.failing = []
        api.teamReply = TeamReply(sharing: false, people: [])
        await store.refresh()
        XCTAssertTrue(store.failedParts.isEmpty)
        XCTAssertEqual(store.inbox.count, 3)
    }

    func testAFailedReloadKeepsWhatWasAlreadyShown() async throws {
        try loadInbox()
        let store = makeStore()
        await store.refresh()
        api.unreachable = ["inbox"]
        await store.refresh()

        XCTAssertEqual(store.inbox.count, 3)
        XCTAssertTrue(store.failedParts.contains(.inbox))
        XCTAssertFalse(api.calls.contains("invitations"), "a network failure is not a missing route")
    }

    func testTeamAndSessionRoomsDecodeAsTheServerSendsThem() async throws {
        api.teamReply = try decodeSocial(TeamReply.self, """
        {"sharing":true,"people":[
          {"account":{"id":"a2","handle":"cal","display_name":"Cal Fisher"},"sessions":[
            {"id":"s1","folder":"cowchat","branch":"feature/inbox","state":"prompt","started_at":"2026-11-18T08:00:00Z",
             "room":{"room_id":"r2","name":"cowchat-inbox"},
             "pull_request":{"repository":"cowboyinc/cowchat","number":7,"title":"Inbox","checks":"running","address":"https://github.com/cowboyinc/cowchat/pull/7"}}]},
          {"account":{"id":"a3","handle":"sam","display_name":"Sam"},"sessions":[]}
        ]}
        """)
        api.roomList = try decodeSocial(SocialRooms.self, """
        {"rooms":[{"room_id":"r2","name":"cowchat-inbox","owner_id":"me","kind":"session",
          "session":{"agent_id":"s1","harness":"claude-code","folder":"cowchat","branch":"feature/inbox","started_at":"2026-11-18T08:00:00Z","ended_at":null}},
          {"room_id":"r9","name":"friends","owner_id":"me"}]}
        """).rooms
        let store = makeStore()
        await store.refresh()

        XCTAssertEqual(store.team.map(\.account.handle), ["cal"])
        XCTAssertEqual(store.team[0].sessions[0].name, "feature/inbox")
        XCTAssertEqual(store.team[0].sessions[0].stateLabel, "Waiting")
        XCTAssertEqual(store.team[0].sessions[0].pullRequest?.name, "cowchat #7")
        XCTAssertEqual(Array(store.sessionRooms.keys), ["r2"])
    }

    func testOnlyRoomsSomeoneElseOwnsCanBeLeftAndOnlyOwnRoomsTakeInvitations() async throws {
        api.roomList = [
            SocialRoom(roomId: "mine", name: "Mine", ownerId: "me"),
            SocialRoom(roomId: "theirs", name: "Theirs", ownerId: "a2"),
        ]
        let store = makeStore()
        await store.refresh()

        XCTAssertTrue(store.canLeave(roomID: "theirs"))
        XCTAssertFalse(store.canLeave(roomID: "mine"))
        XCTAssertFalse(store.canLeave(roomID: "unknown"))
        XCTAssertTrue(store.canInvite(roomID: "mine"))
        XCTAssertFalse(store.canInvite(roomID: "theirs"))

        let left = await store.leave(roomID: "theirs")
        let invited = await store.invite(friendID: "a2", roomID: "mine")
        XCTAssertTrue(left)
        XCTAssertTrue(invited)
        XCTAssertTrue(api.calls.contains("leave theirs"))
        XCTAssertTrue(api.calls.contains("invite a2 mine"))
    }

    func testSigningOutRevokesTheKeyOnTheServer() async {
        let store = makeStore()
        await store.signOut()
        XCTAssertEqual(api.calls, ["logout"])
    }

    // MARK: Mentions from other connections

    func testMentionsFromPlainKeyRoomsJoinTheInboxInOrder() async throws {
        try loadInbox()
        let store = makeStore()
        store.setCommunity([InboxEntry(
            id: "community.r9.m1", title: "pinger", roomID: "r9", roomName: "junction",
            at: CowchatDate.parse("2026-11-18T10:30:00Z")!, message: "look at this", connectionID: "plain"
        )], from: ["plain"])
        await store.refresh()

        XCTAssertEqual(store.inbox.map(\.id), ["i3", "i2", "community.r9.m1", "i1"])
        XCTAssertEqual(store.inbox[2].connectionID, "plain")
    }

    // MARK: First-run setup

    func testANewAccountNeedsSetupUntilItHasANameAndHandle() async {
        api.setupReply = nil
        let store = makeStore()
        await store.refresh()
        XCTAssertTrue(store.needsSetup)

        let done = await store.establishSetup(displayName: " Ryan Chan ", handle: "@Ryan")

        XCTAssertTrue(done)
        XCTAssertTrue(api.calls.contains("establishSetup Ryan Chan Ryan"), "the @ is dropped; the server decides case")
        XCTAssertFalse(store.needsSetup)
        XCTAssertEqual(store.account?.handle, "Ryan")
    }

    func testAnAccountThatIsSetUpOrAServerWithoutSetupNeedsNone() async {
        let ready = makeStore()
        await ready.refresh()
        XCTAssertFalse(ready.needsSetup)

        api.setupReply = nil
        api.capabilityReply = SocialCapabilities(providers: [], setupVersion: nil, handleAvailability: nil, friendLinks: nil, setupHandle: nil)
        let old = makeStore()
        await old.refresh()
        XCTAssertFalse(old.needsSetup)
    }

    func testCheckingAHandleSaysWhetherItCanBeUsed() async {
        api.takenHandles = ["cal"]
        let store = makeStore()
        await store.refresh()

        let taken = await store.handleCheck("@cal")
        let short = await store.handleCheck("ab")
        let free = await store.handleCheck("ryanc")
        XCTAssertEqual(taken, .unavailable("cal"))
        XCTAssertEqual(short, .invalid("ab"))
        XCTAssertEqual(free, .available("ryanc"))
    }

    // MARK: Friend links

    func testFriendLinksParseForBothFormsAndRefuseAnythingElse() {
        XCTAssertEqual(FriendLinkRoute.parse(URL(string: "cowchat://friend/cal?server=wss://chat.example.test/ws")!),
                       FriendLinkRoute(slug: "cal", endpoint: endpoint))
        XCTAssertEqual(FriendLinkRoute.parse(URL(string: "https://Chat.Example.test/friends/cal")!),
                       FriendLinkRoute(slug: "cal", endpoint: endpoint))
        XCTAssertEqual(FriendLinkRoute.parse(URL(string: "cowchat://friend/cal")!)?.endpoint.absoluteString,
                       ConnectionProfile.defaultGlobalURLString)
        for bad in ["cowchat://room/cal", "https://chat.example.test/friends/cal?x=1", "https://chat.example.test/friends/a/b",
                    "cowchat://friend/ab", "cowchat://friend/cal?server=ws://chat.example.test/ws", "http://chat.example.test/friends/cal"] {
            XCTAssertNil(FriendLinkRoute.parse(URL(string: bad)!), bad)
        }
    }

    func testAFriendLinkIsPreviewedAndRequestedOnlyOnItsOwnServer() async {
        let store = makeStore()
        await store.refresh()
        let here = FriendLinkRoute(slug: "cal", endpoint: endpoint)
        let elsewhere = FriendLinkRoute(slug: "cal", endpoint: URL(string: "wss://other.example.test/ws")!)

        XCTAssertTrue(store.serves(here))
        XCTAssertFalse(store.serves(elsewhere))
        let preview = await store.previewFriend(here)
        XCTAssertEqual(preview?.account.displayName, "Cal Fisher")
        let nothing = await store.previewFriend(elsewhere)
        XCTAssertNil(nothing)
        let sent = await store.requestFriend(here)
        XCTAssertTrue(sent)
        XCTAssertTrue(api.calls.contains("requestFriendLink cal"))

        let mine = await store.myFriendLink()
        XCTAssertEqual(mine?.absoluteString, "https://chat.example.test/friends/ryan")
    }

    // MARK: What is new since the last look

    func testTheFirstRefreshReportsNothingNewAndLaterOnesReportEachUnreadItemOnce() async throws {
        let store = makeStore()
        try loadInbox()
        let first = await store.refresh()
        XCTAssertTrue(first.isEmpty, "what was already there is not news")

        api.wire += try decodeSocial(InboxReply.self, """
        {"items":[{"id":"i9","kind":"mention","room_id":"r1","room_name":"design","at":"2026-11-18T13:00:00Z",
          "content":"@ryan ship it","sender":{"kind":"person","account":{"id":"a2","handle":"cal","display_name":"Cal Fisher"},"name":"Cal Fisher"}}]}
        """).items
        let second = await store.refresh()
        let third = await store.refresh()
        XCTAssertEqual(second.map(\.id), ["i9"])
        XCTAssertTrue(third.isEmpty)

        let relaunched = makeStore()
        let again = await relaunched.refresh()
        XCTAssertTrue(again.isEmpty, "the ledger survives a relaunch")
    }

    // MARK: People and agents in an account room

    func testAnAccountRoomListsItsPeopleAndAgentsAndSaysWhichAgentsCanBeTakenOut() async {
        let cal = SocialAccount(id: "a2", handle: "cal", displayName: "Cal Fisher")
        let mine = SocialAgent(id: "g1", ownerId: "me", name: "ryan_bot", discoverable: false)
        let theirs = SocialAgent(id: "g2", ownerId: "a2", name: "chad_bot", discoverable: false)
        let spare = SocialAgent(id: "g3", ownerId: "me", name: "ryan_helper", discoverable: false)
        api.ownAgents = [mine, spare]
        api.participantReply = SocialParticipants(roomId: "r1", name: "design", ownerId: "a2", historyPolicy: "full",
                                                  encrypted: false, slackConnected: false,
                                                  members: [api.account, cal], agents: [mine, theirs])
        let store = makeStore()
        await store.refresh()

        let people = await store.participants(roomID: "r1")

        XCTAssertEqual(people?.members.map(\.handle), ["ryan", "cal"])
        XCTAssertEqual(people.map { room in room.agents.filter { store.canRemove($0, from: room) }.map(\.name) }, ["ryan_bot"])
        XCTAssertEqual(people.map { store.agentsToAdd(to: $0).map(\.name) }, ["ryan_helper"])

        await store.removeAgent("g1", roomID: "r1")
        await store.addAgents(["g3"], roomID: "r1")
        XCTAssertTrue(api.calls.contains("removeAgent r1 g1"))
        XCTAssertTrue(api.calls.contains("addAgents r1 g3"))
    }

    func testTheOwnerOfARoomCanTakeOutAnyAgent() async {
        let theirs = SocialAgent(id: "g2", ownerId: "a2", name: "chad_bot", discoverable: false)
        let room = SocialParticipants(roomId: "r1", name: "design", ownerId: "me", historyPolicy: "full",
                                      encrypted: false, slackConnected: false, members: [], agents: [theirs])
        let store = makeStore()
        await store.refresh()
        XCTAssertTrue(store.canRemove(theirs, from: room))
    }

    // MARK: A room with friends

    func testARoomWithFriendsNamesTheFirstFriendAndInvitesTheRest() async {
        let store = makeStore()
        await store.refresh()

        let roomID = await store.createRoom(name: "  launch ", friendIDs: ["a2", "a3", "a4"])

        XCTAssertEqual(roomID, "new-room")
        XCTAssertTrue(api.calls.contains("createRoom launch a2"))
        XCTAssertTrue(api.calls.contains("invite a3 new-room"))
        XCTAssertTrue(api.calls.contains("invite a4 new-room"))

        let alone = await store.createRoom(name: "solo", friendIDs: [])
        XCTAssertEqual(alone, "new-room")
        XCTAssertTrue(api.calls.contains("createRoom solo -"))
    }

    func testAServerWithoutTheInboxRouteStillShowsInvitationsToThePerson() async throws {
        let cal = SocialAccount(id: "a2", handle: "cal", displayName: "Cal Fisher")
        func invitation(_ id: String, from: SocialAccount, to: SocialAccount, status: String) -> SocialRoomInvitation {
            SocialRoomInvitation(id: id, roomId: "r-\(id)", roomName: "room \(id)", from: from, to: to, proposedAgents: [],
                                 members: [], agents: [], historyPolicy: "full", encrypted: false, slackConnected: false, status: status)
        }
        api.failing = ["inbox"]
        api.invitationList = [
            invitation("in", from: cal, to: api.account, status: "pending"),
            invitation("out", from: api.account, to: cal, status: "pending"),
        ]
        let store = makeStore()
        await store.refresh()

        XCTAssertFalse(store.failedParts.contains(.inbox))
        XCTAssertEqual(store.inbox.map(\.invitationID), ["in"], "only invitations sent to the person")
        XCTAssertEqual(store.inbox.first?.title, "Cal Fisher")
        XCTAssertEqual(store.inbox.first?.roomName, "room in")
        XCTAssertTrue(store.inbox.first?.isOpenInvitation == true)

        let accepted = await store.accept(store.inbox[0])
        XCTAssertTrue(accepted)
        XCTAssertTrue(api.calls.contains("accept in"))
    }
}
