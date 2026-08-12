import XCTest
@testable import CowchatMac

final class ConnectPromptTests: XCTestCase {
    func testConnectPromptIncludesRoomAndDurableAgentLifecycleContract() {
        let prompt = ChatStore.connectPromptText(
            roomName: "design-review",
            connectionInstruction: "connect to the local server"
        )
        XCTAssertTrue(prompt.contains("“design-review”"))
        XCTAssertTrue(prompt.contains("connect to the local server"))
        XCTAssertTrue(prompt.contains("catch up existing messages once with `history --cursor-file`"))
        XCTAssertTrue(prompt.contains("same stable `--name` and `--agent-id`"))
        XCTAssertTrue(prompt.contains("`wait --loop --drain`"))
        XCTAssertTrue(prompt.contains("Choose one `--cursor-file` path unique"))
        XCTAssertTrue(prompt.contains("same cursor file to every `send` and `wait`"))
        XCTAssertTrue(prompt.contains("After processing each wake and sending your reply"))
        XCTAssertTrue(prompt.contains("Do not use `wait --follow`"))
        XCTAssertTrue(prompt.contains("ordinary Cowchat messages cannot restart a completed Codex turn"))
        XCTAssertTrue(prompt.contains("`cowchat-codex relay`"))
        XCTAssertTrue(prompt.contains("`conversation_end`"))
        XCTAssertTrue(prompt.contains("https://cowchat.cowboy.inc/skills.txt"))
    }

    func testTemporaryRoomPromptDoesNotPromiseCursorDurability() {
        let prompt = ChatStore.connectPromptText(
            roomName: "quick-sync",
            connectionInstruction: "connect to the local server",
            isEphemeral: true
        )
        XCTAssertTrue(prompt.contains("Temporary room “quick-sync”"))
        XCTAssertTrue(prompt.contains("messages are not persisted"))
        XCTAssertTrue(prompt.contains("choose or create a permanent room"))
        XCTAssertFalse(prompt.contains("`wait --loop --drain`"))
        XCTAssertFalse(prompt.contains("catch up existing messages"))
    }
}
