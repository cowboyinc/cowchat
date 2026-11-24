import XCTest
@testable import CowchatiOS
import CowchatKit

/// A Keychain that can be made to fail, as a locked or full one does.
final class FailingCredentialStore: CowchatCredentialStore {
    var values: [String: String] = [:]
    var failsWrites = false

    func credential(for account: String) throws -> String? { values[account] }

    func setCredential(_ credential: String?, for account: String) throws {
        if failsWrites { throw CowchatCredentialStoreError.keychain(-25308) }
        values[account] = credential
    }
}

@MainActor
final class HistoryPagingTests: XCTestCase {
    private var connection: FakeConnection!
    private var credentials: FailingCredentialStore!
    private var defaults: UserDefaults!
    private var suiteName: String!
    private let server = ServerChoice.tcp(host: "192.168.1.20", port: 9229)

    override func setUp() async throws {
        connection = FakeConnection()
        connection.serverRooms = [makeRoom(id: "r1", name: "General", encrypted: false)]
        credentials = FailingCredentialStore()
        suiteName = "HistoryPagingTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: suiteName)
    }

    private func signedIn() async -> ChatSession {
        let session = ChatSession(
            record: ConnectionRecord(id: "c1", server: server, name: "Home", displayName: "iPhone"),
            connection: connection, credentials: credentials, defaults: defaults,
            redeemInvite: { _, _ in throw FakeError(message: "no invites") },
            sleep: { _ in try await Task.sleep(nanoseconds: 1_000_000) }
        )
        await session.signIn(server: server, credential: .key("k1"), displayName: "iPhone")
        return session
    }

    private func messages(_ range: ClosedRange<Int>, timestamp: String? = nil) -> [ChatMessage] {
        range.map { makeMessage(id: "m\($0)", seq: $0, timestamp: timestamp ?? String(format: "2026-11-18T%02d:%02d:%02dZ", $0 / 3600, $0 / 60 % 60, $0 % 60)) }
    }

    private func seqs(_ session: ChatSession) -> [Int] { session.timelines["r1"]?.entries.map(\.seq) ?? [] }

    // MARK: Reopening after a long gap

    func testReopeningAfterMoreThanFiveHundredNewMessagesLosesNone() async {
        connection.serverMessages["r1"] = messages(1...1)
        let session = await signedIn()
        await session.open(roomID: "r1")
        session.close(roomID: "r1")
        connection.serverMessages["r1"] = messages(1...502)

        await session.open(roomID: "r1")

        XCTAssertEqual(seqs(session), Array(1...502))
        XCTAssertEqual(session.rooms.first?.isUnread, false)
    }

    func testAGapTooLongToFillLeavesTheNewestMessagesWithNoHole() async {
        connection.serverMessages["r1"] = messages(1...30)
        let session = await signedIn()
        await session.open(roomID: "r1")
        session.close(roomID: "r1")
        connection.serverMessages["r1"] = messages(1...4000)

        await session.open(roomID: "r1")

        // What was loaded before the gap is let go, so the timeline is one unbroken run ending at the newest message.
        let loaded = seqs(session)
        XCTAssertEqual(loaded.last, 4000)
        XCTAssertEqual(loaded, Array((loaded.first ?? 0)...4000), "no hole")
        XCTAssertGreaterThan(loaded.first ?? 0, 30)
        XCTAssertEqual(session.timelines["r1"]?.hasEarlier, true)

        await session.loadEarlier(roomID: "r1")
        let earlier = seqs(session)
        XCTAssertEqual(earlier, Array((earlier.first ?? 0)...4000), "loading earlier carries on from the oldest loaded")
        XCTAssertEqual(earlier.first, (loaded.first ?? 0) - 50)
    }

    // MARK: A catch-up that fails part way

    /// Makes the given history call fail once, counting from the next call.
    private func failHistoryCall(_ number: Int) {
        var calls = 0
        connection.beforeHistory = { [unowned self] in
            calls += 1
            connection.historyErrors["r1"] = calls == number ? CowchatConnectionError.transport("dropped") : nil
        }
    }

