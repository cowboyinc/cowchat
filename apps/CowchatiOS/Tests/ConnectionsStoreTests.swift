import XCTest
@testable import CowchatiOS
import CowchatKit

@MainActor
final class FakeNotifier: InboxNotifying {
    var allows = true
    var posted: [String] = []
    var badges: [Int] = []

    func requestPermission() async -> Bool { allows }
    func post(id: String, title: String, body: String) { posted.append("\(title): \(body)") }
    func setBadge(_ count: Int) { badges.append(count) }
}

@MainActor
final class ConnectionsStoreTests: XCTestCase {
    private var upcoming: [FakeConnection] = []
    private var made: [FakeConnection] = []
    private var credentials: InMemoryCredentialStore!
    private var defaults: UserDefaults!
    private var suiteName: String!
    private var redeemedTokens: [String] = []
    private var accountAPIRequests: [String] = []
    private var accountAPIs: [FakeAccountAPI] = []
    private var accountRooms: [SocialRoom] = []
    private var accountUnreachable: Set<String> = []
    private let notifier = FakeNotifier()
    private var redeemResult: Result<String, Error> = .success("minted-key")

    private let home = ServerChoice.tcp(host: "192.168.1.20", port: 9229)
    private let global = ServerChoice.global(url: "wss://chat.example.test/ws")

