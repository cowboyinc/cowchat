import XCTest
@testable import CowchatMac

final class ThinkingPulsePresentationTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_787_200_000)

    func testPulsesAnchorUnderEachAgentsLatestMessage() throws {
        let messages = [
            message(id: "m1", agent: "claude-b", seq: 1),
            message(id: "m2", agent: "codex-a", seq: 2),
            message(id: "m3", agent: "claude-b", seq: 3),
        ]
        let pulses = [
            "claude-b": pulse(agent: "claude-b", text: "tracing build path", age: 10),
            "codex-a": pulse(agent: "codex-a", text: nil, age: 20),
        ]

        let anchors = ThinkingPulsePresentation.anchors(
            messages: messages, pulses: pulses, currentAgentID: "me", now: now
        )

        // claude-b's pill sits under m3 (its LATEST message), never m1.
        XCTAssertEqual(anchors.byMessageID["m3"]?.map(\.agentID), ["claude-b"])
        XCTAssertNil(anchors.byMessageID["m1"])
        XCTAssertEqual(anchors.byMessageID["m2"]?.map(\.agentID), ["codex-a"])
        XCTAssertTrue(anchors.unanchored.isEmpty)
    }

    func testSilentAgentsPillAtFeedEndAndSelfAndStaleAreExcluded() throws {
        let messages = [message(id: "m1", agent: "claude-b", seq: 1)]
        let pulses = [
            "newcomer": pulse(agent: "newcomer", text: "reading history", age: 5),
            "me": pulse(agent: "me", text: "self must not pill", age: 5),
            "stale": pulse(agent: "stale", text: "expired", age: 500),
        ]

        let anchors = ThinkingPulsePresentation.anchors(
            messages: messages, pulses: pulses, currentAgentID: "me", now: now
        )

        XCTAssertTrue(anchors.byMessageID.isEmpty)
        XCTAssertEqual(anchors.unanchored.map(\.agentID), ["newcomer"])
    }

    func testLabelFormatsTextProgressAndTextlessPulses() {
        XCTAssertEqual(
            ThinkingPulsePresentation.label(
                for: pulse(agent: "claude-b", text: "tracing build path", age: 0)
            ),
            "claude-b: tracing build path"
        )
        XCTAssertEqual(
            ThinkingPulsePresentation.label(
                for: pulse(agent: "claude-b", text: "verifying", age: 0, progress: 42)
            ),
            "claude-b: verifying · 42%"
        )
        XCTAssertEqual(
            ThinkingPulsePresentation.label(for: pulse(agent: "codex-a", text: nil, age: 0)),
            "codex-a is thinking…"
        )
    }

    private func message(id: String, agent: String, seq: Int) -> ChatMessage {
        try! JSONDecoder().decode(
            ChatMessage.self,
            from: JSONSerialization.data(withJSONObject: [
                "message_id": id,
                "room_id": "r1",
                "agent_id": agent,
                "agent_name": agent,
                "content": "hello",
                "timestamp": "2026-08-20T02:00:00Z",
                "seq": seq,
            ])
        )
    }

    private func pulse(
        agent: String,
        text: String?,
        age: TimeInterval,
        progress: Int? = nil
    ) -> AgentThinkingPulse {
        AgentThinkingPulse(
            agentID: agent,
            agentName: agent,
            text: text,
            progress: progress,
            at: now.addingTimeInterval(-age)
        )
    }
}