    func testACatchUpThatFailsPartWayLeavesNoGapOnceItIsRetried() async {
        connection.serverMessages["r1"] = messages(1...30)
        let session = await signedIn()
        await session.open(roomID: "r1")
        session.close(roomID: "r1")
        connection.serverMessages["r1"] = messages(1...230)

        // The newest page arrives, the next one fails.
        failHistoryCall(2)
        await session.open(roomID: "r1")

        let afterFailure = seqs(session)
        XCTAssertEqual(afterFailure, Array((afterFailure.first ?? 0)...(afterFailure.last ?? 0)), "a half-finished catch-up shows no hole")
        XCTAssertEqual(session.rooms.first?.isUnread, true, "the room has messages that were not read")

        connection.beforeHistory = nil
        connection.historyErrors = [:]
        session.close(roomID: "r1")
        await session.open(roomID: "r1")

        XCTAssertEqual(seqs(session), Array(1...230), "the retry fills the middle")
        XCTAssertEqual(session.rooms.first?.isUnread, false)
    }

    func testAMessageArrivingAfterAFailedCatchUpDoesNotHideTheGap() async {
        connection.serverMessages["r1"] = messages(1...30)
        let session = await signedIn()
        await session.open(roomID: "r1")
        session.close(roomID: "r1")
        connection.serverMessages["r1"] = messages(1...230)
        failHistoryCall(2)
        await session.open(roomID: "r1")
        connection.beforeHistory = nil
        connection.historyErrors = [:]

        // A new message arrives live. It must not be taken as proof that everything before it is loaded.
        let live = messages(231...231)[0]
        connection.serverMessages["r1"]?.append(live)
        connection.onEvent?("message_received", messagePayload(live))
        session.close(roomID: "r1")
        await session.open(roomID: "r1")

        XCTAssertEqual(seqs(session), Array(1...231))
    }

    func testAReconnectDoesNotTakeLiveMessagesAsProofOfNoGap() async {
        connection.serverMessages["r1"] = messages(1...30)
        let session = await signedIn()
        await session.open(roomID: "r1")
        // While connected, live messages extend what is known to be loaded.
        for message in messages(31...33) {
            connection.serverMessages["r1"]?.append(message)
            connection.onEvent?("message_received", messagePayload(message))
        }
        XCTAssertEqual(seqs(session), Array(1...33))
        session.close(roomID: "r1")
        connection.serverMessages["r1"] = messages(1...40)

        await session.open(roomID: "r1")

        XCTAssertEqual(seqs(session), Array(1...40))
        XCTAssertEqual(connection.sequenceRequests.suffix(1), [nil], "one page was enough: 31 to 33 were already known")
    }

    // MARK: Messages that share a timestamp

    func testMessagesSharingATimestampAreNotLostAtAPageBoundary() async {
        connection.serverMessages["r1"] = messages(1...51, timestamp: "2026-11-18T10:00:00.000Z")
        let session = await signedIn()
        await session.open(roomID: "r1")
        XCTAssertEqual(seqs(session), Array(2...51))
        XCTAssertEqual(session.timelines["r1"]?.hasEarlier, true)

        await session.loadEarlier(roomID: "r1")

        XCTAssertEqual(seqs(session), Array(1...51))
    }

    func testCatchingUpPagesBySequenceWhenTimestampsRepeat() async {
        connection.serverMessages["r1"] = messages(1...1)
        let session = await signedIn()
        await session.open(roomID: "r1")
        session.close(roomID: "r1")
        connection.serverMessages["r1"] = messages(1...1) + messages(2...121, timestamp: "2026-11-18T11:00:00.000Z")

        await session.open(roomID: "r1")

        XCTAssertEqual(seqs(session), Array(1...121))
    }

    // MARK: Room secrets and the Keychain

    func testASecretTheKeychainRefusedIsNotTreatedAsSaved() async throws {
        connection.serverRooms = [makeRoom(id: "r1", name: "General", encrypted: true)]
        let session = await signedIn()
        credentials.failsWrites = true

        XCTAssertThrowsError(try session.setSecret("correct horse", roomID: "r1")) { error in
            XCTAssertEqual(error.localizedDescription, "Cowchat could not save this room's secret on this iPhone.")
        }
        XCTAssertEqual(session.rooms.first?.hasSecret, false, "the room stays locked")
    }

    func testASecretTheKeychainWouldNotRemoveStaysInUse() async throws {
        connection.serverRooms = [makeRoom(id: "r1", name: "General", encrypted: true)]
        let session = await signedIn()
        try session.setSecret("correct horse", roomID: "r1")
        credentials.failsWrites = true

        XCTAssertThrowsError(try session.setSecret(nil, roomID: "r1")) { error in
            XCTAssertEqual(error.localizedDescription, "Cowchat could not remove this room's secret from this iPhone.")
        }
        XCTAssertEqual(session.rooms.first?.hasSecret, true, "the secret that is still saved is still the one in use")
    }
}
