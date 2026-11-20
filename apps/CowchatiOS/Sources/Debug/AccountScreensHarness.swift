#if DEBUG
import SwiftUI
import CowchatKit

/// Debug builds only, shown when the app is launched with `-accountScreens`: the account screens over a made-up
/// account held in memory, so a UI test can drive them without a Google sign-in. It makes no network calls.
struct AccountScreensHarness: View {
    @StateObject private var account: AccountStore = {
        // Each launch starts with nothing read.
        let suite = "CowchatiOS.accountScreens"
        UserDefaults().removePersistentDomain(forName: suite)
        return AccountStore(connectionID: "demo", api: DemoAccountAPI(), defaults: UserDefaults(suiteName: suite)!,
                            endpoint: URL(string: "wss://chat.example.test/ws"))
    }()
    @State private var sheet: AccountSheetKind?
    @State private var opened = ""
    @State private var showsSetup = false
    @State private var friendLink: FriendLinkRoute?
    @State private var showsPeople = false

    var body: some View {
        VStack(spacing: 16) {
            ForEach([AccountSheetKind.inbox, .friends, .team, .profile]) { kind in
                Button(kind.rawValue.capitalized) { sheet = kind }.accessibilityIdentifier("harness.\(kind.rawValue)")
            }
            Button("Setup") { showsSetup = true }.accessibilityIdentifier("harness.setup")
            Button("Friend link") {
                friendLink = FriendLinkRoute.parse(URL(string: "https://chat.example.test/friends/cal")!)
            }
            .accessibilityIdentifier("harness.friendLink")
            Button("Room people") { showsPeople = true }.accessibilityIdentifier("harness.people")
            Text(opened).accessibilityIdentifier("harness.opened")
        }
        .sheet(isPresented: $showsPeople) {
            NavigationStack {
                List { AccountRoomPeople(account: account, roomID: "r1") }
                    .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { showsPeople = false } } }
            }
        }
        .sheet(isPresented: $showsSetup) { SetupSheet(account: account) }
        .sheet(item: $friendLink) { FriendLinkSheet(route: $0, accounts: [account]) }
        .task { await account.refresh() }
        .sheet(item: $sheet) { kind in
            switch kind {
            case .inbox: InboxView(accounts: [account]) { opened = $0.roomID }
            case .friends: FriendsView(accounts: [account])
            case .team: TeamView(accounts: [account]) { opened = $0.roomID }
            case .profile: ProfileView(accounts: [account])
            }
        }
    }
}

