import XCTest
@testable import CowchatiOS
import CowchatKit

/// What the app does when the Keychain will not delete something: it must not look as if it did.
@MainActor
final class KeychainFailureTests: XCTestCase {
    private var connection: FakeConnection!
    private var credentials: FailingCredentialStore!
    private var defaults: UserDefaults!
    private var suiteName: String!
    private let server = ServerChoice.global(url: "wss://chat.example.test/ws")

    override func setUp() async throws {
        connection = FakeConnection()
        connection.serverRooms = [makeRoom(id: "lobby", name: "lobby")]
        credentials = FailingCredentialStore()
        suiteName = "KeychainFailureTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: suiteName)
    }

    private func makeStore() -> ConnectionsStore {
        ConnectionsStore(
            makeConnection: { [unowned self] in connection },
            credentials: credentials,
            defaults: defaults,
            redeemInvite: { _, _ in "minted-key" },
            sleep: { _ in try await Task.sleep(nanoseconds: 1_000_000) },
            makeAccountAPI: { _, _ in nil }
        )
    }

    private func addedID(_ store: ConnectionsStore) async throws -> String {
        guard case .addedConnection(let id) = try await store.add(server: server, credential: .key("k1")) else {
            throw FakeError(message: "no connection")
        }
        return id
    }

    func testAConnectionWhoseKeyCannotBeDeletedIsNotRemoved() async throws {
        let store = makeStore()
        let id = try await addedID(store)
        credentials.failsWrites = true

        XCTAssertFalse(store.remove(connectionID: id))

        XCTAssertEqual(store.connections.map(\.id), [id], "it is still listed, so it can be removed again")
        XCTAssertEqual(store.phases[id], .connected, "and still signed in, as the next launch would find it")
        XCTAssertEqual(store.storageError, "Cowchat could not remove this connection's key from this iPhone. Try again.")
        XCTAssertEqual(credentials.values.count, 1)

        credentials.failsWrites = false
        store.storageError = nil
        XCTAssertTrue(store.remove(connectionID: id))
        XCTAssertTrue(store.connections.isEmpty)
        XCTAssertTrue(credentials.values.isEmpty)
        XCTAssertNil(store.storageError)
    }

    func testSigningOutFailsWhenTheKeyCannotBeDeleted() async throws {
        let store = makeStore()
        let id = try await addedID(store)
        let session = try XCTUnwrap(store.session(for: id))
        credentials.failsWrites = true

        XCTAssertThrowsError(try session.signOut())

        XCTAssertEqual(session.phase, .connected)
        XCTAssertTrue(session.hasSavedKey)
    }

    func testAnInvitedRoomWhoseSecretCannotBeDeletedIsNotRemoved() async throws {
        let store = makeStore()
        let id = try await addedID(store)
        let session = try XCTUnwrap(store.session(for: id))
        connection.inviteRooms["cinv_abc"] = makeRoom(id: "plans", name: "plans", encrypted: true)
        _ = try await store.add(server: server, credential: .invite("cinv_abc"))
        try session.setSecret("correct horse", roomID: "plans")
        let room = RoomRef(connectionID: id, roomID: "plans")
        XCTAssertTrue(store.canRemove(room))
        credentials.failsWrites = true

        await store.remove(room: room)

        XCTAssertTrue(store.rooms.contains { $0.ref == room }, "the room stays, with the secret it still holds")
        XCTAssertEqual(store.rooms.first { $0.ref == room }?.summary.hasSecret, true)
        XCTAssertEqual(store.storageError, "Cowchat could not remove this room's secret from this iPhone.")
    }
}
