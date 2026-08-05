import XCTest
@testable import CowchatMac

@MainActor
final class AgentIDTests: XCTestCase {
    private func freshDefaults(_ name: String) -> UserDefaults {
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    func testReturnsExistingID() {
        let defaults = freshDefaults("test.cowchat.existing")
        defaults.set("cowchat-mac-kept", forKey: "CowchatMac.agentID")

        XCTAssertEqual(ChatStore.resolveAgentID(defaults: defaults), "cowchat-mac-kept")
    }

    func testGeneratesAndPersistsCowchatPrefixedID() {
        let defaults = freshDefaults("test.cowchat.fresh")

        let id = ChatStore.resolveAgentID(defaults: defaults)

        XCTAssertTrue(id.hasPrefix("cowchat-mac-"))
        XCTAssertEqual(defaults.string(forKey: "CowchatMac.agentID"), id)
        XCTAssertEqual(ChatStore.resolveAgentID(defaults: defaults), id, "stable across calls")
    }

    func testCloudScopesReceiveDistinctStableAgentIDsWithoutChangingLegacyLocalID() {
        let defaults = freshDefaults("test.cowchat.scoped")
        defaults.set("cowchat-mac-local", forKey: "CowchatMac.agentID")

        let firstCloudID = ChatStore.resolveAgentID(
            defaults: defaults,
            scope: "cowchat-cloud.account-one"
        )
        let secondCloudID = ChatStore.resolveAgentID(
            defaults: defaults,
            scope: "cowchat-cloud.account-two"
        )

        XCTAssertNotEqual(firstCloudID, secondCloudID)
        XCTAssertEqual(
            ChatStore.resolveAgentID(defaults: defaults, scope: "cowchat-cloud.account-one"),
            firstCloudID
        )
        XCTAssertEqual(ChatStore.resolveAgentID(defaults: defaults), "cowchat-mac-local")
    }
}
