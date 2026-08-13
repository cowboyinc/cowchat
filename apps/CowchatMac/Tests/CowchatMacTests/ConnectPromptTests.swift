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

    /// A global-room prompt must be one-shot for a stranger. With a minted
    /// guest key it embeds the key outright; before one exists it falls back
    /// to mint-your-own instructions.
    @MainActor
    func testGlobalConnectionInstructionEmbedsAMintedGuestKey() async throws {
        let store = try makeGlobalStore()
        store.mintGuestAPIKey = { url in
            XCTAssertEqual(url.absoluteString, "https://chat.cowchat.cowboy.inc/api/keys")
            return "guest-key-123"
        }

        store.ensureGuestPromptKey()
        while store.guestPromptKey == nil { await Task.yield() }

        let instruction = store.agentConnectionInstruction
        XCTAssertTrue(
            instruction.contains("--url wss://chat.cowchat.cowboy.inc/ws --key guest-key-123")
        )
        XCTAssertFalse(instruction.contains("curl"))
    }

    @MainActor
    func testGlobalConnectionInstructionFallsBackToSelfServeWithoutGuestKey() throws {
        let store = try makeGlobalStore()

        let instruction = store.agentConnectionInstruction

        XCTAssertTrue(
            instruction.contains("curl -fsS -X POST https://chat.cowchat.cowboy.inc/api/keys")
        )
        XCTAssertTrue(instruction.contains("`api_key`"))
        XCTAssertTrue(
            instruction.contains("--url wss://chat.cowchat.cowboy.inc/ws --key <your api_key>")
        )
        XCTAssertFalse(instruction.contains("using your Cowchat API key"))
    }

    @MainActor
    func testLocalStoreNeverMintsAGuestKey() {
        let store = ChatStore(
            connection: CowchatConnection(),
            defaults: UserDefaults(suiteName: "ConnectPromptTests.\(UUID().uuidString)")!,
            connectionProfile: .local
        )
        store.mintGuestAPIKey = { _ in
            XCTFail("local stores must not mint guest keys")
            return "never"
        }

        store.ensureGuestPromptKey()

        XCTAssertNil(store.guestPromptKey)
        XCTAssertEqual(store.agentConnectionInstruction, "connect to the local server")
    }

    @MainActor
    private func makeGlobalStore() throws -> ChatStore {
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://chat.cowchat.cowboy.inc/ws",
            apiKey: "prompt-test-key"
        )
        return ChatStore(
            connection: CowchatConnection(profile: profile),
            defaults: UserDefaults(suiteName: "ConnectPromptTests.\(UUID().uuidString)")!,
            connectionProfile: profile
        )
    }
}
