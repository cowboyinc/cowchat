import Foundation
import CowchatKit

extension RoomTimeline {
    func merging(_ incoming: [TimelineEntry]) -> RoomTimeline {
        var byID = Dictionary(entries.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
        for entry in incoming { byID[entry.id] = entry }
        var merged = self
        merged.entries = byID.values.sorted { ($0.seq, $0.id) < ($1.seq, $1.id) }
        return merged
    }
}

extension TimelineEntry {
    init(message: ChatMessage, myAgentID: String?, secret: String?) {
        var plain: String? = message.content
        if message.content.hasPrefix("cow1:") {
            plain = secret.flatMap { try? RoomCrypto.decrypt(message.content, secret: $0, roomID: message.roomID) }
        }
        let body: Body
        if let plain, let chart = ChartPayload.read(plain) {
            body = .chart(title: chart.title, png: chart.png)
        } else {
            body = plain.map(Body.text) ?? .locked
        }
        let listed = message.attachments
        let files = (message.fileAttachment.map { [$0] } ?? []).filter { file in !listed.contains(file) } + listed
        self.init(
            id: message.messageID,
            seq: message.seq,
            senderID: message.agentID,
            senderName: message.agentName,
            isMine: myAgentID != nil && message.agentID == myAgentID,
            date: CowchatDate.parse(message.timestamp) ?? Date(timeIntervalSince1970: 0),
            body: body,
            isForPeople: MessageVisibility.showsByDefault(message),
            isAgent: message.metadata.senderKind == .agent,
            pullRequest: message.metadata.pullRequest,
            attachments: files
        )
    }
}

enum CowchatDate {
    static func parse(_ string: String?) -> Date? {
        guard let string, !string.isEmpty else { return nil }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: string) ?? ISO8601DateFormatter().date(from: string)
    }
}

enum ReconnectBackoff {
    static func delay(afterFailures failures: Int) -> Duration {
        .seconds(min(30, 1 << min(failures, 5)))
    }
}
