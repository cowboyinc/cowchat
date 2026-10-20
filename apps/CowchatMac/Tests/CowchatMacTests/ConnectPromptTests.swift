import XCTest
@testable import CowchatMac

final class ConnectPromptTests: XCTestCase {
    func testConnectPromptDelegatesCollaborationDetailsToSkill() {
        let prompt = ChatStore.connectPromptText(roomName: "design-review", connectionInstruction: "connect to the local server")
        XCTAssertTrue(prompt.contains("https://cowchat.cowboy.inc/skills.txt"))
        XCTAssertTrue(prompt.contains("follow its agent collaboration instructions"))
        XCTAssertTrue(prompt.contains("“design-review”"))
        XCTAssertTrue(prompt.contains("connect to the local server"))
        XCTAssertTrue(prompt.contains("start collaborating now"))
        XCTAssertLessThan(prompt.count, 280)
        XCTAssertFalse(prompt.contains("--cursor-file"))
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
            instruction.contains("--url wss://chat.cowchat.cowboy.inc/ws --key <api_key>")
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

    func testRoomDiscoveryMatchesTypedSeparatorsWithoutChangingRoomIdentity() {
        let room = makeRoom(id: "existing-id", name: "cowboy-junction")
        XCTAssertTrue(RoomDiscovery.matches(room, query: " cowboy_JUNCTION "))
        XCTAssertTrue(RoomDiscovery.matches(room, query: "junction"))
        XCTAssertTrue(RoomDiscovery.matches(room, query: ""))
        XCTAssertFalse(RoomDiscovery.matches(room, query: "another-room"))
        XCTAssertEqual(room.name, "cowboy-junction")
        XCTAssertEqual(room.id, "existing-id")
    }

    func testRoomActivityLabelsUseCalendarDaysWeeksMonthsAndYears() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        let now = try XCTUnwrap(calendar.date(from: DateComponents(year: 2026, month: 10, day: 20, hour: 12)))
        let cases: [(Calendar.Component, Int, String)] = [
            (.hour, -2, "today"), (.day, -1, "yesterday"), (.day, -6, "6d ago"),
            (.day, -7, "1w ago"), (.month, -1, "1m ago"), (.year, -1, "1y ago"),
            (.day, 1, "today")
        ]
        for (component, offset, expected) in cases {
            let date = try XCTUnwrap(calendar.date(byAdding: component, value: offset, to: now))
            XCTAssertEqual(RoomDiscovery.activityLabel(date, now: now, calendar: calendar), expected)
        }
        XCTAssertNil(RoomDiscovery.activityLabel(nil, now: now, calendar: calendar))
        let nearMidnight = try XCTUnwrap(calendar.date(from: DateComponents(year: 2026, month: 10, day: 20, hour: 0, minute: 1)))
        XCTAssertEqual(RoomDiscovery.activityLabel(nearMidnight.addingTimeInterval(-120), now: nearMidnight, calendar: calendar), "yesterday")
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
