import Foundation
import CowchatKit

struct ConnectionRecord: Codable, Identifiable, Equatable {
    let id: String
    var server: ServerChoice
    /// The label the app shows for this connection.
    var name: String
    /// The name this connection registers with its server.
    var displayName: String
    /// An invite made this connection, so removing the invited room removes the connection.
    var fromInvite = false
    /// The account a sign-in made this connection for, so signing in to it again replaces this connection.
    var accountID: String?

    init(
        id: String, server: ServerChoice, name: String, displayName: String, fromInvite: Bool = false,
        accountID: String? = nil
    ) {
        self.accountID = accountID
        self.id = id
        self.server = server
        self.name = name
        self.displayName = displayName
        self.fromInvite = fromInvite
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        server = try values.decode(ServerChoice.self, forKey: .server)
        name = try values.decode(String.self, forKey: .name)
        displayName = try values.decode(String.self, forKey: .displayName)
        fromInvite = try values.decodeIfPresent(Bool.self, forKey: .fromInvite) ?? false
        accountID = try values.decodeIfPresent(String.self, forKey: .accountID)
    }
}

/// How a connection was made, which is what tells two connections to one server apart.
enum ConnectionSource: Equatable {
    case account(handle: String?)
    case invite
    /// A pasted key, known by its last four characters, as a bank shows an account number.
    case key(ending: String?)

    var label: String {
        switch self {
        case .account(let handle?): "@\(handle)"
        case .account(nil): "Account"
        case .invite: "Invited"
        case .key(let ending?): "Key ••••\(ending)"
        case .key(nil): "Key"
        }
    }
}

struct RoomRef: Hashable, Codable {
    let connectionID: String
    let roomID: String
}

struct MergedRoom: Identifiable, Equatable {
    let ref: RoomRef
    let connectionName: String
    let summary: RoomSummary

    var id: RoomRef { ref }
}

/// A room a connection's key may join but has not.
struct JoinableRoom: Identifiable, Equatable {
    let ref: RoomRef
    let name: String
    let connectionName: String

    var id: RoomRef { ref }
}

enum PastedCredential: Equatable {
    case invite(String)
    case accountKey(String)
    case key(String)
    case unrecognized

    static func detect(_ pasted: String) -> PastedCredential {
        let trimmed = pasted.trimmingCharacters(in: .whitespacesAndNewlines)
        // The Mac app's "Copy connect prompt" wraps the token in a paragraph.
        if let match = trimmed.firstMatch(of: /c(?:inv|guest_invite)_[A-Za-z0-9_-]+/) {
            return .invite(String(match.output))
        }
        guard !trimmed.isEmpty, !trimmed.contains(where: \.isWhitespace) else { return .unrecognized }
        return trimmed.hasPrefix("csoc_") ? .accountKey(trimmed) : .key(trimmed)
    }

    var found: String? {
        switch self {
        case .invite: "Invite"
        case .accountKey: "Account key"
        case .key: "Key"
        case .unrecognized: nil
        }
    }

    var credential: Credential? {
        switch self {
        case .invite(let token): .invite(token)
        case .accountKey(let key), .key(let key): .key(key)
        case .unrecognized: nil
        }
    }
}

extension ServerChoice {
    func isSameServer(as other: ServerChoice) -> Bool {
        switch (self, other) {
        case (.global(let a), .global(let b)):
            return Self.normalized(a) == Self.normalized(b)
        case (.tcp(let hostA, let portA), .tcp(let hostB, let portB)):
            return hostA.lowercased() == hostB.lowercased() && portA == portB
        default:
            return false
        }
    }

    var label: String {
        switch self {
        case .global(let url): URLComponents(string: url)?.host ?? url
        case .tcp(let host, let port): "\(host):\(port)"
        }
    }

    private static func normalized(_ url: String) -> String {
        var value = url.trimmingCharacters(in: .whitespaces).lowercased()
        while value.hasSuffix("/") { value.removeLast() }
        return value
    }
}
