import AppKit
import Foundation

/// Who sent a message, as the server records it in `metadata.sender_kind` from the connection that sent it: a person
/// through the Mac app, or an agent.
enum MessageSenderKind: String, Codable, Equatable {
    case person
    case agent
}

/// A pull request a message carries as finished work, in `metadata.pull_request`. The checks are what the agent saw
/// when it sent the message; the app does not ask GitHub.
struct MessagePullRequest: Codable, Equatable {
    enum Checks: String, Codable, Equatable {
        case passing, failing, running
    }

    /// The repository as GitHub names it, "cowboyinc/cowchat", or its name alone.
    let repository: String
    let number: Int
    let title: String
    /// Nil when the agent sent none, or a state this app does not know.
    let checks: Checks?
    /// Lines added and removed.
    let additions: Int?
    let deletions: Int?
    let address: String

    enum CodingKeys: String, CodingKey {
        case repository, number, title, checks, additions, deletions, address
    }

    init(repository: String, number: Int, title: String, checks: Checks?, additions: Int?, deletions: Int?,
         address: String) {
        self.repository = repository
        self.number = number
        self.title = title
        self.checks = checks
        self.additions = additions
        self.deletions = deletions
        self.address = address
    }

    init(from decoder: Decoder) throws {
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
    var repositoryName: String {
        repository.split(separator: "/").last.map(String.init) ?? repository
    }

    // copy:
    /// The row's name for it: "cowchat #1419".
    var name: String { "\(repositoryName) #\(number)" }

    /// The address, when it is a web address a browser opens.
    var url: URL? {
        guard let url = URL(string: address), let scheme = url.scheme?.lowercased(),
              scheme == "https" || scheme == "http", url.host?.isEmpty == false else { return nil }
        return url
    }

    // copy:
    /// What the checks mark says to the hover and a screen reader.
    var checksLabel: String? {
        switch checks {
        case .passing: "Checks passed"
        case .failing: "Checks failed"
        case .running: "Checks running"
        case nil: nil
        }
    }

    /// The diff size as the row shows it, "+214" and "−38", or nil when the agent sent none.
    var diff: (added: String, removed: String)? {
        guard let additions, let deletions else { return nil }
        return ("+\(additions)", "\u{2212}\(deletions)")
    }
}

/// A file a message carries under its text, in `metadata.attachments`, with the fields a file message's metadata has.
struct MessageAttachment: Codable, Equatable {
    let blobID: String
    let name: String?
    let size: Int?
    let sha256: String?

    enum CodingKeys: String, CodingKey {
        case name, size, sha256
        case blobID = "blob_id"
    }

    init(blobID: String, name: String?, size: Int?, sha256: String?) {
        self.blobID = blobID
        self.name = name
        self.size = size
        self.sha256 = sha256
    }

    var file: FileAttachment? {
        guard !blobID.isEmpty else { return nil }
        return FileAttachment(blobID: blobID, name: name?.isEmpty == false ? name! : "attachment", size: size)
    }

    /// One entry of the list, kept when it reads and dropped alone when it does not.
    struct Lenient: Decodable {
        let attachment: MessageAttachment?

        init(from decoder: Decoder) throws {
            attachment = try? MessageAttachment(from: decoder)
        }
    }
}

/// A label on a message, in `metadata.tags`.
struct MessageTag: Codable, Equatable {
    let key: String
    let value: String?

    /// How a filter names it: "github=core#1", or the key alone when it has no value.
    var label: String { value.map { "\(key)=\($0)" } ?? key }

    /// Whether a filter picks this tag: its whole label, or its key for any value.
    func matches(_ filter: String) -> Bool { filter == label || filter == key }

    /// One entry of the list, kept when it reads and dropped alone when it does not.
    struct Lenient: Decodable {
        let tag: MessageTag?

        init(from decoder: Decoder) throws {
            tag = try? MessageTag(from: decoder)
        }
    }
}

extension FileAttachment {
    /// Whether it is a picture the feed draws as a thumbnail, by its name.
    var isImage: Bool {
        let ext = (name as NSString).pathExtension.lowercased()
        return ["png", "jpg", "jpeg", "gif", "heic", "webp", "tiff", "bmp"].contains(ext)
    }
}

extension ChatMessage {
    /// The files listed on the message, for drawing under its text.
    var attachments: [FileAttachment] { metadata.attachments.compactMap(\.file) }
}

/// Which messages a room shows. Every message from a person shows, and every agent message marked for people. A room's
/// switch shows the rest.
enum MessageVisibility {
    /// What an agent's message with no `display_external` flag means is Chad's to settle (his plan, task 8). The skill
    /// says such a message stays in Cowchat, so until he does the room hides it with the unmarked ones. This is the one
    /// place that guess is made. Slack forwarding reads the flag on its own (`MessageMetadata.shouldDisplayExternal`).
    static let missingFlagShows = false

