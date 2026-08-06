import XCTest
@testable import CowchatMac

final class ConnectPromptTests: XCTestCase {
    func testConnectPromptNamesTheExactRoomAndInstruction() {
        let prompt = ChatStore.connectPromptText(
            roomName: "design-review",
            connectionInstruction: "connect to the local server"
        )
        XCTAssertTrue(prompt.contains("“design-review”"))
        XCTAssertTrue(prompt.contains("connect to the local server"))
        XCTAssertTrue(prompt.contains("https://cowchat.cowboy.inc/skills.txt"))
    }
}
