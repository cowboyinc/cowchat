import XCTest
@testable import CowchatiOS
import CowchatKit

@MainActor
final class ChatSessionTests: XCTestCase {
    private var connection: FakeConnection!
    private var credentials: InMemoryCredentialStore!
    private var defaults: UserDefaults!
    private var suiteName: String!
    private var sleeps: [Duration] = []
    private var redeemedURLs: [URL] = []
    private var redeemedTokens: [String] = []
    private var redeemResult: Result<String, Error> = .success("redeemed-key")
    private var blobRequests: [URLRequest] = []
    private var blobStatus = 200

    private let tcp = ServerChoice.tcp(host: "192.168.1.20", port: 9229)
    private let global = ServerChoice.global(url: "wss://chat.example.test/ws")

    override func setUp() async throws {
        connection = FakeConnection()
        connection.serverRooms = [
            makeRoom(id: "r1", name: "General", lastActivity: "2026-11-18T10:00:00Z"),
            makeRoom(id: "r2", name: "Random", lastActivity: "2026-11-18T09:00:00Z"),
        ]
        credentials = InMemoryCredentialStore()
        suiteName = "ChatSessionTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
        sleeps = []
        redeemedURLs = []
        redeemedTokens = []
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: suiteName)
    }

    private func makeSession() -> ChatSession {
        ChatSession(
            record: ConnectionRecord(id: "c1", server: tcp, name: "Home", displayName: "Ryan's iPhone"),
            connection: connection,
            credentials: credentials,
            defaults: defaults,
            redeemInvite: { [unowned self] url, token in
                redeemedURLs.append(url)
                redeemedTokens.append(token)
                return try redeemResult.get()
            },
            sleep: { [unowned self] duration in sleeps.append(duration) },
            fetchBlob: { [unowned self] request in
                blobRequests.append(request)
                let response = HTTPURLResponse(url: request.url!, statusCode: blobStatus, httpVersion: nil, headerFields: nil)!
                return (Data("blob".utf8), response)
            }
        )
    }

    private func signedIn() async -> ChatSession {
        let session = makeSession()
        await session.signIn(server: tcp, credential: .key("k1"), displayName: "Ryan's iPhone")
        return session
    }

    // MARK: Sign in

    func testSignInConnectsRegistersAndListsRooms() async {
        let session = await signedIn()

        XCTAssertEqual(session.phase, .connected)
        XCTAssertEqual(session.displayName, "Ryan's iPhone")
        XCTAssertEqual(session.server, tcp)
        XCTAssertEqual(session.rooms.map(\.name), ["General", "Random"])
        XCTAssertEqual(connection.profile?.kind, .local)
        XCTAssertEqual(connection.profile?.localHost, "192.168.1.20")
        XCTAssertEqual(connection.profile?.apiKey, "k1")
        XCTAssertEqual(connection.registeredNames, ["Ryan's iPhone"])
    }

    func testSignInPersistsAStableAgentIDAcrossSessions() async {
        let first = await signedIn()
        first.signOut()
        _ = await signedIn()

        XCTAssertEqual(connection.registeredAgentIDs.count, 2)
        XCTAssertEqual(Set(connection.registeredAgentIDs).count, 1)
        XCTAssertTrue(connection.registeredAgentIDs[0].hasPrefix("cowchat-ios-"))
    }

    func testSignInFailureToReachServerIsActionable() async throws {
        connection.connectFailures = [CowchatConnectionError.timeout]
        let session = makeSession()
        await session.signIn(server: tcp, credential: .key("k1"), displayName: "Ryan")

        XCTAssertEqual(
            session.phase,
            .failed("Could not reach 192.168.1.20:9229. Check the address and your network, then try again.")
        )
        XCTAssertEqual(credentials.values, [:])
    }

    func testSignInRejectedKeyIsActionable() async throws {
        connection.registerError = CowchatConnectionError.server(message: "invalid api key", code: "unauthorized")
        let session = makeSession()
        await session.signIn(server: tcp, credential: .key("bad"), displayName: "Ryan")

        XCTAssertEqual(session.phase, .failed("The server refused this key (invalid api key). Check the key and try again."))
    }

    func testSignInWithInsecureGlobalURLFails() async {
        let session = makeSession()
        await session.signIn(server: .global(url: "ws://chat.example.test/ws"), credential: .key("k1"), displayName: "Ryan")

        XCTAssertEqual(session.phase, .failed("Cowchat Cloud requires a secure wss:// URL."))
    }

    func testInviteOnGlobalRedeemsOverHTTPSThenSignsInWithTheReturnedKey() async {
        let session = makeSession()
        await session.signIn(server: global, credential: .invite("cinv_abc"), displayName: "Ryan")

        XCTAssertEqual(session.phase, .connected)
        XCTAssertEqual(redeemedURLs, [URL(string: "https://chat.example.test/api/invites/redeem")!])
        XCTAssertEqual(connection.profile?.apiKey, "redeemed-key")
    }

    func testInviteIsFoundInsideThePastedConnectPrompt() async {
        let prompt = """
        Read https://cowchat.cowboy.inc/skills.txt and follow its agent collaboration instructions.
        Join the exact room “iphone-test” and start collaborating now.
        Server: wss://chat.example.test/ws
        Redeem this single-use invite for an API key: `curl -fsS -X POST https://chat.example.test/api/invites/redeem -H 'Content-Type: application/json' -d '{"token":"cinv_Ab3-x_9"}'`
        Use `--url wss://chat.example.test/ws --key <api_key>` on every command.
        """
        let session = makeSession()
        await session.signIn(server: global, credential: .invite(prompt), displayName: "Ryan")

        XCTAssertEqual(redeemedTokens, ["cinv_Ab3-x_9"])
        XCTAssertEqual(session.phase, .connected)
    }

    func testBareInviteTokenIsSentUnchanged() async {
        let session = makeSession()
        await session.signIn(server: global, credential: .invite("  cguest_invite_77aa\n"), displayName: "Ryan")
        XCTAssertEqual(redeemedTokens, ["cguest_invite_77aa"])
    }

    func testInviteOnTCPServerFailsClearly() async {
        let session = makeSession()
        await session.signIn(server: tcp, credential: .invite("cinv_abc"), displayName: "Ryan")

        XCTAssertEqual(
            session.phase,
            .failed("Invite links work only with the Global server. Sign in to this server with an API key.")
        )
        XCTAssertEqual(redeemedURLs, [])
    }

    func testRestoreSignsBackInFromPersistedState() async {
        _ = await signedIn()
        connection.disconnect()

        let relaunched = makeSession()
        await relaunched.restore()

        XCTAssertEqual(relaunched.phase, .connected)
        XCTAssertEqual(relaunched.displayName, "Ryan's iPhone")
        XCTAssertEqual(relaunched.server, tcp)
        XCTAssertEqual(connection.profile?.apiKey, "k1")
    }

    func testRestoreWhileOfflineKeepsRetryingInsteadOfFailing() async {
        _ = await signedIn()
        connection.disconnect()
        connection.connectFailures = [CowchatConnectionError.timeout, CowchatConnectionError.timeout]

        let relaunched = makeSession()
        await relaunched.restore()
        XCTAssertEqual(relaunched.phase, .reconnecting)

        let reconnected = await eventually { relaunched.phase == .connected }
        XCTAssertTrue(reconnected)
        XCTAssertEqual(sleeps, [.seconds(1)])
    }

    func testRestoreWithNothingPersistedStaysSignedOut() async {
        let session = makeSession()
        await session.restore()
        XCTAssertEqual(session.phase, .signedOut)
    }

    // MARK: Rooms and history

    func testOpenLoadsLatestHistoryAndMembers() async {
        connection.serverMessages["r1"] = (1...3).map { makeMessage(id: "m\($0)", seq: $0) }
        connection.serverAgents["r1"] = [AgentPresence(agentID: "agent-other", name: "Other")]
        let session = await signedIn()

        await session.open(roomID: "r1")

        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.id), ["m1", "m2", "m3"])
        XCTAssertEqual(session.timelines["r1"]?.hasEarlier, false)
        XCTAssertEqual(session.timelines["r1"]?.isLoading, false)
        XCTAssertEqual(session.members["r1"], [RoomMember(id: "agent-other", name: "Other")])
        XCTAssertEqual(connection.joinedRooms, ["r1"])
    }

    func testLoadEarlierPagesBackwardsUntilTheStart() async {
        connection.serverMessages["r1"] = (1...120).map { makeMessage(id: "m\($0)", seq: $0) }
        let session = await signedIn()

        await session.open(roomID: "r1")
        XCTAssertEqual(session.timelines["r1"]?.entries.first?.id, "m71")
        XCTAssertEqual(session.timelines["r1"]?.hasEarlier, true)

        await session.loadEarlier(roomID: "r1")
        XCTAssertEqual(session.timelines["r1"]?.entries.first?.id, "m21")
        XCTAssertEqual(session.timelines["r1"]?.hasEarlier, true)

        await session.loadEarlier(roomID: "r1")
        XCTAssertEqual(session.timelines["r1"]?.entries.count, 120)
        XCTAssertEqual(session.timelines["r1"]?.entries.first?.id, "m1")
        XCTAssertEqual(session.timelines["r1"]?.hasEarlier, false)
        XCTAssertEqual(session.timelines["r1"]?.isLoading, false)
    }

    func testLiveMessageAppendsToOpenRoomAndMarksClosedRoomUnread() async {
        let session = await signedIn()
        await session.open(roomID: "r1")

        connection.deliver(makeMessage(id: "live1", room: "r1", seq: 5))
        connection.deliver(makeMessage(id: "live2", room: "r2", seq: 6))

        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.id), ["live1"])
        XCTAssertNil(session.timelines["r2"])
        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.isUnread, false)
        XCTAssertEqual(session.rooms.first { $0.id == "r2" }?.isUnread, true)
    }

    func testOpeningAnUnreadRoomClearsItAndThatPersists() async {
        let session = await signedIn()
        connection.deliver(makeMessage(id: "live", room: "r2", seq: 6))
        await session.open(roomID: "r2")
        session.close(roomID: "r2")
        XCTAssertEqual(session.rooms.first { $0.id == "r2" }?.isUnread, false)

        connection.deliver(makeMessage(id: "live3", room: "r2", seq: 7))
        let relaunched = makeSession()
        await relaunched.restore()
        XCTAssertEqual(relaunched.rooms.first { $0.id == "r2" }?.isUnread, true)
        XCTAssertEqual(relaunched.rooms.first { $0.id == "r1" }?.isUnread, false)
    }

    func testThinkingFramesDoNotEnterTheTimeline() async {
        let session = await signedIn()
        await session.open(roomID: "r1")
        connection.deliver(makeMessage(id: "t", room: "r1", seq: 5, type: "thinking"))
        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.id), [])
    }

    // MARK: Sending

    func testSendMergesTheReturnedMessageAndTheEchoDoesNotDuplicate() async throws {
        let session = await signedIn()
        await session.open(roomID: "r1")

        try await session.send(roomID: "r1", text: "hello", mentions: ["agent-other"])
        let sent = try XCTUnwrap(connection.serverMessages["r1"]?.last)
        connection.onEvent?("message_received", messagePayload(sent))

        let entries = session.timelines["r1"]?.entries ?? []
        XCTAssertEqual(entries.map(\.body), [.text("hello")])
        XCTAssertEqual(entries.map(\.isMine), [true])
        XCTAssertEqual(connection.lastSentMentions, ["agent-other"])
    }

    func testSendEncryptsWhenTheRoomHasASecret() async throws {
        let session = await signedIn()
        try session.setSecret("s3cret", roomID: "r1")
        await session.open(roomID: "r1")

        try await session.send(roomID: "r1", text: "private", mentions: [])

        let wire = try XCTUnwrap(connection.serverMessages["r1"]?.last?.content)
        XCTAssertTrue(wire.hasPrefix("cow1:"))
        XCTAssertEqual(try RoomCrypto.decrypt(wire, secret: "s3cret", roomID: "r1"), "private")
        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.body), [.text("private")])
    }

    func testSendToAnEncryptedRoomWithoutASecretThrows() async throws {
        connection.serverRooms = [makeRoom(id: "r1", name: "Vault", encrypted: true)]
        let session = await signedIn()
        do {
            try await session.send(roomID: "r1", text: "x", mentions: [])
            XCTFail("expected a throw")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Add this room's secret before sending.")
        }
        XCTAssertEqual(connection.serverMessages["r1"], nil)
    }

    // MARK: Encryption

    func testEncryptedMessagesAreLockedUntilTheRightSecretIsSet() async throws {
        let sealed = try RoomCrypto.encrypt("the plan", secret: "right", roomID: "r1")
        connection.serverMessages["r1"] = [
            makeMessage(id: "m1", seq: 1, content: sealed),
            makeMessage(id: "m2", seq: 2, content: "plain"),
        ]
        let session = await signedIn()
        await session.open(roomID: "r1")

        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.body), [.locked, .text("plain")])
        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.isEncrypted, true)
        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.hasSecret, false)

        try session.setSecret("wrong", roomID: "r1")
        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.body), [.locked, .text("plain")])

        try session.setSecret("right", roomID: "r1")
        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.body), [.text("the plan"), .text("plain")])
        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.hasSecret, true)
    }

    func testAnEncryptedRoomFlagShowsWithoutAnyMessages() async {
        connection.serverRooms = [makeRoom(id: "r1", name: "Vault", encrypted: true)]
        let session = await signedIn()
        XCTAssertEqual(session.rooms.map(\.isEncrypted), [true])
        XCTAssertEqual(session.rooms.map(\.hasSecret), [false])
    }

    func testSecretSurvivesRelaunchFromKeychain() async throws {
        let sealed = try RoomCrypto.encrypt("kept", secret: "right", roomID: "r1")
        connection.serverMessages["r1"] = [makeMessage(id: "m1", seq: 1, content: sealed)]
        let session = await signedIn()
        try session.setSecret("right", roomID: "r1")

        let relaunched = makeSession()
        await relaunched.restore()
        await relaunched.open(roomID: "r1")
        XCTAssertEqual(relaunched.timelines["r1"]?.entries.map(\.body), [.text("kept")])
    }

    func testSendRefusesPlaintextWhenASavedRoomSecretCannotBeRead() async throws {
        let session = await signedIn()
        try session.setSecret("right", roomID: "r1")
        credentials.values = credentials.values.filter { !$0.key.contains("r1") }

        let relaunched = makeSession()
        await relaunched.restore()
        do {
            try await relaunched.send(roomID: "r1", text: "x", mentions: [])
            XCTFail("expected a throw")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Add this room's secret before sending.")
        }
        XCTAssertEqual(connection.serverMessages["r1"], nil)
    }

    func testEachKeyRegistersItsOwnAgentIdentity() async {
        let session = makeSession()
        await session.signIn(server: tcp, credential: .key("k1"), displayName: "Ryan")
        session.signOut()
        await session.signIn(server: tcp, credential: .key("k2"), displayName: "Ryan")
        session.signOut()
        await session.signIn(server: tcp, credential: .key("k2"), displayName: "Ryan")

        let ids = connection.registeredAgentIDs
        XCTAssertEqual(ids.count, 3)
        XCTAssertNotEqual(ids[0], ids[1])
        XCTAssertEqual(ids[1], ids[2])
    }

    // MARK: Reconnect

    func testDropReconnectsWithBackoffAndBackfillsLoadedRooms() async {
        connection.serverMessages["r1"] = [makeMessage(id: "m1", seq: 1)]
        let session = await signedIn()
        await session.open(roomID: "r1")

        connection.connectFailures = [
            CowchatConnectionError.timeout, CowchatConnectionError.timeout, CowchatConnectionError.timeout,
        ]
        connection.drop()
        XCTAssertEqual(session.phase, .reconnecting)
        connection.serverMessages["r1"]?.append(makeMessage(id: "m2", seq: 2))

        let reconnected = await eventually { session.phase == .connected }
        XCTAssertTrue(reconnected)
        XCTAssertEqual(sleeps, [.seconds(1), .seconds(2), .seconds(4)])
        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.id), ["m1", "m2"])
        XCTAssertEqual(connection.registeredAgentIDs.count, 2)
    }

    func testBackfillAfterALongGapPagesBackToTheLoadedTimeline() async {
        connection.serverMessages["r1"] = [makeMessage(id: "m1", seq: 1)]
        let session = await signedIn()
        await session.open(roomID: "r1")

        connection.drop()
        connection.serverMessages["r1"]? += (2...130).map { makeMessage(id: "m\($0)", seq: $0) }

        let reconnected = await eventually { session.phase == .connected }
        XCTAssertTrue(reconnected)
        XCTAssertEqual(session.timelines["r1"]?.entries.count, 130)
    }

    func testBecomingActiveAfterBackgroundReconnectsImmediatelyAndBackfills() async {
        let session = await signedIn()
        await session.open(roomID: "r1")

        session.sceneEnteredBackground()
        connection.serverMessages["r1"] = [makeMessage(id: "missed", seq: 9)]
        session.sceneBecameActive()

        let reconnected = await eventually { session.phase == .connected }
        XCTAssertTrue(reconnected)
        XCTAssertEqual(sleeps, [])
        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.id), ["missed"])
    }

    func testMessagesMissedWhileAwayMarkAClosedRoomUnread() async {
        let session = await signedIn()
        await session.open(roomID: "r1")
        session.close(roomID: "r1")

        session.sceneEnteredBackground()
        connection.serverMessages["r1"] = [makeMessage(id: "missed", seq: 9)]
        session.sceneBecameActive()

        let reconnected = await eventually { session.phase == .connected }
        XCTAssertTrue(reconnected)
        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.isUnread, true)
    }

    func testARoomWhoseLoadWasInterruptedStopsLoadingAfterReconnect() async {
        let session = await signedIn()
        connection.beforeHistory = { [unowned self] in
            connection.beforeHistory = nil
            connection.drop()
        }
        await session.open(roomID: "r1")

        let reconnected = await eventually { session.phase == .connected }
        XCTAssertTrue(reconnected)
        XCTAssertEqual(session.timelines["r1"]?.isLoading, false)
    }

    func testARoomThatCannotLoadStaysUnread() async {
        let session = await signedIn()
        connection.deliver(makeMessage(id: "live", room: "r2", seq: 6))
        connection.historyErrors["r2"] = CowchatConnectionError.server(message: "not a member", code: nil)

        await session.open(roomID: "r2")

        XCTAssertEqual(session.timelines["r2"]?.isLoading, false)
        XCTAssertEqual(session.rooms.first { $0.id == "r2" }?.isUnread, true)
    }

    func testARoomTheServerRefusesDoesNotStopTheReconnect() async {
        connection.serverMessages["r1"] = [makeMessage(id: "m1", seq: 1)]
        let session = await signedIn()
        await session.open(roomID: "r1")
        await session.open(roomID: "r2")

        connection.historyErrors["r2"] = CowchatConnectionError.server(message: "room not found", code: nil)
        connection.drop()
        connection.serverMessages["r1"]?.append(makeMessage(id: "m2", seq: 2))

        let reconnected = await eventually { session.phase == .connected }
        XCTAssertTrue(reconnected, "\(session.phase)")
        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.id), ["m1", "m2"])
    }

    func testSignOutDoesNotReconnectAndErasesCredentials() async throws {
        let session = await signedIn()
        try session.setSecret("s3cret", roomID: "r1")
        XCTAssertNotEqual(credentials.values, [:])

        session.signOut()
        connection.drop()
        try await Task.sleep(nanoseconds: 50_000_000)

        XCTAssertEqual(session.phase, .signedOut)
        XCTAssertEqual(connection.registeredAgentIDs.count, 1)
        XCTAssertEqual(credentials.values, [:])
        XCTAssertEqual(session.rooms, [])

        let relaunched = makeSession()
        await relaunched.restore()
        XCTAssertEqual(relaunched.phase, .signedOut)
    }

    // MARK: Room actions

    func testPeopleRefreshListsWhoIsConnectedNow() async {
        let session = await signedIn()
        await session.open(roomID: "r1")
        connection.serverAgents["r1"] = [AgentPresence(agentID: "agent-new", name: "New")]

        await session.refreshPeople(roomID: "r1")

        XCTAssertEqual(session.members["r1"], [RoomMember(id: "agent-new", name: "New")])
    }

    func testCreateInviteAsksForASingleUseInviteToThatRoom() async throws {
        let session = await signedIn()

        let token = try await session.createInvite(roomID: "r2")

        XCTAssertEqual(token, "cinv_made1")
        XCTAssertEqual(connection.createdInvites.map(\.roomID), ["r2"])
        XCTAssertEqual(connection.createdInvites.map(\.singleUse), [true])
    }

    func testOnlyTheRoomsCreatorCanRenameIt() async {
        let session = await signedIn()
        let me = connection.registeredAgentIDs.last
        connection.serverRooms = [
            makeRoom(id: "mine", name: "Mine", createdBy: me),
            makeRoom(id: "theirs", name: "Theirs", createdBy: "agent-other"),
            makeRoom(id: "lobby", name: "lobby", createdBy: me),
        ]
        await session.refreshRooms()

        XCTAssertTrue(session.canRename(roomID: "mine"))
        XCTAssertFalse(session.canRename(roomID: "theirs"))
        XCTAssertFalse(session.canRename(roomID: "lobby"))
    }

    func testRenameTrimsTheNameAndUpdatesTheRoomList() async throws {
        let session = await signedIn()

        try await session.rename(roomID: "r1", to: "  Planning \n")

        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.name, "Planning")
        XCTAssertEqual(connection.serverRooms.first { $0.id == "r1" }?.name, "Planning")
    }

    func testRenameRefusesAnEmptyNameAndReportsAServerRefusal() async throws {
        let session = await signedIn()
        do {
            try await session.rename(roomID: "r1", to: "   ")
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Enter a room name.")
        }
        connection.renameError = CowchatConnectionError.server(message: "Room name 'x' already taken", code: "room_name_taken")
        do {
            try await session.rename(roomID: "r1", to: "x")
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Room name 'x' already taken")
        }
        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.name, "General")
    }

    func testArchivingMarksTheRoomAndSurvivesRelaunch() async {
        let session = await signedIn()

        session.setArchived(true, roomID: "r2")
        XCTAssertEqual(session.rooms.filter(\.isArchived).map(\.id), ["r2"])

        let relaunched = makeSession()
        await relaunched.restore()
        XCTAssertEqual(relaunched.rooms.filter(\.isArchived).map(\.id), ["r2"])

        relaunched.setArchived(false, roomID: "r2")
        XCTAssertTrue(relaunched.rooms.filter(\.isArchived).isEmpty)
    }

    func testTheLobbyCannotBeArchived() async {
        connection.serverRooms.append(makeRoom(id: "lobby", name: "lobby"))
        let session = await signedIn()

        session.setArchived(true, roomID: "lobby")

        XCTAssertTrue(session.rooms.filter(\.isArchived).isEmpty)
    }

    func testCreateRoomMakesAPrivateRoomThatIsListedAndCanBeRenamed() async throws {
        let session = await signedIn()

        let roomID = try await session.createRoom(name: "  Planning \n")

        XCTAssertEqual(connection.createdRooms.map(\.name), ["Planning"])
        XCTAssertEqual(connection.createdRooms.map(\.isPublic), [false])
        XCTAssertEqual(session.rooms.first { $0.id == roomID }?.name, "Planning")
        XCTAssertTrue(session.canRename(roomID: roomID))
    }

    func testCreateRoomRefusesAnEmptyNameAndLobbyWithoutAskingTheServer() async throws {
        let session = await signedIn()
        for (name, message) in [("  ", "Enter a room name."), ("Lobby", "\u{201C}lobby\u{201D} is reserved. Choose another name.")] {
            do {
                _ = try await session.createRoom(name: name)
                XCTFail("expected an error for \(name)")
            } catch {
                XCTAssertEqual(error.localizedDescription, message)
            }
        }
        XCTAssertTrue(connection.createdRooms.isEmpty)
    }

    func testCreateRoomReportsANameThatIsTaken() async throws {
        let session = await signedIn()
        do {
            _ = try await session.createRoom(name: "General")
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error.localizedDescription, "A room named General already exists.")
        }
    }

    // MARK: Messages for people

    func testAgentMessagesNotMarkedForPeopleAreFlagged() async {
        connection.serverMessages["r1"] = [
            makeMessage(id: "person", seq: 1, senderKind: "person"),
            makeMessage(id: "quiet", seq: 2, senderKind: "agent"),
            makeMessage(id: "marked", seq: 3, senderKind: "agent", displayExternal: true),
            makeMessage(id: "old", seq: 4),
        ]
        let session = await signedIn()
        await session.open(roomID: "r1")

        XCTAssertEqual(session.timelines["r1"]?.entries.filter(\.isForPeople).map(\.id), ["person", "marked", "old"])
        XCTAssertEqual(session.timelines["r1"]?.entries.filter(\.isAgent).map(\.id), ["quiet", "marked"],
                       "only senders the server labelled as agents; an unlabelled sender is not marked")
    }

    func testAHiddenAgentMessageLeavesAClosedRoomReadUnlessShowAllIsOn() async {
        let session = await signedIn()

        connection.deliver(makeMessage(id: "q1", room: "r1", seq: 1, senderKind: "agent"))
        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.isUnread, false)

        session.setShowsAllMessages(true, roomID: "r1")
        connection.deliver(makeMessage(id: "q2", room: "r1", seq: 2, senderKind: "agent"))
        XCTAssertEqual(session.rooms.first { $0.id == "r1" }?.isUnread, true)
    }

    func testShowAllMessagesIsKeptPerRoomAcrossRelaunch() async {
        let session = await signedIn()
        session.setShowsAllMessages(true, roomID: "r1")
        XCTAssertEqual(session.showAllRoomIDs, ["r1"])

        let relaunched = makeSession()
        await relaunched.restore()
        XCTAssertEqual(relaunched.showAllRoomIDs, ["r1"])
        relaunched.setShowsAllMessages(false, roomID: "r1")
        XCTAssertTrue(relaunched.showAllRoomIDs.isEmpty)
    }

    // MARK: Thinking and presence

    func testAThinkingEventShowsAPulseUnderTheAgentsLastMessageUntilItSpeaks() async {
        connection.serverMessages["r1"] = [makeMessage(id: "m1", seq: 1, from: "agent-other")]
        let session = await signedIn()
        await session.open(roomID: "r1")

        connection.emit("thinking", messagePayload(makeMessage(id: "t1", seq: 2, from: "agent-other", content: "reading the diff", type: "thinking")))

        let anchors = session.pulseAnchors(roomID: "r1", now: Date())
        XCTAssertEqual(anchors.byMessageID["m1"]?.map(ThinkingPulsePresentation.label), ["Other: reading the diff"])
        XCTAssertEqual(session.timelines["r1"]?.entries.map(\.id), ["m1"])

        connection.deliver(makeMessage(id: "m2", seq: 3, from: "agent-other"))
        XCTAssertTrue(session.pulseAnchors(roomID: "r1", now: Date()).byMessageID.isEmpty)
    }

    func testAPresenceUpdateSetsTheMembersStatusAndPulseInOpenRooms() async {
        connection.serverAgents["r1"] = [AgentPresence(agentID: "agent-other", name: "Other")]
        let session = await signedIn()
        await session.open(roomID: "r1")

        connection.emit("presence_update", ["agent_id": "agent-other", "status": "working", "status_detail": "running tests", "progress": 40])

        XCTAssertEqual(session.members["r1"]?.first?.status, "working")
        XCTAssertEqual(
            session.pulseAnchors(roomID: "r1", now: Date()).unanchored.map(ThinkingPulsePresentation.label),
            ["Other: running tests · 40%"]
        )

        connection.emit("presence_update", ["agent_id": "agent-other", "status": "idle"])
        XCTAssertEqual(session.members["r1"]?.first?.status, "idle")
        XCTAssertTrue(session.pulseAnchors(roomID: "r1", now: Date()).unanchored.isEmpty)
    }

    // MARK: Live room list

    func testRoomEventsAddRenameAndRemoveRoomsWithoutARefresh() async {
        let session = await signedIn()

        connection.emit("room_created", room: makeRoom(id: "r3", name: "Fresh"))
        XCTAssertEqual(Set(session.rooms.map(\.name)), ["General", "Random", "Fresh"])

        connection.emit("room_updated", room: makeRoom(id: "r3", name: "Renamed"))
        XCTAssertEqual(session.rooms.first { $0.id == "r3" }?.name, "Renamed")

        await session.open(roomID: "r3")
        connection.emit("room_destroyed", ["room_id": "r3"])
        XCTAssertNil(session.rooms.first { $0.id == "r3" })
        XCTAssertNil(session.timelines["r3"])
    }

    // MARK: Search and joining

    func testMessageSearchFindsRoomsByTextAndSenderAndSkipsEncryptedRooms() async {
        connection.serverRooms.append(makeRoom(id: "r3", name: "Sealed", encrypted: true))
        connection.serverMessages["r1"] = [makeMessage(id: "a", room: "r1", seq: 1, content: "ship the Parser fix")]
        connection.serverMessages["r2"] = [makeMessage(id: "b", room: "r2", seq: 1, name: "Parsley", content: "hello")]
        connection.serverMessages["r3"] = [makeMessage(id: "c", room: "r3", seq: 1, content: "parser")]
        let session = await signedIn()

        let parser = await session.messageMatches(query: "parser")
        let sender = await session.messageMatches(query: "parsl")
        let nothing = await session.messageMatches(query: "  ")
        XCTAssertEqual(parser, ["r1"])
        XCTAssertEqual(sender, ["r2"])
        XCTAssertTrue(nothing.isEmpty)
    }

    func testJoiningARoomFromTheJoinableListAddsItToTheRooms() async throws {
        connection.joinableRooms = [makeRoom(id: "pub", name: "Public square")]
        let session = await signedIn()

        let joinable = await session.joinableRooms()
        XCTAssertEqual(joinable.map(\.name), ["Public square"])

        try await session.join(roomID: "pub")
        XCTAssertTrue(session.rooms.contains { $0.id == "pub" })
        XCTAssertEqual(connection.joinedRooms, ["pub"])
        let after = await session.joinableRooms()
        XCTAssertTrue(after.isEmpty)
    }

    // MARK: Message content

    func testAMessageCarriesItsPullRequestAndFiles() async {
        connection.serverMessages["r1"] = [
            makeMessage(id: "pr", seq: 1, extra: ["pull_request": [
                "repository": "cowboyinc/cowchat", "number": 12, "title": "Fold agent messages",
                "checks": "passing", "additions": 214, "deletions": 38, "address": "https://github.com/cowboyinc/cowchat/pull/12",
            ]]),
            makeMessage(id: "file", seq: 2, type: "file", extra: ["blob_id": "b-1", "name": "notes.txt", "size": 120]),
            makeMessage(id: "listed", seq: 3, extra: ["attachments": [["blob_id": "b-2", "name": "shot.png"]]]),
        ]
        let session = await signedIn()
        await session.open(roomID: "r1")
        let entries = session.timelines["r1"]?.entries ?? []

        XCTAssertEqual(entries[0].pullRequest?.name, "cowchat #12")
        XCTAssertEqual(entries[0].pullRequest?.checks, .passing)
        XCTAssertEqual(entries[1].attachments, [FileAttachment(blobID: "b-1", name: "notes.txt", size: 120)])
        XCTAssertEqual(entries[2].attachments.map(\.name), ["shot.png"])
        XCTAssertTrue(entries[2].attachments[0].isImage)
    }

    func testAChartMessageBecomesAChartBody() async {
        connection.serverMessages["r1"] = [
            makeMessage(id: "c", seq: 1, content: chartContent(title: "Latency")),
            makeMessage(id: "bad", seq: 2, content: "cowchat-chart:v1\n{\"title\":\"x\",\"png\":\"AAAA\"}"),
        ]
        let session = await signedIn()
        await session.open(roomID: "r1")
        let bodies = session.timelines["r1"]?.entries.map(\.body) ?? []

        XCTAssertEqual(bodies.first, .chart(title: "Latency", png: tinyPNG))
        if case .text = bodies.last {} else { XCTFail("a chart that does not read stays text") }
    }

    func testAttachmentsDownloadFromTheGlobalServerWithTheKey() async throws {
        let session = makeSession()
        await session.signIn(server: global, credential: .key("k1"), displayName: "Ryan")
        let file = FileAttachment(blobID: "b-1", name: "notes.txt", size: 4)

        let data = try await session.attachmentData(file, roomID: "r1")

        XCTAssertEqual(data, Data("blob".utf8))
        XCTAssertEqual(blobRequests.first?.url?.absoluteString, "https://chat.example.test/api/blobs/b-1")
        XCTAssertEqual(blobRequests.first?.value(forHTTPHeaderField: "x-cowchat-key"), "k1")

        blobStatus = 404
        do {
            _ = try await session.attachmentData(file, roomID: "r1")
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error.localizedDescription, "This file is no longer available.")
        }
    }

    func testAttachmentsAreUnavailableOnAPlainServerInEncryptedRoomsAndForOddIDs() async {
        let plain = await signedIn()
        XCTAssertNil(plain.attachmentRequest(FileAttachment(blobID: "b-1", name: "a", size: nil), roomID: "r1"))

        connection.serverRooms.append(makeRoom(id: "sealed", name: "Sealed", encrypted: true))
        let session = makeSession()
        await session.signIn(server: global, credential: .key("k1"), displayName: "Ryan")
        XCTAssertNil(session.attachmentRequest(FileAttachment(blobID: "b-1", name: "a", size: nil), roomID: "sealed"))
        XCTAssertNil(session.attachmentRequest(FileAttachment(blobID: "../keys", name: "a", size: nil), roomID: "r1"))
        XCTAssertNotNil(session.attachmentRequest(FileAttachment(blobID: "b-1", name: "a", size: nil), roomID: "r1"))
    }

    func testMentionsOfAnAccountAreFoundInUnencryptedRoomsAndNotFromThisPhone() async {
        connection.serverRooms.append(makeRoom(id: "r3", name: "Sealed", encrypted: true))
        connection.serverMessages["r1"] = [
            makeMessage(id: "hit", room: "r1", seq: 1, name: "Cal", content: "@ryan look", extra: ["mentions": ["me"]]),
            makeMessage(id: "other", room: "r1", seq: 2, extra: ["mentions": ["someone-else"]]),
            makeMessage(id: "plain", room: "r1", seq: 3),
        ]
        connection.serverMessages["r3"] = [makeMessage(id: "sealed", room: "r3", seq: 1, extra: ["mentions": ["me"]])]
        let session = await signedIn()

        let found = await session.mentions(of: "me")

        XCTAssertEqual(found.map(\.message.id), ["hit"])
        XCTAssertEqual(found.first?.roomName, "General")
    }
}
