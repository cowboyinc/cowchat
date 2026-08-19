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
    func createInvite(roomID: String, singleUse: Bool) async throws -> String {
        throw CancellationError()
    }
    func listInvites(roomID: String) async throws -> [RoomInvite] { [] }
    func revokeInvite(inviteID: String) async throws {}
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

    func testSignupURLDerivesFromCloudURL() {
        XCTAssertEqual(
            WorkspaceStore.signupURL(forCloudURLString: "wss://chat.cowchat.cowboy.inc/ws")?
                .absoluteString,
            "https://chat.cowchat.cowboy.inc/api/keys"
        )
        XCTAssertEqual(
            WorkspaceStore.signupURL(forCloudURLString: "wss://cloud.example:8443/ws")?
                .absoluteString,
            "https://cloud.example:8443/api/keys"
        )
        XCTAssertNil(WorkspaceStore.signupURL(forCloudURLString: "http://cloud.example/ws"))
        XCTAssertNil(WorkspaceStore.signupURL(forCloudURLString: "not a url"))
    }

    @MainActor
    func testConnectToGlobalMintsAKeyWhenFieldIsEmpty() async {
        var requestedURLs: [URL] = []
        let fixture = makeWorkspace(withGlobal: false) { url in
            requestedURLs.append(url)
            return "minted-key"
        }
        defer { fixture.cleanup() }

        let connected = await fixture.workspace.connectToGlobal(
            url: "wss://cloud.invalid/ws",
            apiKey: "   "
        )

        XCTAssertTrue(connected)
        XCTAssertEqual(
            requestedURLs.map(\.absoluteString),
            ["https://cloud.invalid/api/keys"]
        )
        XCTAssertNotNil(fixture.workspace.global)
        XCTAssertTrue(fixture.preferences.isGlobalEnabled())
        XCTAssertEqual(
            try fixture.preferences.loadConfiguredCloudProfile()?.apiKey,
            "minted-key"
        )
    }

    @MainActor
    func testConnectToGlobalUsesPastedKeyWithoutMinting() async {
        var mintCalls = 0
        let fixture = makeWorkspace(withGlobal: false) { _ in
            mintCalls += 1
            return "should-not-be-used"
        }
        defer { fixture.cleanup() }

        let connected = await fixture.workspace.connectToGlobal(
            url: "wss://cloud.invalid/ws",
            apiKey: "pasted-key"
        )

        XCTAssertTrue(connected)
        XCTAssertEqual(mintCalls, 0)
        XCTAssertEqual(
            try fixture.preferences.loadConfiguredCloudProfile()?.apiKey,
            "pasted-key"
        )
    }

    @MainActor
    func testConnectToGlobalSurfacesSignupFailureInline() async {
        let fixture = makeWorkspace(withGlobal: false) { _ in
            throw GlobalSignupError.signupClosed
        }
        defer { fixture.cleanup() }

        let connected = await fixture.workspace.connectToGlobal(
            url: "wss://cloud.invalid/ws",
            apiKey: ""
        )

        XCTAssertFalse(connected)
        XCTAssertNil(fixture.workspace.global)
        XCTAssertEqual(
            fixture.workspace.globalSetupError,
            GlobalSignupError.signupClosed.localizedDescription
        )
        XCTAssertFalse(fixture.preferences.isGlobalEnabled())
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
    private func makeWorkspace(
        withGlobal: Bool,
        requestAPIKey: @escaping (URL) async throws -> String = { _ in
            throw GlobalSignupError.signupClosed
        }
    ) -> (
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
                global: global,
                requestAPIKey: requestAPIKey
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
