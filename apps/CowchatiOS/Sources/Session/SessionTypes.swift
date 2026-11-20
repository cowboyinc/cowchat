import Foundation
import CowchatKit

enum ServerChoice: Equatable, Codable {
    case global(url: String)
    case tcp(host: String, port: UInt16)

    static let defaultGlobal = ServerChoice.global(url: ConnectionProfile.defaultGlobalURLString)
}

enum Credential: Equatable {
    case key(String)
    case invite(String)
}

enum SessionPhase: Equatable {
    case signedOut
    case connecting
    case connected
    case reconnecting
    case failed(String)
}

struct RoomSummary: Identifiable, Equatable {
    let id: String
    var name: String
    var isEncrypted: Bool
    var hasSecret: Bool
    var isUnread: Bool
    var lastActivity: Date?
    var isArchived = false
    /// For a session's room: `working`, `prompt` (waiting for its person) or `idle`.
    var sessionState: String?
}

struct TimelineEntry: Identifiable, Equatable {
    enum Body: Equatable {
        case text(String)
        case locked
        case chart(title: String, png: Data)
    }

    let id: String
    let seq: Int
    let senderID: String
    let senderName: String
    let isMine: Bool
    let date: Date
    let body: Body
    /// False for an agent message its sender did not mark for people. The room folds those away.
    var isForPeople = true
    /// True when the server labelled the sender an agent. It labels senders only on account and guest connections.
    var isAgent = false
    var pullRequest: MessagePullRequest?
    var attachments: [FileAttachment] = []
}

/// A chart sent as a message: `cowchat-chart:v1`, a newline, then JSON holding a title and a PNG. The checks match
/// the Mac app's reader.
enum ChartPayload {
    static let prefix = "cowchat-chart:v1\n"
    static let maxPNGBytes = 256 * 1024

    static func read(_ content: String) -> (title: String, png: Data)? {
        guard content.hasPrefix(prefix),
              content.utf8.count <= maxPNGBytes * 4 / 3 + 132 * 1024,
              let data = String(content.dropFirst(prefix.count)).data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: String],
              Set(object.keys).isSubset(of: ["title", "png", "openui"]),
              let title = object["title"], !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              title.unicodeScalars.count <= 200,
              title.unicodeScalars.allSatisfy({ $0.value > 31 && !(127...159).contains($0.value) }),
              let encoded = object["png"], let png = Data(base64Encoded: encoded), png.base64EncodedString() == encoded,
              png.count >= 33, png.count <= maxPNGBytes,
              Array(png.prefix(16)) == [137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82]
        else { return nil }
        let bytes = [UInt8](png)
        let width = bytes[16..<20].reduce(UInt64(0)) { ($0 << 8) | UInt64($1) }
        let height = bytes[20..<24].reduce(UInt64(0)) { ($0 << 8) | UInt64($1) }
        guard width > 0, height > 0, width <= 4096, height <= 4096, width * height <= 8_000_000 else { return nil }
        return (title, png)
    }
}

struct RoomMember: Identifiable, Equatable {
    let id: String
    let name: String
    var status: String?

    var isActive: Bool { ["working", "thinking"].contains(status?.lowercased() ?? "") }
}

/// One row of a room's feed: a message, or a folded run of agent messages named by its first message.
enum FeedItem: Identifiable, Equatable {
    case entry(TimelineEntry, quiet: Bool)
    case run(id: String, count: Int, isOpen: Bool)

    var id: String {
        switch self {
        case .entry(let entry, _): entry.id
        case .run(let id, _, _): "run/\(id)"
        }
    }
}

enum TimelineFeed {
    static func items(_ entries: [TimelineEntry], showsAll: Bool, openRuns: Set<String>) -> [FeedItem] {
        var items: [FeedItem] = []
        var run: [TimelineEntry] = []
        func closeRun() {
            guard let first = run.first else { return }
            let isOpen = openRuns.contains(first.id)
            items.append(.run(id: first.id, count: run.count, isOpen: isOpen))
            if isOpen { items += run.map { .entry($0, quiet: true) } }
            run = []
        }
        for entry in entries {
            if entry.isForPeople || showsAll {
                closeRun()
                items.append(.entry(entry, quiet: !entry.isForPeople))
            } else {
                run.append(entry)
            }
        }
        closeRun()
        return items
    }

    static func runTitle(count: Int) -> String { count == 1 ? "1 agent message" : "\(count) agent messages" }
}

enum PresenceLine {
    static func summary(members: [RoomMember], myAgentID: String?, phase: SessionPhase) -> String {
        guard phase == .connected else { return "Offline" }
        let others = members.filter { $0.id != myAgentID }
        let active = others.filter(\.isActive)
        if !active.isEmpty { return active.prefix(2).map(\.name).joined(separator: ", ") + " active" }
        switch others.count {
        case 0: return "No agents connected"
        case 1: return "1 agent connected"
        default: return "\(others.count) agents connected"
        }
    }
}

struct RoomTimeline: Equatable {
    var entries: [TimelineEntry] = []
    var hasEarlier = true
    var isLoading = false
}
