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

    /// A global-room prompt must be one-shot for a stranger: it has to say how
    /// to mint a key, not assume the recipient already holds one.
    @MainActor
    func testGlobalConnectionInstructionIsSelfContained() throws {
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://chat.cowchat.cowboy.inc/ws",
            apiKey: "prompt-test-key"
        )
        let store = ChatStore(
            connection: CowchatConnection(profile: profile),
            defaults: UserDefaults(suiteName: "ConnectPromptTests.\(UUID().uuidString)")!,
            connectionProfile: profile
        )

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
}