    override func setUp() async throws {
        upcoming = []
        made = []
        credentials = InMemoryCredentialStore()
        suiteName = "ConnectionsStoreTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
        redeemedTokens = []
        redeemResult = .success("minted-key")
        accountUnreachable = []
        accountAPIs = []
        accountAPIRequests = []
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: suiteName)
    }

    private func makeStore() -> ConnectionsStore {
        ConnectionsStore(
            makeConnection: { [unowned self] in
                let connection = upcoming.isEmpty ? FakeConnection() : upcoming.removeFirst()
                made.append(connection)
                return connection
            },
            credentials: credentials,
            defaults: defaults,
            redeemInvite: { [unowned self] _, token in
                redeemedTokens.append(token)
                return try redeemResult.get()
            },
            sleep: { _ in try await Task.sleep(nanoseconds: 1_000_000) },
            makeAccountAPI: { [unowned self] endpoint, key in
                accountAPIRequests.append("\(endpoint.host ?? "") \(key)")
                let api = FakeAccountAPI()
                api.roomList = accountRooms
                api.unreachable = accountUnreachable
                accountAPIs.append(api)
                return api
            },
            notifier: notifier
        )
    }

    private func fake(_ rooms: Room...) -> FakeConnection {
        let connection = FakeConnection()
        connection.serverRooms = rooms
        return connection
    }

    private func addedID(_ outcome: AddOutcome) throws -> String {
        guard case .addedConnection(let id) = outcome else {
            XCTFail("expected a new connection, got \(outcome)")
            throw FakeError(message: "unexpected outcome")
        }
        return id
    }

    private func message(of error: Error) -> String { error.localizedDescription }

    // MARK: Adding

    func testAddingAKeyCreatesAConnectionWithDefaultNames() async throws {
        upcoming = [fake(makeRoom(id: "lobby", name: "lobby", lastActivity: "2026-11-18T10:00:00Z"))]
        let store = makeStore()

        let id = try addedID(try await store.add(server: home, credential: .key("k1")))

        XCTAssertEqual(store.connections, [
            ConnectionRecord(id: id, server: home, name: "192.168.1.20:9229", displayName: "iPhone"),
        ])
        XCTAssertEqual(store.phases[id], .connected)
        XCTAssertEqual(store.rooms.map(\.ref), [RoomRef(connectionID: id, roomID: "lobby")])
        XCTAssertEqual(store.rooms.map(\.connectionName), ["192.168.1.20:9229 · Key ••••k1"])
        XCTAssertEqual(made[0].registeredNames, ["iPhone"])
        XCTAssertNotNil(store.session(for: id))
    }

    func testAddingUsesTheGivenNames() async throws {
        let store = makeStore()
        let id = try addedID(try await store.add(server: home, credential: .key("k1"), name: " Home ", displayName: "Ryan"))
        XCTAssertEqual(store.connections.first?.name, "Home")
        XCTAssertEqual(store.connections.first?.displayName, "Ryan")
        XCTAssertEqual(store.session(for: id)?.displayName, "Ryan")
    }

    func testRoomsFromAllConnectionsMergeByRecency() async throws {
        upcoming = [
            fake(makeRoom(id: "lobby", name: "lobby", lastActivity: "2026-11-18T08:00:00Z"),
                 makeRoom(id: "a", name: "alpha", lastActivity: "2026-11-18T11:00:00Z")),
            fake(makeRoom(id: "lobby", name: "lobby", lastActivity: "2026-11-18T10:00:00Z"),
                 makeRoom(id: "quiet", name: "quiet")),
        ]
        let store = makeStore()
        let first = try addedID(try await store.add(server: home, credential: .key("k1"), name: "Home"))
        let second = try addedID(try await store.add(server: global, credential: .key("k2"), name: "Global"))

        XCTAssertEqual(store.rooms.map(\.ref), [
            RoomRef(connectionID: first, roomID: "a"),
            RoomRef(connectionID: second, roomID: "lobby"),
            RoomRef(connectionID: first, roomID: "lobby"),
            RoomRef(connectionID: second, roomID: "quiet"),
        ])
        XCTAssertEqual(store.rooms.map(\.connectionName), ["Home", "Global", "Home", "Global"])
    }

    func testAddingAKeyAnExistingConnectionAlreadyHoldsIsRefused() async throws {
        let store = makeStore()
        _ = try await store.add(server: home, credential: .key("k1"))

        do {
            _ = try await store.add(server: .tcp(host: "192.168.1.20", port: 9229), credential: .key(" k1\n"))
            XCTFail("expected a refusal")
        } catch {
            XCTAssertEqual(message(of: error), "This key is already connected.")
        }
        XCTAssertEqual(store.connections.count, 1)
        XCTAssertEqual(made.count, 1)
    }

    func testTheSameKeyOnAnotherServerIsAllowed() async throws {
        let store = makeStore()
        _ = try await store.add(server: home, credential: .key("k1"))
        _ = try await store.add(server: global, credential: .key("k1"))
        XCTAssertEqual(store.connections.count, 2)
    }

    func testAFailedFirstSignInLeavesNoConnection() async {
        let failing = FakeConnection()
        failing.connectFailures = [CowchatConnectionError.timeout]
        upcoming = [failing]
        let store = makeStore()

        do {
            _ = try await store.add(server: home, credential: .key("k1"))
            XCTFail("expected a throw")
        } catch {
            XCTAssertEqual(
                message(of: error),
                "Could not reach 192.168.1.20:9229. Check the address and your network, then try again."
            )
        }
        XCTAssertEqual(store.connections, [])
        XCTAssertEqual(store.phases, [:])
        XCTAssertEqual(credentials.values, [:])
        XCTAssertEqual(makeStore().connections, [])
    }

    func testAFailedAddDoesNotDisturbAnExistingConnection() async throws {
        let failing = FakeConnection()
        failing.registerError = CowchatConnectionError.server(message: "invalid api key", code: "unauthorized")
        upcoming = [fake(makeRoom(id: "lobby", name: "lobby")), failing]
        let store = makeStore()
        let first = try addedID(try await store.add(server: home, credential: .key("k1")))

        do {
            _ = try await store.add(server: global, credential: .key("bad"))
            XCTFail("expected a throw")
        } catch {}

        XCTAssertEqual(store.connections.map(\.id), [first])
        XCTAssertEqual(store.phases, [first: .connected])
        XCTAssertEqual(store.rooms.map(\.ref.roomID), ["lobby"])
    }

    // MARK: Invites

    func testAnInviteJoinsARoomOnAnExistingPlainKeyConnectionToTheSameServer() async throws {
        let existing = fake(makeRoom(id: "lobby", name: "lobby"))
        existing.inviteRooms["cinv_abc"] = makeRoom(id: "r9", name: "design-review", lastActivity: "2026-11-18T10:00:00Z")
        upcoming = [existing]
        let store = makeStore()
        let id = try addedID(try await store.add(server: global, credential: .key("plain-key")))

        let outcome = try await store.add(
            server: .global(url: "wss://CHAT.example.test/ws/"),
            credential: .invite("Join with cinv_abc today")
        )

        XCTAssertEqual(outcome, .joinedRoom(RoomRef(connectionID: id, roomID: "r9")))
        XCTAssertEqual(existing.redeemedInviteTokens, ["cinv_abc"])
        XCTAssertEqual(redeemedTokens, [])
        XCTAssertEqual(store.connections.count, 1)
        XCTAssertEqual(store.rooms.map(\.summary.name), ["design-review", "lobby"])
    }

    // MARK: Removing what an invite added

    func testARoomAddedByInviteCanBeRemovedAndStaysGoneAfterARefresh() async throws {
        let existing = fake(makeRoom(id: "lobby", name: "lobby"))
        existing.inviteRooms["cinv_abc"] = makeRoom(id: "r9", name: "design-review")
        upcoming = [existing]
        let store = makeStore()
        let id = try addedID(try await store.add(server: global, credential: .key("plain-key")))
        _ = try await store.add(server: global, credential: .invite("cinv_abc"))
        let invited = RoomRef(connectionID: id, roomID: "r9")
        XCTAssertTrue(store.canRemove(invited))
        XCTAssertFalse(store.canRemove(RoomRef(connectionID: id, roomID: "lobby")), "only what an invite added")

        await store.remove(room: invited)

        XCTAssertEqual(store.rooms.map(\.summary.name), ["lobby"])
        XCTAssertEqual(store.connections.count, 1, "the connection it was added to stays")
        await store.refreshRooms()
        XCTAssertEqual(store.rooms.map(\.summary.name), ["lobby"], "the server still lists it; the phone does not")
        XCTAssertFalse(store.canRemove(invited))
    }

    func testARemovedInviteRoomStaysGoneAfterRelaunchAndReturnsWhenRedeemedAgain() async throws {
        let existing = fake(makeRoom(id: "lobby", name: "lobby"))
        existing.inviteRooms["cinv_abc"] = makeRoom(id: "r9", name: "design-review")
        upcoming = [existing]
        let store = makeStore()
        let id = try addedID(try await store.add(server: global, credential: .key("plain-key")))
        _ = try await store.add(server: global, credential: .invite("cinv_abc"))
        await store.remove(room: RoomRef(connectionID: id, roomID: "r9"))

        let relaunchedConnection = fake(makeRoom(id: "lobby", name: "lobby"), makeRoom(id: "r9", name: "design-review"))
        relaunchedConnection.inviteRooms["cinv_abc"] = makeRoom(id: "r9", name: "design-review")
        upcoming = [relaunchedConnection]
        let relaunched = makeStore()
        await relaunched.restore()
        XCTAssertEqual(relaunched.rooms.map(\.summary.name), ["lobby"])

        _ = try await relaunched.add(server: global, credential: .invite("cinv_abc"))
        XCTAssertEqual(Set(relaunched.rooms.map(\.summary.name)), ["lobby", "design-review"])
        XCTAssertTrue(relaunched.canRemove(RoomRef(connectionID: id, roomID: "r9")))
    }

    func testRemovingTheRoomOfAConnectionMadeFromAnInviteRemovesThatConnection() async throws {
        upcoming = [fake(makeRoom(id: "lobby", name: "lobby"), makeRoom(id: "r9", name: "design-review"))]
        let store = makeStore()
        let id = try addedID(try await store.add(server: global, credential: .invite("cinv_abc")))
        let invited = RoomRef(connectionID: id, roomID: "r9")
        XCTAssertTrue(store.canRemove(invited))
        XCTAssertFalse(store.canRemove(RoomRef(connectionID: id, roomID: "lobby")))

        await store.remove(room: invited)

        XCTAssertEqual(store.connections, [])
        XCTAssertEqual(store.rooms, [])
        XCTAssertTrue(credentials.values.isEmpty, "the minted key is erased")
    }

    func testRoomsOnAKeyConnectionCannotBeRemoved() async throws {
        upcoming = [fake(makeRoom(id: "r1", name: "plans"))]
        let store = makeStore()
        let id = try addedID(try await store.add(server: home, credential: .key("home-key")))
        let ref = RoomRef(connectionID: id, roomID: "r1")

        XCTAssertFalse(store.canRemove(ref))
        await store.remove(room: ref)

        XCTAssertEqual(store.rooms.map(\.summary.name), ["plans"])
    }

    func testAnInviteNextToAnAccountKeyConnectionMakesANewConnection() async throws {
        upcoming = [fake(makeRoom(id: "lobby", name: "lobby")), fake(makeRoom(id: "r9", name: "design-review"))]
        let store = makeStore()
        _ = try await store.add(server: global, credential: .key("csoc_device"))

        let second = try addedID(try await store.add(server: global, credential: .invite("cinv_abc")))

        XCTAssertEqual(redeemedTokens, ["cinv_abc"])
        XCTAssertEqual(made[0].redeemedInviteTokens, [])
        XCTAssertEqual(made[1].profile?.apiKey, "minted-key")
        XCTAssertEqual(store.connections.count, 2)
        XCTAssertEqual(store.phases[second], .connected)
    }

    func testAnInviteForAnotherServerMakesANewConnection() async throws {
        let store = makeStore()
        _ = try await store.add(server: .global(url: "wss://other.example.test/ws"), credential: .key("plain-key"))

        _ = try addedID(try await store.add(server: global, credential: .invite("cinv_abc")))

        XCTAssertEqual(redeemedTokens, ["cinv_abc"])
        XCTAssertEqual(made[0].redeemedInviteTokens, [])
        XCTAssertEqual(store.connections.count, 2)
    }

    func testAnInviteRefusedByTheExistingConnectionReportsAndAddsNothing() async throws {
        let store = makeStore()
        _ = try await store.add(server: global, credential: .key("plain-key"))

        do {
            _ = try await store.add(server: global, credential: .invite("cinv_gone"))
            XCTFail("expected a throw")
        } catch {
            XCTAssertTrue(message(of: error).contains("invite not found"), message(of: error))
        }
        XCTAssertEqual(redeemedTokens, [])
        XCTAssertEqual(store.connections.count, 1)
    }

    func testAMintedInviteKeySurvivesAFailedFirstConnectAndRetryUsesIt() async throws {
        let flaky = fake(makeRoom(id: "r9", name: "design-review"))
        flaky.connectFailures = [CowchatConnectionError.timeout]
        upcoming = [flaky]
        let store = makeStore()

        do {
            _ = try await store.add(server: global, credential: .invite("cinv_once"))
            XCTFail("expected a throw")
        } catch {}

        let id = try XCTUnwrap(store.connections.first?.id)
        XCTAssertEqual(store.connections.count, 1)
        guard case .failed = store.phases[id] else { return XCTFail("expected failed, got \(String(describing: store.phases[id]))") }

        await store.retry(connectionID: id)

        XCTAssertEqual(store.phases[id], .connected)
        XCTAssertEqual(redeemedTokens, ["cinv_once"])
        XCTAssertEqual(flaky.profile?.apiKey, "minted-key")
        XCTAssertEqual(store.rooms.map(\.summary.name), ["design-review"])
    }

    // MARK: Isolation

    func testADroppedConnectionDoesNotAffectTheOthers() async throws {
        let stable = fake(makeRoom(id: "lobby", name: "lobby"))
        let flaky = fake(makeRoom(id: "ops", name: "ops"))
        upcoming = [stable, flaky]
        let store = makeStore()
        let first = try addedID(try await store.add(server: home, credential: .key("k1")))
        let second = try addedID(try await store.add(server: global, credential: .key("k2")))

        flaky.connectFailures = Array(repeating: CowchatConnectionError.timeout, count: 10_000)
        flaky.drop()

        XCTAssertEqual(store.phases[first], .connected)
        XCTAssertEqual(store.phases[second], .reconnecting)
        XCTAssertTrue(store.rooms.contains { $0.ref == RoomRef(connectionID: first, roomID: "lobby") })

        stable.deliver(makeMessage(id: "m1", room: "lobby", seq: 4))
        XCTAssertEqual(store.rooms.first { $0.ref.connectionID == first }?.summary.isUnread, true)
        store.remove(connectionID: second)
    }

    func testUnreadAndSecretsAreKeptPerConnection() async throws {
        let sealed = try RoomCrypto.encrypt("the plan", secret: "right", roomID: "lobby")
        let a = fake(makeRoom(id: "lobby", name: "lobby"))
        let b = fake(makeRoom(id: "lobby", name: "lobby"))
        a.serverMessages["lobby"] = [makeMessage(id: "m1", room: "lobby", seq: 1, content: sealed)]
        b.serverMessages["lobby"] = [makeMessage(id: "m1", room: "lobby", seq: 1, content: sealed)]
        upcoming = [a, b]
        let store = makeStore()
        let first = try addedID(try await store.add(server: home, credential: .key("k1")))
        let second = try addedID(try await store.add(server: global, credential: .key("k2")))

        store.session(for: first)?.setSecret("right", roomID: "lobby")
        a.deliver(makeMessage(id: "m2", room: "lobby", seq: 2))
        await store.session(for: second)?.open(roomID: "lobby")

        let rows = Dictionary(uniqueKeysWithValues: store.rooms.map { ($0.ref.connectionID, $0.summary) })
        XCTAssertEqual(rows[first]?.isUnread, true)
        XCTAssertEqual(rows[second]?.isUnread, false)
        XCTAssertEqual(rows[first]?.hasSecret, true)
        XCTAssertEqual(rows[second]?.hasSecret, false)
        XCTAssertEqual(store.session(for: second)?.timelines["lobby"]?.entries.map(\.body), [.locked])
    }

    func testEachConnectionRegistersItsOwnAgentIdentity() async throws {
        let store = makeStore()
        _ = try await store.add(server: home, credential: .key("k1"))
        _ = try await store.add(server: global, credential: .key("k1"))
        XCTAssertNotEqual(made[0].registeredAgentIDs, made[1].registeredAgentIDs)
    }

    // MARK: Relaunch, rename, remove

    func testRestoreReconnectsEveryConnectionWithItsOwnKey() async throws {
        upcoming = [fake(makeRoom(id: "lobby", name: "lobby")), fake(makeRoom(id: "ops", name: "ops"))]
        let store = makeStore()
        let first = try addedID(try await store.add(server: home, credential: .key("k1"), name: "Home"))
        let second = try addedID(try await store.add(server: global, credential: .key("k2"), name: "Global"))
        let firstAgentIDs = made.map(\.registeredAgentIDs)

        let offline = FakeConnection()
        offline.connectFailures = Array(repeating: CowchatConnectionError.timeout, count: 10_000)
        upcoming = [offline, fake(makeRoom(id: "ops", name: "ops"))]
        let relaunched = makeStore()
        XCTAssertEqual(relaunched.connections.map(\.id), [first, second])
        await relaunched.restore()

        XCTAssertEqual(relaunched.phases[first], .reconnecting)
        XCTAssertEqual(relaunched.phases[second], .connected)
        XCTAssertEqual(made[3].profile?.apiKey, "k2")
        XCTAssertEqual(made[3].registeredAgentIDs, firstAgentIDs[1])
        XCTAssertEqual(relaunched.rooms.map(\.connectionName), ["Global"])
        relaunched.remove(connectionID: first)
    }

    func testRenameUpdatesRowsAndPersists() async throws {
        upcoming = [fake(makeRoom(id: "lobby", name: "lobby"))]
        let store = makeStore()
        let id = try addedID(try await store.add(server: home, credential: .key("k1")))

        store.rename(connectionID: id, to: "  Studio ")
        store.rename(connectionID: id, to: "   ")

        XCTAssertEqual(store.connections.first?.name, "Studio")
        XCTAssertEqual(store.rooms.map(\.connectionName), ["Studio"])
        XCTAssertEqual(makeStore().connections.first?.name, "Studio")
    }

    func testRemoveErasesOnlyThatConnection() async throws {
        upcoming = [fake(makeRoom(id: "lobby", name: "lobby")), fake(makeRoom(id: "ops", name: "ops"))]
        let store = makeStore()
        let first = try addedID(try await store.add(server: home, credential: .key("k1")))
        let second = try addedID(try await store.add(server: global, credential: .key("k2")))
        store.session(for: first)?.setSecret("s1", roomID: "lobby")
        store.session(for: second)?.setSecret("s2", roomID: "ops")

        store.remove(connectionID: first)

        XCTAssertEqual(store.connections.map(\.id), [second])
        XCTAssertNil(store.session(for: first))
        XCTAssertEqual(store.phases, [second: .connected])
        XCTAssertEqual(store.rooms.map(\.ref.roomID), ["ops"])
        XCTAssertEqual(Set(credentials.values.values), ["k2", "s2"])
        XCTAssertFalse(made[0].isConnected)
        XCTAssertTrue(made[1].isConnected)
        XCTAssertEqual(makeStore().connections.map(\.id), [second])
    }

    // MARK: Scene

    func testBackgroundAndReturnReachEveryConnection() async throws {
        let store = makeStore()
        let first = try addedID(try await store.add(server: home, credential: .key("k1")))
        let second = try addedID(try await store.add(server: global, credential: .key("k2")))

        store.sceneEnteredBackground()
        XCTAssertEqual(store.phases, [first: .reconnecting, second: .reconnecting])
        XCTAssertEqual(made.map(\.isConnected), [false, false])

        store.sceneBecameActive()
        let back = await eventually { store.phases == [first: .connected, second: .connected] }
        XCTAssertTrue(back)
    }

    func testARoomIsCreatedOnTheChosenConnection() async throws {
        upcoming = [fake(makeRoom(id: "a", name: "A")), fake(makeRoom(id: "b", name: "B"))]
        let store = makeStore()
        _ = try await store.add(server: home, credential: .key("k1"))
        let second = try addedID(try await store.add(server: global, credential: .key("k2")))

        let ref = try await store.createRoom(connectionID: second, name: "Planning")

        XCTAssertEqual(ref.connectionID, second)
        XCTAssertTrue(made[0].createdRooms.isEmpty)
        XCTAssertEqual(made[1].createdRooms.map(\.name), ["Planning"])
        XCTAssertTrue(store.rooms.contains { $0.ref == ref && $0.summary.name == "Planning" })
    }

    func testOnlyConnectedConnectionsCanTakeANewRoom() async throws {
        upcoming = [fake(), fake()]
        let store = makeStore()
        let first = try addedID(try await store.add(server: home, credential: .key("k1")))
        let second = try addedID(try await store.add(server: global, credential: .key("k2")))
        store.sceneEnteredBackground()
        made[0].isConnected = false

        XCTAssertTrue(store.connectedConnections.isEmpty)
        do {
            _ = try await store.createRoom(connectionID: first, name: "Planning")
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Reconnect before creating a room.")
        }
        _ = second
    }

    func testSearchAndJoiningWorkAcrossConnections() async throws {
        let first = fake(makeRoom(id: "a", name: "A"))
        first.serverMessages["a"] = [makeMessage(id: "m", room: "a", seq: 1, content: "the needle")]
        let second = fake(makeRoom(id: "a", name: "A"))
        second.joinableRooms = [makeRoom(id: "pub", name: "Public square")]
        upcoming = [first, second]
        let store = makeStore()
        let firstID = try addedID(try await store.add(server: home, credential: .key("k1")))
        let secondID = try addedID(try await store.add(server: global, credential: .key("k2"), name: "Work"))

        let matches = await store.messageMatches(query: "needle")
        XCTAssertEqual(matches, [RoomRef(connectionID: firstID, roomID: "a")])

        let joinable = await store.joinableRooms()
        XCTAssertEqual(joinable, [
            JoinableRoom(ref: RoomRef(connectionID: secondID, roomID: "pub"), name: "Public square", connectionName: "Work"),
        ])

        try await store.join(joinable[0].ref)
        XCTAssertTrue(store.rooms.contains { $0.ref == joinable[0].ref })
    }

    func testOnlyAConnectionSignedInToAnAccountGetsAnAccountStore() async throws {
        upcoming = [fake(), fake(), fake()]
        let store = makeStore()
        _ = try await store.add(server: global, credential: .key("plain-key"))
        _ = try await store.add(server: home, credential: .key("csoc_on_a_plain_server"))
        let signedIn = try addedID(try await store.add(server: global, credential: .key("csoc_device")))

        XCTAssertEqual(store.accounts.map(\.connectionID), [signedIn])
        XCTAssertEqual(accountAPIRequests, ["chat.example.test csoc_device"])
        let loaded = await eventually { store.account(for: signedIn)?.account?.handle == "ryan" }
        XCTAssertTrue(loaded)

        store.remove(connectionID: signedIn)
        XCTAssertTrue(store.accounts.isEmpty)
    }

    func testASessionWaitingForItsPersonComesFirstAndAnEndedOneLosesItsMark() async throws {
        upcoming = [fake(
            makeRoom(id: "busy", name: "Busy", lastActivity: "2026-11-18T12:00:00Z"),
            makeRoom(id: "s1", name: "cowchat-main", lastActivity: "2026-11-18T08:00:00Z")
        )]
        let store = makeStore()
        _ = try await store.add(server: global, credential: .key("csoc_device"))

        made[0].emit("session_state", ["agent_id": "a", "room_id": "s1", "state": "prompt"])
        XCTAssertEqual(store.rooms.map(\.summary.name), ["cowchat-main", "Busy"])
        XCTAssertEqual(store.rooms[0].summary.sessionState, "prompt")

        made[0].emit("session_state", ["agent_id": "a", "room_id": "s1", "state": "ended"])
        XCTAssertEqual(store.rooms.map(\.summary.name), ["Busy", "cowchat-main"])
        XCTAssertNil(store.rooms[1].summary.sessionState)
    }

    func testSigningInToAnAccountAlreadyConnectedReplacesThatConnection() async throws {
        upcoming = [fake(), fake(), fake()]
        let store = makeStore()
        let first = try addedID(try await store.add(server: global, credential: .key("csoc_one")))
        let loaded = await eventually { store.account(for: first)?.account != nil }
        XCTAssertTrue(loaded)
        let accountID = try XCTUnwrap(store.account(for: first)?.account?.id)

        let second = try addedID(
            try await store.add(server: global, credential: .key("csoc_two"), accountID: accountID))

        XCTAssertEqual(store.connections.map(\.id), [second])
        let revoked = await eventually { self.accountAPIs.first?.calls.contains("logout") == true }
        XCTAssertTrue(revoked, "the old device key is signed out")

        _ = try await store.add(server: global, credential: .key("csoc_three"), accountID: "someone-else")
        XCTAssertEqual(store.connections.count, 2)
    }

    func testARepeatSignInReplacesAConnectionWhoseAccountNeverLoaded() async throws {
        upcoming = [fake(), fake()]
        accountUnreachable = ["me"]
        let store = makeStore()
        _ = try await store.add(server: global, credential: .key("csoc_one"), accountID: "me")

        let second = try addedID(try await store.add(server: global, credential: .key("csoc_two"), accountID: "me"))

        XCTAssertEqual(store.connections.map(\.id), [second])
    }

    func testAnAccountSignInThatCannotConnectSignsItsNewKeyOut() async {
        let failing = FakeConnection()
        failing.connectFailures = [CowchatConnectionError.timeout]
        upcoming = [failing]
        let store = makeStore()

        _ = try? await store.add(server: global, credential: .key("csoc_new"), accountID: "me")

        XCTAssertEqual(store.connections, [])
        let revoked = await eventually { self.accountAPIs.contains { $0.calls.contains("logout") } }
        XCTAssertTrue(revoked)
        XCTAssertEqual(accountAPIRequests, ["chat.example.test csoc_new"])
    }

    func testOldMentionsOnAConnectionAddedLaterAreNotNewsAndReadingClearsTheBadge() async throws {
        let plain = fake(makeRoom(id: "j", name: "junction"))
        plain.serverMessages["j"] = [mention("m1", seq: 1)]
        upcoming = [fake(), plain]
        let store = makeStore()
        let accountID = try addedID(try await store.add(server: global, credential: .key("csoc_device")))
        _ = await eventually { store.account(for: accountID)?.account != nil }
        _ = await store.setNotificationsEnabled(true)
        await store.refreshAccounts()

        _ = try await store.add(server: global, credential: .key("plain-key"))
        await store.refreshAccounts()
        XCTAssertEqual(notifier.posted, [], "what the new connection already held is not news")

        plain.serverMessages["j"]?.append(mention("m2", seq: 2))
        plain.serverRooms = [makeRoom(id: "j", name: "junction", lastActivity: "2026-11-18T10:05:00Z")]
        await store.refreshRooms()
        await store.refreshAccounts()
        XCTAssertEqual(notifier.posted, ["pinger: look at m2"])
        XCTAssertEqual(notifier.badges.last, 2)

        store.account(for: accountID)?.markAllRead()
        XCTAssertEqual(notifier.badges.last, 0)
    }

    func testRemovingAConnectionLeavesNothingOfItInDefaults() async throws {
        upcoming = [fake()]
        let store = makeStore()
        let id = try addedID(try await store.add(server: global, credential: .key("csoc_device")))
        _ = await eventually { store.account(for: id)?.account != nil }
        await store.refreshAccounts()

        store.remove(connectionID: id)

        let left = defaults.persistentDomain(forName: suiteName)?.keys.filter { $0.contains(id) } ?? []
        XCTAssertEqual(left, [])
    }

    func testRemovingAnAccountConnectionRevokesItsKeyOnTheServer() async throws {
        upcoming = [fake()]
        let store = makeStore()
        let id = try addedID(try await store.add(server: global, credential: .key("csoc_device")))

        store.remove(connectionID: id)

        let revoked = await eventually { self.accountAPIs.first?.calls.contains("logout") == true }
        XCTAssertTrue(revoked)
    }

    func testLeavingAnAccountRoomTellsTheServerAndDropsItFromTheList() async throws {
        accountRooms = [SocialRoom(roomId: "theirs", name: "Theirs", ownerId: "a2")]
        upcoming = [fake(makeRoom(id: "theirs", name: "Theirs"), makeRoom(id: "other", name: "Other"))]
        let store = makeStore()
        let id = try addedID(try await store.add(server: global, credential: .key("csoc_device")))
        let ref = RoomRef(connectionID: id, roomID: "theirs")
        let loaded = await eventually { store.canLeave(ref) }
        XCTAssertTrue(loaded)
        XCTAssertFalse(store.canLeave(RoomRef(connectionID: id, roomID: "other")))

        made[0].serverRooms.removeAll { $0.id == "theirs" }
        await store.leave(ref)

        XCTAssertTrue(accountAPIs[0].calls.contains("leave theirs"))
        XCTAssertEqual(store.rooms.map(\.summary.name), ["Other"])
    }

    func testEachConnectionKnowsWhetherAnAccountAnInviteOrAKeyMadeIt() async throws {
        upcoming = [fake(), fake(), fake()]
        let store = makeStore()
        let key = try addedID(try await store.add(server: home, credential: .key("plain-key")))
        let account = try addedID(try await store.add(server: global, credential: .key("csoc_device")))
        let invite = try addedID(try await store.add(server: global, credential: .invite("cinv_abc")))

        XCTAssertEqual(store.source(of: key), .key(ending: "-key"), "the last four characters, as a bank shows an account")
        XCTAssertEqual(store.source(of: invite), .invite)
        let named = await eventually { store.source(of: account) == .account(handle: "ryan") }
        XCTAssertTrue(named)

        made[1].isConnected = false
        store.sceneEnteredBackground()
        XCTAssertEqual(store.source(of: account), .account(handle: "ryan"), "known while offline too")
    }

    func testRoomsOfTwoConnectionsWithOneNameSayWhichIsWhich() async throws {
        upcoming = [fake(makeRoom(id: "a", name: "A")), fake(makeRoom(id: "b", name: "B")), fake(makeRoom(id: "c", name: "C"))]
        let store = makeStore()
        _ = try await store.add(server: global, credential: .key("csoc_device"))
        _ = try await store.add(server: global, credential: .invite("cinv_abc"))
        _ = try await store.add(server: home, credential: .key("plain-key"), name: "Home")

        let names = Dictionary(uniqueKeysWithValues: store.rooms.map { ($0.summary.name, $0.connectionName) })
        let named = await eventually {
            store.rooms.first { $0.summary.name == "A" }?.connectionName == "chat.example.test · @ryan"
        }
        XCTAssertTrue(named, "the handle shows once the account has loaded")
        XCTAssertEqual(names["A"], "chat.example.test · Account", "and Account until then")
        XCTAssertEqual(names["B"], "chat.example.test · Invited")
        XCTAssertEqual(names["C"], "Home")
    }

    private func mention(_ id: String, seq: Int) -> ChatMessage {
        makeMessage(id: id, room: "j", seq: seq, name: "pinger", content: "@ryan look at \(id)", extra: ["mentions": ["me"]])
    }

    func testAMentionInAPlainKeyRoomOnTheSameServerReachesTheAccountsInbox() async throws {
        let plain = fake(makeRoom(id: "j", name: "junction"))
        plain.serverMessages["j"] = [mention("m1", seq: 1)]
        let elsewhere = fake(makeRoom(id: "j", name: "home-room"))
        elsewhere.serverMessages["j"] = [mention("m2", seq: 1)]
        upcoming = [fake(), plain, elsewhere]
        let store = makeStore()
        let accountID = try addedID(try await store.add(server: global, credential: .key("csoc_device")))
        let plainID = try addedID(try await store.add(server: global, credential: .key("plain-key")))
        _ = try await store.add(server: home, credential: .key("home-key"))
        _ = await eventually { store.account(for: accountID)?.account != nil }

        await store.refreshAccounts()

        let inbox = store.account(for: accountID)?.inbox ?? []
        XCTAssertEqual(inbox.map(\.id), ["community.j.m1"])
        XCTAssertEqual(inbox.first?.connectionID, plainID)
        XCTAssertEqual(inbox.first?.message, "look at m1")
        XCTAssertEqual(inbox.first?.roomName, "junction")
    }

    func testNewInboxItemsNotifyOnlyWhenNotificationsAreOnAndTheBadgeCountsUnread() async throws {
        let plain = fake(makeRoom(id: "j", name: "junction"))
        upcoming = [fake(), plain]
        let store = makeStore()
        let accountID = try addedID(try await store.add(server: global, credential: .key("csoc_device")))
        _ = try await store.add(server: global, credential: .key("plain-key"))
        _ = await eventually { store.account(for: accountID)?.account != nil }
        await store.refreshAccounts()

        plain.serverMessages["j"] = [mention("m1", seq: 1)]
        plain.serverRooms = [makeRoom(id: "j", name: "junction", lastActivity: "2026-11-18T10:00:00Z")]
        await store.refreshRooms()
        await store.refreshAccounts()
        XCTAssertTrue(notifier.posted.isEmpty, "off by default")

        let granted = await store.setNotificationsEnabled(true)
        XCTAssertTrue(granted)
        plain.serverMessages["j"]?.append(mention("m2", seq: 2))
        plain.serverRooms = [makeRoom(id: "j", name: "junction", lastActivity: "2026-11-18T10:05:00Z")]
        await store.refreshRooms()
        await store.refreshAccounts()

        XCTAssertEqual(notifier.posted, ["pinger: look at m2"])
        XCTAssertEqual(notifier.badges.last, 2)

        notifier.allows = false
        let relaunched = makeStore()
        XCTAssertTrue(relaunched.notificationsEnabled, "the choice is kept")
        let refused = await relaunched.setNotificationsEnabled(true)
        XCTAssertFalse(refused, "it stays off when iOS refuses permission")
        XCTAssertFalse(relaunched.notificationsEnabled)
    }

    func testAnAccountConnectionHasItsAccountStoreBeforeItConnects() async throws {
        upcoming = [fake()]
        let store = makeStore()
        _ = try await store.add(server: global, credential: .key("csoc_device"))

        let offline = FakeConnection()
        offline.connectFailures = [URLError(.notConnectedToInternet)]
        upcoming = [offline]
        let relaunched = makeStore()

        XCTAssertEqual(relaunched.accounts.count, 1, "a background refresh can read the Inbox without the socket")
    }

    func testANewRoomOnAnAccountConnectionIsMadeThroughTheAccountWithItsFriends() async throws {
        let connection = fake()
        upcoming = [connection]
        let store = makeStore()
        let id = try addedID(try await store.add(server: global, credential: .key("csoc_device")))
        _ = await eventually { store.account(for: id)?.account != nil }
        connection.serverRooms = [makeRoom(id: "new-room", name: "launch")]

        let ref = try await store.createRoom(connectionID: id, name: "launch", friendIDs: ["a2"])

        XCTAssertEqual(ref, RoomRef(connectionID: id, roomID: "new-room"))
        XCTAssertTrue(accountAPIs[0].calls.contains("createRoom launch a2"))
        XCTAssertTrue(connection.createdRooms.isEmpty, "not through the room frame")
        XCTAssertTrue(store.rooms.contains { $0.ref == ref })
    }

    func testARoomRowNamesTheKeyByItsEndingAndMarksRoomsAnInviteAdded() async throws {
        let first = fake(makeRoom(id: "own", name: "Own"))
        first.inviteRooms["cinv_abc"] = makeRoom(id: "shared", name: "Shared")
        upcoming = [first, fake(makeRoom(id: "other", name: "Other"))]
        let store = makeStore()
        _ = try await store.add(server: global, credential: .key("plain-key-9f3a"))
        _ = try await store.add(server: global, credential: .invite("cinv_abc"))
        _ = try await store.add(server: home, credential: .key("home-key-77c1"))

        let names = Dictionary(uniqueKeysWithValues: store.rooms.map { ($0.summary.name, $0.connectionName) })
        XCTAssertEqual(names["Own"], "chat.example.test · Key ••••9f3a")
        XCTAssertEqual(names["Shared"], "chat.example.test · Invited", "a room an invite added to that key")
        XCTAssertEqual(names["Other"], "192.168.1.20:9229 · Key ••••77c1")
    }

    func testTheUnreadTotalFollowsTheInboxAndItsReadMarks() async throws {
        let plain = fake(makeRoom(id: "j", name: "junction"))
        plain.serverMessages["j"] = [mention("m1", seq: 1), mention("m2", seq: 2)]
        upcoming = [fake(), plain]
        let store = makeStore()
        let accountID = try addedID(try await store.add(server: global, credential: .key("csoc_device")))
        _ = try await store.add(server: global, credential: .key("plain-key"))
        _ = await eventually { store.account(for: accountID)?.account != nil }
        XCTAssertEqual(store.inboxUnread, 0)

        await store.refreshAccounts()
        XCTAssertEqual(store.inboxUnread, 2)

        let account = try XCTUnwrap(store.account(for: accountID))
        account.markRead(account.inbox[0])
        XCTAssertEqual(store.inboxUnread, 1)
        account.markAllRead()
        XCTAssertEqual(store.inboxUnread, 0)
    }
}