    /// Whether a room shows the message with its switch off: from a person always, from a sender the server did not
    /// record always, since older messages carry no kind, and from an agent only when it marked the message.
    static func showsByDefault(_ message: ChatMessage) -> Bool {
        guard message.metadata.senderKind == .agent else { return true }
        return message.metadata.displayExternal ?? missingFlagShows
    }
}

/// Messages a room hides, one after another between messages it shows, drawn as one quiet row that opens in place. It
/// is named by its first message, so later messages join the same run.
struct HiddenMessageRun: Identifiable, Equatable {
    let id: String
    let messages: [ChatMessage]
    let isOpen: Bool

    // copy:
    var title: String { messages.count == 1 ? "1 agent message" : "\(messages.count) agent messages" }
}

/// One row of a room's feed.
enum MessageFeedItem: Identifiable, Equatable {
    /// A message. A quiet one is one the room hides by default, drawn because its run is open or the switch is on.
    case message(ChatMessage, quiet: Bool)
    case run(HiddenMessageRun)

    var id: String {
        switch self {
        case .message(let message, _): message.id
        case .run(let run): "run/\(run.id)"
        }
    }
}

enum MessageFeed {
    /// A room's messages as feed rows: each message the room shows, each run of hidden ones as its row, and an open
    /// run's messages after its row. With the switch on every message is a row, the hidden ones quiet. A tag narrows the
    /// feed to the messages carrying it, whether or not the room hides them.
    static func items(_ messages: [ChatMessage], showsAll: Bool, openRuns: Set<String>,
                      tag: String? = nil) -> [MessageFeedItem] {
        if let tag {
            return messages
                .filter { $0.metadata.tags.contains { $0.matches(tag) } }
                .map { .message($0, quiet: false) }
        }
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

    /// The tags on the loaded messages, for the room menu's filter.
    static func tagLabels(_ messages: [ChatMessage]) -> [String] {
        Set(messages.flatMap { $0.metadata.tags.map(\.label) }).sorted()
    }
}

/// Where the app sends a message's web address, such as a pull request's. The prototype replaces it, so a press there
/// opens nothing.
@MainActor
enum MessageLinks {
    static var open: (URL) -> Void = { NSWorkspace.shared.open($0) }
}

/// Where a saved attachment opens when its thumbnail is pressed. Tests replace it.
@MainActor
enum AttachmentPreview {
    static var show: (URL) -> Void = { QuickLookPanel.shared.show($0) }
}

extension ChatStore {
    /// The session thumbnails load over. The prototype points it at its simulated server, as it does the download's.
    static var thumbnailSession: URLSession = .shared
    /// Pictures already loaded, by blob ID. Blobs never change once uploaded.
    private static var thumbnailImages: [String: NSImage] = [:]

    /// A picture attachment for its thumbnail, fetched as a download is but kept in memory, not saved. Nil when the
    /// room's server has no blob route or the blob is not a picture.
    func thumbnailImage(_ attachment: FileAttachment) async -> NSImage? {
        if let image = Self.thumbnailImages[attachment.blobID] { return image }
        guard selectedRoom?.encrypted != true, connectionProfile.kind == .cowchatCloud,
              let url = WorkspaceStore.blobURL(
                  forCloudURLString: connectionProfile.endpointDescription,
                  blobID: attachment.blobID
              )
        else { return nil }
        var request = URLRequest(url: url)
        request.setValue(connectionProfile.apiKey, forHTTPHeaderField: "x-cowchat-key")
        request.timeoutInterval = 60
        guard let (data, response) = try? await Self.thumbnailSession.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let image = NSImage(data: data) else { return nil }
        if Self.thumbnailImages.count >= 200 { Self.thumbnailImages.removeAll() }
        Self.thumbnailImages[attachment.blobID] = image
        return image
    }

    /// Saves a picture attachment where the attachment chip saves files, then opens it in Quick Look, as pressing its
    /// thumbnail does.
    @discardableResult
    func openAttachment(_ attachment: FileAttachment) async throws -> URL {
        let saved = try await downloadAttachment(attachment)
        AttachmentPreview.show(saved)
        return saved
    }
}
