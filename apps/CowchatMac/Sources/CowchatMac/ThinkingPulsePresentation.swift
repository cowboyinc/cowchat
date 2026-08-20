import Foundation

/// Pure placement/formatting rules for in-feed thinking pulse pills: each
/// pulsing agent's pill sits under that agent's most recent message; agents
/// who haven't spoken yet pill at the end of the feed.
enum ThinkingPulsePresentation {
    static let freshnessWindow: TimeInterval = 120

    /// `byMessageID` maps a message id to the pulses anchored beneath it
    /// (an agent anchors to its own latest message). `unanchored` holds
    /// fresh pulses from agents with no message in the feed, oldest first.
    static func anchors(
        messages: [ChatMessage],
        pulses: [String: AgentThinkingPulse],
        currentAgentID: String,
        now: Date,
        window: TimeInterval = freshnessWindow
    ) -> (byMessageID: [String: [AgentThinkingPulse]], unanchored: [AgentThinkingPulse]) {
        let fresh = pulses.values.filter {
            $0.agentID != currentAgentID && now.timeIntervalSince($0.at) < window
        }
        guard !fresh.isEmpty else { return ([:], []) }

        var latestMessageIDByAgent: [String: String] = [:]
        for message in messages.reversed()
        where latestMessageIDByAgent[message.agentID] == nil {
            latestMessageIDByAgent[message.agentID] = message.id
        }

        var byMessageID: [String: [AgentThinkingPulse]] = [:]
        var unanchored: [AgentThinkingPulse] = []
        for pulse in fresh {
            if let anchor = latestMessageIDByAgent[pulse.agentID] {
                byMessageID[anchor, default: []].append(pulse)
            } else {
                unanchored.append(pulse)
            }
        }
        for (key, value) in byMessageID {
            byMessageID[key] = value.sorted { $0.at < $1.at }
        }
        unanchored.sort { $0.at < $1.at }
        return (byMessageID, unanchored)
    }

    static func label(for pulse: AgentThinkingPulse) -> String {
        guard let text = pulse.text else { return "\(pulse.agentName) is thinking…" }
        var line = "\(pulse.agentName): \(text)"
        if let progress = pulse.progress { line += " · \(progress)%" }
        return line
    }
}
