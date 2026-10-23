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
    func redeemInvite(token: String) async throws -> String {
        throw CancellationError()
    }
    func join(roomID: String) async throws {}
    func leave(roomID: String) async throws {}
    func history(roomID: String, limit: Int, before: String?) async throws -> [ChatMessage] { [] }
    func send(roomID: String, content: String) async throws -> ChatMessage {
        throw CancellationError()
    }
    func disconnect() {}
}

private final class InMemoryCredentialStore: CowchatCredentialStore {
    func accounts(withPrefix prefix: String) throws -> [String] { credentials.keys.filter { $0.hasPrefix(prefix) } }
    private var credentials: [String: String] = [:]

    func credential(for account: String) throws -> String? { credentials[account] }

    func setCredential(_ credential: String?, for account: String) throws {
        credentials[account] = credential
    }
}

final class WorkspaceStoreTests: XCTestCase {
    @MainActor
    func testPendingFriendInvitationPersistsWithoutChangingEndpointOrSending() throws {
        let fixture = makeWorkspace(withGlobal: true)
        defer { fixture.cleanup() }
        let workspace = fixture.workspace
        let endpoint = workspace.social.endpoint
        let invitation = URL(string: "cowchat://friend/Chad-DePue")!
        workspace.receiveFriendInvitation(invitation)
        let pending = try XCTUnwrap(workspace.pendingFriendInvitation)
        XCTAssertEqual(workspace.social.endpoint, endpoint)
        workspace.useSocialSession(makeSession())
        XCTAssertEqual(workspace.pendingFriendInvitation, pending)
        workspace.useSocialSession(nil)
        XCTAssertEqual(workspace.pendingFriendInvitation, pending)
        workspace.receiveFriendInvitation(URL(string: "cowchat://friend/Chad?evil=1")!)
        XCTAssertEqual(workspace.pendingFriendInvitation, pending)
        workspace.clearFriendInvitation(pending)
        XCTAssertNil(workspace.pendingFriendInvitation)
    }

    @MainActor
    func testPendingFriendInvitationRestoresAfterAppRestartAndOnlyExplicitClearRemovesIt() throws {
        let suite = "FriendInvitationTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let preferences = ConnectionProfilePreferences(defaults: defaults, credentialStore: InMemoryCredentialStore())
        func make() -> WorkspaceStore {
            WorkspaceStore(local: ChatStore(connection: StubConnection(), defaults: defaults, connectionProfile: .local), preferences: preferences, defaults: defaults, socialCredentials: InMemoryCredentialStore())
        }
        let first = make()
        first.receiveFriendInvitation(URL(string: "cowchat://friend/Chad-DePue")!)
        let restored = make()
        XCTAssertEqual(restored.pendingFriendInvitation, first.pendingFriendInvitation)
        let old = try XCTUnwrap(restored.pendingFriendInvitation)
        restored.receiveFriendInvitation(URL(string: "cowchat://friend/Another")!)
        restored.clearFriendInvitation(old)
        XCTAssertEqual(make().pendingFriendInvitation?.slug, "Another")
        restored.clearFriendInvitation(try XCTUnwrap(restored.pendingFriendInvitation))
        XCTAssertNil(make().pendingFriendInvitation)
    }

    @MainActor
    func testAccountSignInAndSignOutPreserveSharedGlobalConnection() async {
        let fixture = makeWorkspace(withGlobal: true)
        defer { fixture.cleanup() }
        let workspace = fixture.workspace
        let shared = workspace.global
        await workspace.select(room: makeRoom(id: "shared", name: "shared"), on: .global)
        workspace.useSocialSession(makeSession())
        XCTAssertTrue(workspace.global === shared)
        XCTAssertEqual(workspace.activeServer, .global)
        XCTAssertNotNil(workspace.accountGlobal)
        workspace.activate(server: .accountGlobal)

        workspace.useSocialSession(nil)

        XCTAssertNil(workspace.accountGlobal)
        XCTAssertTrue(workspace.global === shared)
        XCTAssertEqual(workspace.activeServer, .global)
        XCTAssertTrue(workspace.activeStore === shared)
        XCTAssertTrue(fixture.preferences.isGlobalEnabled())
    }

    @MainActor
    func testAccountSignOutWithoutSharedGlobalFallsBackToLocal() {
        let fixture = makeWorkspace(withGlobal: false)
        defer { fixture.cleanup() }
        fixture.workspace.useSocialSession(makeSession())
        fixture.workspace.activate(server: .accountGlobal)

        fixture.workspace.useSocialSession(nil)

        XCTAssertEqual(fixture.workspace.activeServer, .local)
        XCTAssertTrue(fixture.workspace.activeStore === fixture.workspace.local)
    }

