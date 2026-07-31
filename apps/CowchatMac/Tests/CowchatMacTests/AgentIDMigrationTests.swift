import XCTest
@testable import CowchatMac

@MainActor
final class AgentIDMigrationTests: XCTestCase {
    private func freshDefaults(_ name: String) -> UserDefaults {
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    func testMigratesLegacyAgentIDWithoutDeletingIt() {
        let new = freshDefaults("test.cowchat.new")
        let legacy = freshDefaults("test.clawchat.legacy")
        legacy.set("clawchat-mac-abc123", forKey: "ClawChatMac.agentID")

        let id = ChatStore.resolveAgentID(defaults: new, legacy: legacy)

        XCTAssertEqual(id, "clawchat-mac-abc123")
        XCTAssertEqual(new.string(forKey: "CowchatMac.agentID"), "clawchat-mac-abc123")
        XCTAssertEqual(legacy.string(forKey: "ClawChatMac.agentID"), "clawchat-mac-abc123")
    }

    func testPrefersExistingNewValueOverLegacy() {
        let new = freshDefaults("test.cowchat.new2")
        let legacy = freshDefaults("test.clawchat.legacy2")
        new.set("cowchat-mac-kept", forKey: "CowchatMac.agentID")
        legacy.set("clawchat-mac-ignored", forKey: "ClawChatMac.agentID")

        XCTAssertEqual(
            ChatStore.resolveAgentID(defaults: new, legacy: legacy),
            "cowchat-mac-kept"
        )
    }

    func testGeneratesCowchatPrefixedIDWhenNothingExists() {
        let new = freshDefaults("test.cowchat.new3")

        let id = ChatStore.resolveAgentID(defaults: new, legacy: nil)

        XCTAssertTrue(id.hasPrefix("cowchat-mac-"))
        XCTAssertEqual(new.string(forKey: "CowchatMac.agentID"), id)
    }
}
