import Foundation


/// Who sent a message, as the server records it in `metadata.sender_kind` from the connection that sent it: a person
/// through the Mac app, or an agent.
public enum MessageSenderKind: String, Codable, Equatable {
    case person
    case agent
}

/// A pull request a message carries as finished work, in `metadata.pull_request`. The checks are what the agent saw
/// when it sent the message; the app does not ask GitHub.
public struct MessagePullRequest: Codable, Equatable {
    public enum Checks: String, Codable, Equatable {
        case passing, failing, running
    }

    /// The repository as GitHub names it, "cowboyinc/cowchat", or its name alone.
    public let repository: String
    public let number: Int
    public let title: String
    /// Nil when the agent sent none, or a state this app does not know.
    public let checks: Checks?
    /// Lines added and removed.
    public let additions: Int?
    public let deletions: Int?
    public let address: String

    public enum CodingKeys: String, CodingKey {
        case repository, number, title, checks, additions, deletions, address
    }

    public init(repository: String, number: Int, title: String, checks: Checks?, additions: Int?, deletions: Int?,
         address: String) {
        self.repository = repository
        self.number = number
        self.title = title
        self.checks = checks
        self.additions = additions
        self.deletions = deletions
        self.address = address
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        repository = try values.decode(String.self, forKey: .repository)
        number = try values.decode(Int.self, forKey: .number)
        title = try values.decode(String.self, forKey: .title)
        address = try values.decode(String.self, forKey: .address)
        checks = try? values.decodeIfPresent(Checks.self, forKey: .checks)
        additions = try? values.decodeIfPresent(Int.self, forKey: .additions)
        deletions = try? values.decodeIfPresent(Int.self, forKey: .deletions)
    }

    /// The repository's own name, "cowchat".
    public var repositoryName: String {
        repository.split(separator: "/").last.map(String.init) ?? repository
    }

    // copy:
    /// The row's name for it: "cowchat #1419".
    public var name: String { "\(repositoryName) #\(number)" }

    /// The address, when it is a web address a browser opens.
    public var url: URL? {
        guard let url = URL(string: address), let scheme = url.scheme?.lowercased(),
              scheme == "https" || scheme == "http", url.host?.isEmpty == false else { return nil }
        return url
    }

    // copy:
    /// What the checks mark says to the hover and a screen reader.
    public var checksLabel: String? {
        switch checks {
        case .passing: "Checks passed"
        case .failing: "Checks failed"
        case .running: "Checks running"
        case nil: nil
        }
    }

    /// The diff size as the row shows it, "+214" and "−38", or nil when the agent sent none.
    public var diff: (added: String, removed: String)? {
        guard let additions, let deletions else { return nil }
        return ("+\(additions)", "\u{2212}\(deletions)")
    }
}

/// A file a message carries under its text, in `metadata.attachments`, with the fields a file message's metadata has.
public struct MessageAttachment: Codable, Equatable {
    public let blobID: String
    public let name: String?
    public let size: Int?
    public let sha256: String?

    public enum CodingKeys: String, CodingKey {
        case name, size, sha256
        case blobID = "blob_id"
    }

    public init(blobID: String, name: String?, size: Int?, sha256: String?) {
        self.blobID = blobID
        self.name = name
        self.size = size
        self.sha256 = sha256
    }

    public var file: FileAttachment? {
        guard !blobID.isEmpty else { return nil }
        return FileAttachment(blobID: blobID, name: name?.isEmpty == false ? name! : "attachment", size: size)
    }

    /// One entry of the list, kept when it reads and dropped alone when it does not.
    public struct Lenient: Decodable {
        public let attachment: MessageAttachment?

        public init(from decoder: Decoder) throws {
            attachment = try? MessageAttachment(from: decoder)
        }
    }
}

extension FileAttachment {
    /// Whether it is a picture the feed draws as a thumbnail, by its name.
    public var isImage: Bool {
        let ext = (name as NSString).pathExtension.lowercased()
        return ["png", "jpg", "jpeg", "gif", "heic", "webp", "tiff", "bmp"].contains(ext)
    }
}

extension ChatMessage {
    /// The files listed on the message, for drawing under its text.
    public var attachments: [FileAttachment] { metadata.attachments.compactMap(\.file) }
}

/// Which messages a room shows. Every message from a person shows, and every agent message marked for people. A room's
/// switch shows the rest.
public enum MessageVisibility {
    /// What an agent's message with no `display_external` flag means is Chad's to settle (his plan, task 8). The skill
    /// says such a message stays in Cowchat, so until he does the room hides it with the unmarked ones. This is the one
    /// place that guess is made. Slack forwarding reads the flag on its own (`MessageMetadata.shouldDisplayExternal`).
    public static let missingFlagShows = false

    /// Whether a room shows the message with its switch off: from a person always, from a sender the server did not
    /// record always, since older messages carry no kind, and from an agent only when it marked the message.
    public static func showsByDefault(_ message: ChatMessage) -> Bool {
        guard message.metadata.senderKind == .agent else { return true }
        return message.metadata.displayExternal ?? missingFlagShows
    }
}

/// Messages a room hides, one after another between messages it shows, drawn as one quiet row that opens in place. It
/// is named by its first message, so later messages join the same run.
public struct HiddenMessageRun: Identifiable, Equatable {
    public let id: String
    public let messages: [ChatMessage]
    public let isOpen: Bool

    // copy:
    public var title: String { messages.count == 1 ? "1 agent message" : "\(messages.count) agent messages" }
}

/// One row of a room's feed.
public enum MessageFeedItem: Identifiable, Equatable {
    /// A message. A quiet one is one the room hides by default, drawn because its run is open or the switch is on.
    case message(ChatMessage, quiet: Bool)
    case run(HiddenMessageRun)

    public var id: String {
        switch self {
        case .message(let message, _): message.id
        case .run(let run): "run/\(run.id)"
        }
    }
}

public enum MessageFeed {
    /// A room's messages as feed rows: each message the room shows, each run of hidden ones as its row, and an open
    /// run's messages after its row. With the switch on every message is a row, the hidden ones quiet.
    public static func items(_ messages: [ChatMessage], showsAll: Bool, openRuns: Set<String>) -> [MessageFeedItem] {
        var items: [MessageFeedItem] = []
        var run: [ChatMessage] = []
        func closeRun() {
            guard let first = run.first else { return }
            let isOpen = openRuns.contains(first.id)
            items.append(.run(HiddenMessageRun(id: first.id, messages: run, isOpen: isOpen)))
            if isOpen { items += run.map { .message($0, quiet: true) } }
            run = []
        }
        for message in messages {
            let shows = MessageVisibility.showsByDefault(message)
            if shows || showsAll {
                closeRun()
                items.append(.message(message, quiet: !shows))
            } else {
                run.append(message)
            }
        }
        closeRun()
        return items
    }
}
