import CryptoKit
import Foundation
import Security

public struct SocialAccount: Codable, Identifiable, Equatable {
    public let id: String
    public var handle: String
    public var displayName: String

    public init(id: String, handle: String, displayName: String) {
        self.id = id
        self.handle = handle
        self.displayName = displayName
    }
}
public struct SocialAgent: Codable, Identifiable, Equatable {
    public let id: String
    public let ownerId: String
    public let name: String
    public let discoverable: Bool

    public init(id: String, ownerId: String, name: String, discoverable: Bool) {
        self.id = id
        self.ownerId = ownerId
        self.name = name
        self.discoverable = discoverable
    }
}
public struct SocialFriend: Decodable, Identifiable {
    public let account: SocialAccount
    public let agents: [SocialAgent]
    public var id: String { account.id }

    public init(account: SocialAccount, agents: [SocialAgent]) {
        self.account = account
        self.agents = agents
    }
}
public struct SocialFriendRequest: Decodable, Identifiable {
    public let id: String
    public let from: SocialAccount
    public let to: SocialAccount
    public let status: String

    public init(id: String, from: SocialAccount, to: SocialAccount, status: String) {
        self.id = id
        self.from = from
        self.to = to
        self.status = status
    }
}
public struct SocialRoom: Decodable, Identifiable {
    /// The session an account room was made for, which the server records when the session registers and keeps for
    /// the room's life (navigation note, section 4.1; a question for Chad beside his task 4).
    public struct Session: Decodable, Equatable {
        public let agentId: String
        public let harness: String
        public let folder: String
        public let branch: String
        public let startedAt: String
        /// When the session ended, or nil while it is live. A resumed session clears it.
        public let endedAt: String?
    }

    public let roomId: String
    public let name: String
    public let ownerId: String
    /// "session" or "company" (Chad's task 4), or nil for any other room.
    public var kind: String?
    public var session: Session?
    public var id: String { roomId }

    public init(roomId: String, name: String, ownerId: String, kind: String? = nil, session: Session? = nil) {
        self.roomId = roomId
        self.name = name
        self.ownerId = ownerId
        self.kind = kind
        self.session = session
    }

    /// Whether the room was made for one session, which is what makes an agent's marked message count in the Inbox.
    public var isSessionRoom: Bool { kind == "session" }
}
public struct SocialRoomInvitation: Decodable, Identifiable {
    public let id: String
    public let roomId: String
    public let roomName: String
    public let from: SocialAccount
    public let to: SocialAccount
    public let proposedAgents: [SocialAgent]
    public let members: [SocialAccount]
    public let agents: [SocialAgent]
    public let historyPolicy: String
    public let encrypted: Bool
    public let slackConnected: Bool
    public let status: String

    public init(id: String, roomId: String, roomName: String, from: SocialAccount, to: SocialAccount, proposedAgents: [SocialAgent], members: [SocialAccount], agents: [SocialAgent], historyPolicy: String, encrypted: Bool, slackConnected: Bool, status: String) {
        self.id = id
        self.roomId = roomId
        self.roomName = roomName
        self.from = from
        self.to = to
        self.proposedAgents = proposedAgents
        self.members = members
        self.agents = agents
        self.historyPolicy = historyPolicy
        self.encrypted = encrypted
        self.slackConnected = slackConnected
        self.status = status
    }
}
public struct SocialParticipants: Decodable {
    public let roomId: String
    public let name: String
    public let ownerId: String
    public let historyPolicy: String
    public let encrypted: Bool
    public let slackConnected: Bool
    public let members: [SocialAccount]
    public let agents: [SocialAgent]

    public init(roomId: String, name: String, ownerId: String, historyPolicy: String, encrypted: Bool, slackConnected: Bool, members: [SocialAccount], agents: [SocialAgent]) {
        self.roomId = roomId
        self.name = name
        self.ownerId = ownerId
        self.historyPolicy = historyPolicy
        self.encrypted = encrypted
        self.slackConnected = slackConnected
        self.members = members
        self.agents = agents
    }
}
public struct SocialMe: Decodable {
    public struct Identity: Decodable, Equatable { public let provider: String; public let subject: String; public let label: String }
    public let account: SocialAccount
    public let identities: [Identity]

