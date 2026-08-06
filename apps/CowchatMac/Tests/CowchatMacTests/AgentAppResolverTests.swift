import XCTest
@testable import CowchatMac

final class AgentAppResolverTests: XCTestCase {
    func testKnownAgentNamesResolve() {
        XCTAssertEqual(
            AgentAppResolver.resolvedApp(forAgentNamed: "Claude Code")?.bundleID,
            "com.anthropic.claudefordesktop"
        )
        XCTAssertEqual(AgentAppResolver.resolvedApp(forAgentNamed: "codex-cli")?.bundleID, "com.openai.codex")
        XCTAssertEqual(AgentAppResolver.resolvedApp(forAgentNamed: "ChatGPT agent")?.bundleID, "com.openai.chat")
        XCTAssertEqual(AgentAppResolver.resolvedApp(forAgentNamed: "CLAUDE")?.displayName, "Claude")
    }

    func testUnknownAgentNameResolvesNil() {
        XCTAssertNil(AgentAppResolver.resolvedApp(forAgentNamed: "mystery-bot"))
    }
}