    @MainActor
    func testDisconnectingSharedGlobalPreservesSignedInAccount() {
        let fixture = makeWorkspace(withGlobal: true, withAccountSession: true)
        defer { fixture.cleanup() }
        let workspace = fixture.workspace
        workspace.useSocialSession(workspace.social.savedSession)
        let accountStore = workspace.accountGlobal
        workspace.activate(server: .global)

        workspace.disableGlobalRooms()

        XCTAssertNil(workspace.global)
        XCTAssertNotNil(workspace.social.savedSession)
        XCTAssertEqual(workspace.social.phase, .signedIn)
        XCTAssertTrue(workspace.accountGlobal === accountStore)
        XCTAssertEqual(workspace.activeServer, .accountGlobal)
        XCTAssertTrue(workspace.activeStore === accountStore)
        XCTAssertFalse(fixture.preferences.isGlobalEnabled())
    }

    @MainActor
    func testSearchTextFansOutToAllThreeStoresAndNewAccountStore() {
        let fixture = makeWorkspace(withGlobal: true)
        defer { fixture.cleanup() }
        let workspace = fixture.workspace
        workspace.searchText = "before sign in"
        workspace.useSocialSession(makeSession())
        XCTAssertEqual(workspace.accountGlobal?.searchText, "before sign in")
        workspace.searchText = "deploy"

        for server in WorkspaceStore.Server.allCases {
            XCTAssertEqual(workspace.store(for: server)?.searchText, "deploy")
        }
    }

    @MainActor
    func testOverlappingRoomIDsRemainScopedToEachConnection() async {
        let fixture = makeWorkspace(withGlobal: true)
        defer { fixture.cleanup() }
        let workspace = fixture.workspace
        workspace.useSocialSession(makeSession())
        let room = makeRoom(id: "same-id", name: "design")

        for selectedServer in WorkspaceStore.Server.allCases {
            await workspace.select(room: room, on: selectedServer)
            XCTAssertTrue(workspace.activeStore === workspace.store(for: selectedServer))
            for server in WorkspaceStore.Server.allCases {
                XCTAssertEqual(workspace.isSelected(room, on: server), server == selectedServer)
            }
        }
    }

    @MainActor
    func testSignedInAccountAllowsSameServerSharedKeyButRejectsServerSwitch() {
        let fixture = makeWorkspace(withGlobal: true, withAccountSession: true)
        defer { fixture.cleanup() }
        let workspace = fixture.workspace
        workspace.useSocialSession(workspace.social.savedSession)
        let accountStore = workspace.accountGlobal

        XCTAssertTrue(workspace.saveGlobalConfiguration(url: "wss://cloud.invalid/ws", apiKey: "replacement"))
        XCTAssertTrue(workspace.accountGlobal === accountStore)
        XCTAssertEqual(workspace.social.savedSession?.endpoint.absoluteString, "wss://cloud.invalid/ws")
        XCTAssertFalse(workspace.saveGlobalConfiguration(url: "wss://other.invalid/ws", apiKey: "other"))
        XCTAssertEqual(workspace.global?.connectionProfile.endpointURL?.absoluteString, "wss://cloud.invalid/ws")
        XCTAssertEqual(try fixture.preferences.loadConfiguredCloudProfile()?.apiKey, "replacement")
        XCTAssertNotNil(workspace.globalSetupError)
    }

    private func makeSession() -> SocialSession {
        SocialSession(
            endpoint: URL(string: "wss://cloud.invalid/ws")!,
            deviceKey: "csoc_fixture",
            account: SocialAccount(id: "fixture-account", handle: "fixture", displayName: "Fixture")
        )
    }

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
        withAccountSession: Bool = false,
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
            _ = try! preferences.save(profile)
            preferences.setGlobalEnabled(true)
        }
        let socialCredentials = InMemoryCredentialStore()
        if withAccountSession {
            try! socialCredentials.setCredential(
                try! JSONEncoder().encode(makeSession()).base64EncodedString(),
                for: "cowchat-social-session-v1"
            )
        }
        return (
            WorkspaceStore(
                local: local,
                preferences: preferences,
                defaults: defaults,
                global: global,
                makeAccountStore: { profile in
                    ChatStore(connection: StubConnection(), defaults: defaults, connectionProfile: profile)
                },
                socialCredentials: socialCredentials,
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