    public init(account: SocialAccount, identities: [Identity]) {
        self.account = account
        self.identities = identities
    }
}
public struct SocialCapabilities: Decodable {
    public let providers: [String]
    public let setupVersion: Int?
    public let handleAvailability: Bool?
    public let friendLinks: Bool?
    public let setupHandle: Bool?

    public init(providers: [String], setupVersion: Int?, handleAvailability: Bool?, friendLinks: Bool?, setupHandle: Bool?) {
        self.providers = providers
        self.setupVersion = setupVersion
        self.handleAvailability = handleAvailability
        self.friendLinks = friendLinks
        self.setupHandle = setupHandle
    }
}
public struct SocialHandleAvailability: Decodable { public let handle: String; public let available: Bool; public let valid: Bool; public init(handle: String, available: Bool, valid: Bool) { self.handle = handle; self.available = available; self.valid = valid } }
public struct SocialFriendLink: Decodable, Equatable { public let slug: String; public let url: URL; public init(slug: String, url: URL) { self.slug = slug; self.url = url } }
public enum SocialFriendLinkState: Equatable {
    case idle, loading, loaded(SocialFriendLink), failed(String)
}
public enum SocialFriendRelationship: String, Decodable {
    case `self`, friends
    case outgoingPending = "outgoing_pending"
    case incomingPending = "incoming_pending"
    case none
}
public struct SocialFriendLinkPreview: Decodable { public let account: SocialAccount; public let relationship: SocialFriendRelationship; public init(account: SocialAccount, relationship: SocialFriendRelationship) { self.account = account; self.relationship = relationship } }
public enum SocialHandleCheck: Equatable {
    case idle, checking(String), available(String), unavailable(String), invalid(String), failed(String)
    public func permits(_ input: String) -> Bool { self == .available(input) }
    public var message: String {
        switch self {
        case .idle: return "Choose your public username."
        case .checking: return "Checking username…"
        case .available: return "Username available."
        case .unavailable: return "That username is taken. Try another."
        case .invalid: return "Use 3–40 letters, numbers, hyphens or underscores."
        case .failed: return "Could not check. Edit the username to try again."
        }
    }
}
/// What `POST /social/v1/setup` made: the account's bot, and no room, since every session gets its own room when it
/// registers (Chad's task 4).
public struct SocialSetup: Decodable {
    public let agentId: String
    public let agentName: String
    public let agent: SocialAgent?
    public let botAdmitted: Bool

    public init(agentId: String, agentName: String, agent: SocialAgent?, botAdmitted: Bool) {
        self.agentId = agentId
        self.agentName = agentName
        self.agent = agent
        self.botAdmitted = botAdmitted
    }
}
public struct SocialSetupReply: Decodable { public let setup: SocialSetup? }

public enum SocialStarterNames {
    public static func stem(_ name: String) -> String {
        var value = ""
        for scalar in name.unicodeScalars {
            if scalar.properties.isAlphabetic || scalar.properties.numericType != nil {
                value.unicodeScalars.append(scalar)
            } else if !value.isEmpty && !value.hasSuffix("_") {
                value += "_"
            }
            if value.unicodeScalars.count >= 70 { break }
        }
        value = value.trimmingCharacters(in: CharacterSet(charactersIn: "_"))
        return value.isEmpty ? "Cowboy" : value
    }
    public static func bot(_ name: String) -> String { stem(name) + "_bot" }
}
public struct SocialAcknowledgement: Decodable { public let ok: Bool }
public struct SocialAgents: Decodable { public let agents: [SocialAgent] }
public struct SocialFriends: Decodable { public let friends: [SocialFriend] }
public struct SocialRequests: Decodable { public let requests: [SocialFriendRequest] }
public struct SocialInvitations: Decodable { public let invitations: [SocialRoomInvitation] }
public struct SocialRooms: Decodable { public let rooms: [SocialRoom] }
public struct SocialCreatedAgent: Decodable { public let agent: SocialAgent; public let apiKey: String }
public struct SocialOAuthStart: Decodable { public let requestId: String; public let authorizationUrl: URL; public let expiresIn: Int; public init(requestId: String, authorizationUrl: URL, expiresIn: Int) { self.requestId = requestId; self.authorizationUrl = authorizationUrl; self.expiresIn = expiresIn } }
public struct SocialOAuthExchange: Decodable {
    public let status: String
    public let deviceKey: String?
    public let account: SocialAccount?