final class DemoAccountAPI: AccountAPI {
    private let me = SocialAccount(id: "me", handle: "ryan", displayName: "Ryan Chan")
    private let cal = SocialAccount(id: "a2", handle: "cal", displayName: "Cal Fisher")
    private var profile: SocialAccount?
    private var invitationOpen = true
    private var friendIDs: Set<String> = []
    private var requestOpen = true
    private var sent: [String] = []
    private var botInRoom = true
    private let bot = SocialAgent(id: "g1", ownerId: "me", name: "ryan_bot", discoverable: false)

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) -> T {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try! decoder.decode(type, from: Data(json.utf8))
    }

    func me() async throws -> SocialMe { SocialMe(account: profile ?? me, identities: []) }

    func updateMe(displayName: String, handle: String) async throws -> SocialMe {
        profile = SocialAccount(id: "me", handle: handle, displayName: displayName)
        return SocialMe(account: profile!, identities: [])
    }

    func inbox() async throws -> [InboxWireItem] {
        let person = #"{"id":"a2","handle":"cal","display_name":"Cal Fisher"}"#
        let invitation = invitationOpen ? """
        ,{"id":"i3","kind":"invitation","room_id":"r3","room_name":"launch","at":"2026-11-18T09:00:00Z",
          "invitation":{"id":"inv1","room_id":"r3","room_name":"launch","from":\(person),
          "to":{"id":"me","handle":"ryan","display_name":"Ryan Chan"},"proposed_agents":[],"members":[],"agents":[],
          "history_policy":"full","encrypted":false,"slack_connected":false,"status":"pending"}}
        """ : ""
        return decode(InboxReply.self, """
        {"items":[
          {"id":"i1","kind":"mention","room_id":"r1","room_name":"design","at":"2026-11-18T10:00:00Z",
           "content":"@ryan could you review the plan","sender":{"kind":"person","account":\(person),"name":"Cal Fisher"}},
          {"id":"i2","kind":"marked","room_id":"r2","room_name":"cowchat-main","at":"2026-11-18T11:00:00Z",
           "content":"Tests pass. Ready for review.","sender":{"kind":"agent","account":\(person),"agent_id":"s1","name":"cowchat-main"}}
          \(invitation)
        ]}
        """).items
    }

    func acceptInvitation(id: String) async throws { invitationOpen = false }
    func declineInvitation(id: String) async throws { invitationOpen = false }

    func friends() async throws -> [SocialFriend] {
        friendIDs.contains("a2") ? [SocialFriend(account: cal, agents: [])] : []
    }

    func friendRequests() async throws -> [SocialFriendRequest] {
        (requestOpen ? [SocialFriendRequest(id: "q1", from: cal, to: me, status: "pending")] : [])
            + sent.map { SocialFriendRequest(id: "s-\($0)", from: me, to: SocialAccount(id: $0, handle: $0, displayName: $0), status: "pending") }
    }

    func addFriend(handle: String) async throws {
        if handle == "nobody" { throw CowchatConnectionError.server(message: "not found", code: nil) }
        sent.append(SocialAPI.normalizedHandle(handle))
    }

    func answerFriendRequest(id: String, accept: Bool) async throws {
        requestOpen = false
        if accept { friendIDs.insert("a2") }
    }

    func removeFriend(id: String) async throws { friendIDs.remove(id) }

    func rooms() async throws -> [SocialRoom] {
        [SocialRoom(roomId: "r1", name: "design", ownerId: "a2"), SocialRoom(roomId: "r2", name: "cowchat-main", ownerId: "me")]
    }
    func capabilities() async throws -> SocialCapabilities {
        SocialCapabilities(providers: [], setupVersion: nil, handleAvailability: nil, friendLinks: nil, setupHandle: nil)
    }
    func setup() async throws -> SocialSetup? { nil }
    func establishSetup(displayName: String, handle: String) async throws -> SocialSetup? {
        profile = SocialAccount(id: "me", handle: handle, displayName: displayName)
        return SocialSetup(agentId: "bot", agentName: "\(handle)_bot", agent: nil, botAdmitted: true)
    }
    func checkHandle(_ handle: String) async throws -> SocialHandleAvailability {
        SocialHandleAvailability(handle: handle, available: handle != "cal", valid: handle.count >= 3)
    }
    func friendLink() async throws -> SocialFriendLink {
        SocialFriendLink(slug: "ryan", url: URL(string: "https://chat.example.test/friends/ryan")!)
    }
    func previewFriendLink(slug: String) async throws -> SocialFriendLinkPreview {
        SocialFriendLinkPreview(account: cal, relationship: .none)
    }
    func requestFriendLink(slug: String) async throws {}
    func participants(roomID: String) async throws -> SocialParticipants {
        SocialParticipants(roomId: roomID, name: "design", ownerId: "a2", historyPolicy: "full", encrypted: false,
                           slackConnected: false, members: [profile ?? me, cal],
                           agents: botInRoom ? [bot, SocialAgent(id: "g2", ownerId: "a2", name: "chad_bot", discoverable: false)]
                               : [SocialAgent(id: "g2", ownerId: "a2", name: "chad_bot", discoverable: false)])
    }
    func agents() async throws -> [SocialAgent] { [bot] }
    func removeAgent(roomID: String, agentID: String) async throws { botInRoom = false }
    func addAgents(roomID: String, agentIDs: [String]) async throws { botInRoom = true }
    func createRoom(name: String, firstFriendID: String?) async throws -> String { "r-new" }
    func invitations() async throws -> [SocialRoomInvitation] { [] }
    func logout() async throws {}
    func leaveRoom(roomID: String) async throws {}
    func invite(friendID: String, roomID: String) async throws {}

    func team() async throws -> TeamReply {
        decode(TeamReply.self, """
        {"sharing":true,"people":[{"account":{"id":"a2","handle":"cal","display_name":"Cal Fisher"},"sessions":[
          {"id":"s1","folder":"cowchat","branch":"feature/inbox","state":"prompt","started_at":"2026-11-18T08:00:00Z",
           "room":{"room_id":"r2","name":"cowchat-inbox"},
           "pull_request":{"repository":"cowboyinc/cowchat","number":7,"title":"Inbox","checks":"running","address":"https://github.com/cowboyinc/cowchat/pull/7"}}]}]}
        """)
    }
}
#endif
