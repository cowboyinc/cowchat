import XCTest
@testable import CowchatMac

final class ConnectPromptTests: XCTestCase {
    func testConnectPromptIncludesActiveTurnListeningContract() {
        let prompt = ChatStore.connectPromptText(
            roomName: "design-review",
            connectionInstruction: "connect to the local server"
        )

        XCTAssertTrue(prompt.contains("“design-review”"))
        XCTAssertTrue(prompt.contains("connect to the local server"))
        XCTAssertTrue(prompt.contains("one unique, stable `--name` and `--agent-id` pair"))
        XCTAssertTrue(prompt.contains("same pair on every Cowchat agent command"))
        XCTAssertTrue(prompt.contains("unique to this server, room, and agent"))
        XCTAssertTrue(prompt.contains("highest message sequence you actually processed"))
        XCTAssertTrue(prompt.contains("use `0` if there is no history"))
        XCTAssertTrue(
            prompt.contains(#"run `thinking design-review "<one line on what you're doing>"`"#)
        )
        XCTAssertTrue(prompt.contains("refresh it roughly every minute"))
        XCTAssertTrue(prompt.contains("returning `wait --loop --drain --cursor-file <that-file>`"))
        XCTAssertTrue(prompt.contains("never recompute the floor from the room tip"))
        XCTAssertTrue(prompt.contains("Do not use `wait --follow`"))
        XCTAssertTrue(prompt.contains("send your reply, then immediately run the exact same returning wait again"))
        XCTAssertTrue(prompt.contains("do not end this task while collaboration is active"))
        XCTAssertTrue(prompt.contains("ordinary Cowchat messages alone cannot resume it automatically"))
        XCTAssertTrue(prompt.contains("explicitly configured external wake mechanism"))
        XCTAssertTrue(prompt.contains("https://cowchat.cowboy.inc/skills.txt"))
        XCTAssertFalse(prompt.contains("cowchat-codex"))
        XCTAssertFalse(prompt.contains("wake relay"))
        XCTAssertFalse(prompt.contains("will automatically resume"))
    }

    func testConnectPromptPreservesCloudConnectionInstruction() {
        let instruction = "connect to Cowchat Cloud at wss://cloud.example/ws using your Cowchat Cloud API key"
        let prompt = ChatStore.connectPromptText(
            roomName: "cloud-review",
            connectionInstruction: instruction
        )

        XCTAssertTrue(prompt.contains("“cloud-review”"))
        XCTAssertTrue(prompt.contains(instruction))
    }

    /// A global-room prompt must be one-shot for a stranger. With a cached
    /// single-use invite it embeds the redeem call; before one exists it
    /// falls back to open self-serve signup instructions.
    @MainActor
    func testGlobalInstructionEmbedsInviteRedeemCall() throws {
        let store = try makeGlobalStore()

        let instruction = store.agentConnectionInstruction(inviteToken: "cinv_abc123")

        XCTAssertTrue(
            instruction.contains(
                "curl -fsS -X POST https://chat.cowchat.cowboy.inc/api/invites/redeem"
            )
        )
        XCTAssertTrue(instruction.contains(#"{"token":"cinv_abc123"}"#))
        XCTAssertTrue(
            instruction.contains("--url wss://chat.cowchat.cowboy.inc/ws --key <your api_key>")
        )
        XCTAssertTrue(instruction.contains("single-use"))
    }

    @MainActor
    func testGlobalInstructionFallsBackToSelfServeWithoutInvite() throws {
        let store = try makeGlobalStore()

        let instruction = store.agentConnectionInstruction(inviteToken: nil)

        XCTAssertTrue(
            instruction.contains("curl -fsS -X POST https://chat.cowchat.cowboy.inc/api/keys")
        )
        XCTAssertTrue(instruction.contains("`api_key`"))
        XCTAssertFalse(instruction.contains("using your Cowchat API key"))
    }

    @MainActor
    func testCopyConsumesTheCachedInviteAndMintsAReplacement() async throws {
        let connection = PromptInviteStubConnection()
        connection.invites = ["cinv_first", "cinv_second"]
        let store = try makeGlobalStore(connection: connection)
        store.connectionStatus = .connected
        let room = makeRoom(id: "room-1", name: "design")

        store.ensurePromptInvite(for: room)
        while store.promptInviteTokens[room.id] == nil { await Task.yield() }
        XCTAssertEqual(store.promptInviteTokens[room.id], "cinv_first")

        // The displayed prompt does not consume.
        XCTAssertTrue(store.connectPrompt(for: room).contains("cinv_first"))
        XCTAssertTrue(store.connectPrompt(for: room).contains("cinv_first"))

        // Copying consumes and re-mints.
        let copied = await store.copyableConnectPrompt(for: room)
        XCTAssertTrue(copied.contains("cinv_first"))
        while store.promptInviteTokens[room.id] == nil { await Task.yield() }
        XCTAssertEqual(store.promptInviteTokens[room.id], "cinv_second")
    }

    /// Copying with nothing cached (rapid double-copy, or a room whose
    /// connect screen never appeared) awaits a mint inline — every copy is
    /// its own invitation, never a silent fallback.
    @MainActor
    func testCopyWithEmptyCacheAwaitsAFreshMint() async throws {
        let connection = PromptInviteStubConnection()
        connection.invites = ["cinv_a", "cinv_b", "cinv_c", "cinv_d"]
        let store = try makeGlobalStore(connection: connection)
        store.connectionStatus = .connected
        let room = makeRoom(id: "room-2", name: "ops")

        // No ensurePromptInvite ran; the copy must still carry an invite.
        let first = await store.copyableConnectPrompt(for: room)
        XCTAssertTrue(first.contains("cinv_a"), "first copy should mint inline")

        // Two rapid copies produce two distinct invites.
        while store.promptInviteTokens[room.id] == nil { await Task.yield() }
        let second = await store.copyableConnectPrompt(for: room)
        let third = await store.copyableConnectPrompt(for: room)
        let tokens = [first, second, third].compactMap { prompt in
            ["cinv_a", "cinv_b", "cinv_c", "cinv_d"].first { prompt.contains($0) }
        }
        XCTAssertEqual(tokens.count, 3)
        XCTAssertEqual(Set(tokens).count, 3, "each copy must carry a distinct invite")
        XCTAssertFalse(third.contains("/api/keys"), "no self-serve fallback while minting works")
    }

    @MainActor
    func testLocalStoreNeverMintsPromptInvites() {
        let connection = PromptInviteStubConnection()
        let store = ChatStore(
            connection: connection,
            defaults: UserDefaults(suiteName: "ConnectPromptTests.\(UUID().uuidString)")!,
            connectionProfile: .local
        )
        store.connectionStatus = .connected

        store.ensurePromptInvite(for: makeRoom(id: "r", name: "local-room"))

        XCTAssertTrue(store.promptInviteTokens.isEmpty)
        XCTAssertFalse(connection.didMint)
        XCTAssertEqual(
            store.agentConnectionInstruction(inviteToken: nil),
            "connect to the local server"
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

    @MainActor
    private func makeGlobalStore(
        connection: (any CowchatConnectionProtocol)? = nil
    ) throws -> ChatStore {
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://chat.cowchat.cowboy.inc/ws",
            apiKey: "prompt-test-key"
        )
        return ChatStore(
            connection: connection ?? CowchatConnection(profile: profile),
            defaults: UserDefaults(suiteName: "ConnectPromptTests.\(UUID().uuidString)")!,
            connectionProfile: profile
        )
    }
}

@MainActor
private final class PromptInviteStubConnection: CowchatConnectionProtocol {
    var onEvent: ((String, [String: Any]) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?
    var invites: [String] = []
    private(set) var didMint = false

    func connect() async throws {}
    func register(name: String, agentID: String) async throws -> CowchatRegistration {
        CowchatRegistration(agentID: agentID)
    }
    func listRooms() async throws -> [Room] { [] }
    func listAgents(roomID: String) async throws -> [AgentPresence] { [] }
    func createRoom(
        name: String, description: String?, parentID: String?, isPublic: Bool
    ) async throws -> Room { throw CancellationError() }
    func rename(roomID: String, name: String) async throws -> Room { throw CancellationError() }
    func destroy(roomID: String) async throws {}
    func createInvite(roomID: String, singleUse: Bool) async throws -> String {
        didMint = true
        guard !invites.isEmpty else { throw CancellationError() }
        return invites.removeFirst()
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