    public init(status: String, deviceKey: String?, account: SocialAccount?) {
        self.status = status
        self.deviceKey = deviceKey
        self.account = account
    }
}

public enum SocialAPIError: LocalizedError {
    case friendsUnavailable

    public var errorDescription: String? {
        "This Cowchat server needs an update before Friends is available."
    }
}

public final class SocialHTTPDelegate: NSObject, URLSessionTaskDelegate {
    public func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

public struct SocialAPI {
    public static let secureSession = URLSession(configuration: .ephemeral, delegate: SocialHTTPDelegate(), delegateQueue: nil)
    public let endpoint: URL
    public let key: String?
    public var session: URLSession = secureSession

    public init(endpoint: URL, key: String? = nil, session: URLSession = secureSession) throws {
        guard endpoint.scheme == "wss", endpoint.host != nil, endpoint.user == nil,
              endpoint.password == nil, endpoint.query == nil, endpoint.fragment == nil else {
            throw ConnectionProfileError.invalidCloudURL
        }
        self.endpoint = endpoint
        self.key = key
        self.session = session
    }

    public func makeRequest(_ path: [String], auth: Bool = true, method: String = "GET", body: [String: Any]? = nil,
                     query: [URLQueryItem] = []) throws -> URLRequest {
        var components = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)!
        components.scheme = "https"
        components.path = ""
        var url = components.url!
        for segment in path { url.appendPathComponent(segment) }
        if !query.isEmpty, var withQuery = URLComponents(url: url, resolvingAgainstBaseURL: false) {
            withQuery.queryItems = query
            url = withQuery.url ?? url
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.timeoutInterval = 20
        if auth, let key { request.setValue(key, forHTTPHeaderField: "x-cowchat-key") }
        if let body {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        return request
    }

    public func request<T: Decodable>(_ path: [String], auth: Bool = true, method: String = "GET", body: [String: Any]? = nil,
                               query: [URLQueryItem] = []) async throws -> T {
        let request = try makeRequest(path, auth: auth, method: method, body: body, query: query)
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw CowchatConnectionError.invalidResponse }
        if path == ["social", "v1", "capabilities"],
           http.statusCode == 404 || (http.statusCode == 200 && http.mimeType == "text/html") {
            throw SocialAPIError.friendsUnavailable
        }
        guard http.statusCode == 200 else {
            let detail = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String
            throw CowchatConnectionError.server(message: detail ?? "Cowchat account request failed (HTTP \(http.statusCode)).", code: nil)
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(T.self, from: data)
    }

    public func social<T: Decodable>(_ path: [String], method: String = "GET", body: [String: Any]? = nil,
                              query: [URLQueryItem] = []) async throws -> T {
        try await request(["social", "v1"] + path, method: method, body: body, query: query)
    }

    public static func normalizedHandle(_ input: String) -> String {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.hasPrefix("@") ? String(trimmed.dropFirst()) : trimmed
    }

    public static func verifier() throws -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
            throw CowchatConnectionError.transport("Could not create a secure sign-in request.")
        }
        return base64URL(Data(bytes))
    }

    public static func challenge(_ verifier: String) -> String {
        base64URL(Data(SHA256.hash(data: Data(verifier.utf8))))
    }
    private static func base64URL(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }
}
