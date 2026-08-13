import XCTest
@testable import CowchatMac

@MainActor
private final class StubConnection: CowchatConnectionProtocol {
    var onEvent: ((String, [String: Any]) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?

    func connect() async throws {}
    func register(name: String, agentID: String) async throws -> CowchatRegistration {
        CowchatRegistration(agentID: agentID)
    }
    func listRooms() async throws -> [Room] { [] }
    func listAgents(roomID: String) async throws -> [AgentPresence] { [] }
    func createRoom(
        name: String,
        description: String?,
        parentID: String?,
        isPublic: Bool
    ) async throws -> Room {
        throw CancellationError()
    }
    func rename(roomID: String, name: String) async throws -> Room { throw CancellationError() }
    func destroy(roomID: String) async throws {}
    func join(roomID: String) async throws {}
    func leave(roomID: String) async throws {}
    func history(roomID: String, limit: Int, before: String?) async throws -> [ChatMessage] { [] }
    func send(roomID: String, content: String) async throws -> ChatMessage {
        throw CancellationError()
    }
    func disconnect() {}
}

private final class InMemoryCredentialStore: CowchatCredentialStore {
    private var credentials: [String: String] = [:]

    func credential(for account: String) throws -> String? { credentials[account] }

    func setCredential(_ credential: String?, for account: String) throws {
        credentials[account] = credential
    }
}

final class WorkspaceStoreTests: XCTestCase {
    @MainActor
    func testSelectingARoomActivatesItsServer() async throws {
        let fixture = makeWorkspace(withGlobal: true)
        defer { fixture.cleanup() }
        let workspace = fixture.workspace
        let room = makeRoom(id: "shared-id", name: "design")

        XCTAssertEqual(workspace.activeServer, .local)
        await workspace.select(room: room, on: .global)

        XCTAssertEqual(workspace.activeServer, .global)
        XCTAssertTrue(workspace.activeStore === workspace.global)
        XCTAssertTrue(workspace.isSelected(room, on: .global))
        // The same room id on the inactive server must not read as selected.
        XCTAssertFalse(workspace.isSelected(room, on: .local))

        await workspace.select(room: room, on: .local)
        XCTAssertEqual(workspace.activeServer, .local)
        XCTAssertTrue(workspace.isSelected(room, on: .local))
        XCTAssertFalse(workspace.isSelected(room, on: .global))
    }

    @MainActor
    func testSearchTextFansOutToBothStores() {
        let fixture = makeWorkspace(withGlobal: true)
        defer { fixture.cleanup() }

        fixture.workspace.searchText = "deploy"

        XCTAssertEqual(fixture.workspace.local.searchText, "deploy")
        XCTAssertEqual(fixture.workspace.global?.searchText, "deploy")
    }

    @MainActor
    func testDisableGlobalRoomsFallsBackToLocalAndPersists() async {
        let fixture = makeWorkspace(withGlobal: true)
        defer { fixture.cleanup() }
        let workspace = fixture.workspace
        await workspace.select(room: makeRoom(id: "g1", name: "global-room"), on: .global)

        workspace.disableGlobalRooms()

        XCTAssertNil(workspace.global)
        XCTAssertEqual(workspace.activeServer, .local)
        XCTAssertTrue(workspace.activeStore === workspace.local)
        XCTAssertFalse(fixture.preferences.isGlobalEnabled())
    }

    @MainActor
    func testSaveGlobalConfigurationRejectsInvalidURLInline() {
        let fixture = makeWorkspace(withGlobal: false)
        defer { fixture.cleanup() }

        XCTAssertFalse(
            fixture.workspace.saveGlobalConfiguration(
                url: "http://cloud.invalid/ws",
                apiKey: "key"
            )
        )
        XCTAssertNil(fixture.workspace.global)
        XCTAssertNotNil(fixture.workspace.globalSetupError)
    }

    @MainActor
    func testSaveGlobalConfigurationCreatesAndEnablesGlobalStore() {
        let fixture = makeWorkspace(withGlobal: false)
        defer { fixture.cleanup() }

        XCTAssertTrue(
            fixture.workspace.saveGlobalConfiguration(
                url: "wss://cloud.invalid/ws",
                apiKey: "workspace-save-key"
            )
        )

        XCTAssertNotNil(fixture.workspace.global)
        XCTAssertNil(fixture.workspace.globalSetupError)
        XCTAssertTrue(fixture.preferences.isGlobalEnabled())
        XCTAssertEqual(
            fixture.workspace.global?.connectionProfile.endpointURL?.absoluteString,
            "wss://cloud.invalid/ws"
        )
    }

    @MainActor
    func testConfiguredGlobalValuesDefaultToCowboysServer() {
        let fixture = makeWorkspace(withGlobal: false)
        defer { fixture.cleanup() }

        let values = fixture.workspace.configuredGlobalValues()

        XCTAssertEqual(values.url, ConnectionProfile.defaultGlobalURLString)
        XCTAssertEqual(values.apiKey, "")
    }

    @MainActor
    private func makeWorkspace(withGlobal: Bool) -> (
        workspace: WorkspaceStore,
        preferences: ConnectionProfilePreferences,
        cleanup: () -> Void
    ) {
        let suiteName = "WorkspaceStoreTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        let preferences = ConnectionProfilePreferences(
            defaults: defaults,
            credentialStore: InMemoryCredentialStore()
        )
        let local = ChatStore(
            connection: StubConnection(),
            defaults: defaults,
            connectionProfile: .local
        )
        var global: ChatStore?
        if withGlobal {
            let profile = try! ConnectionProfile.cowchatCloud(
                urlString: "wss://cloud.invalid/ws",
                apiKey: "test-key"
            )
            global = ChatStore(
                connection: StubConnection(),
                defaults: defaults,
                connectionProfile: profile,
                connectionPreferences: preferences
            )
            preferences.setGlobalEnabled(true)
        }
        return (
            WorkspaceStore(
                local: local,
                preferences: preferences,
                defaults: defaults,
                global: global
            ),
            preferences,
            { defaults.removePersistentDomain(forName: suiteName) }
        )
    }

    private func makeRoom(id: String, name: String) -> Room {
        Room(
            roomID: id,
            name: name,
            description: nil,
            parentID: nil,
            createdAt: "2026-08-13T12:00:00Z",
            createdBy: nil,
            visibility: "public",
            lastActivity: nil,
            memberCount: nil,
            encrypted: false
        )
    }
}
