import CryptoKit
import Foundation
import Security

struct SocialAccount: Codable, Identifiable, Equatable {
    let id: String
    var handle: String
    var displayName: String
}
struct SocialAgent: Codable, Identifiable, Equatable {
    let id: String
    let ownerId: String
    let name: String
    let discoverable: Bool
}
struct SocialFriend: Decodable, Identifiable {
    let account: SocialAccount
    let agents: [SocialAgent]
    var id: String { account.id }
}
struct SocialFriendRequest: Decodable, Identifiable {
    let id: String
    let from: SocialAccount
    let to: SocialAccount
    let status: String
}
struct SocialRoom: Decodable, Identifiable {
    let roomId: String
    let name: String
    let ownerId: String
    var id: String { roomId }
}
struct SocialRoomInvitation: Decodable, Identifiable {
    let id: String
    let roomId: String
    let roomName: String
    let from: SocialAccount
    let to: SocialAccount
    let proposedAgents: [SocialAgent]
    let members: [SocialAccount]
    let agents: [SocialAgent]
    let historyPolicy: String
    let encrypted: Bool
    let slackConnected: Bool
    let status: String
}
struct SocialParticipants: Decodable {
    let roomId: String
    let name: String
    let ownerId: String
    let historyPolicy: String
    let encrypted: Bool
    let slackConnected: Bool
    let members: [SocialAccount]
    let agents: [SocialAgent]
}
struct SocialMe: Decodable {
    struct Identity: Decodable, Equatable { let provider: String; let subject: String; let label: String }
    let account: SocialAccount
    let identities: [Identity]
}
struct SocialCapabilities: Decodable { let providers: [String]; let setupVersion: Int? }
struct SocialSetup: Decodable {
    let agentId: String
    let agentName: String
    let roomId: String
    let roomName: String
    let agent: SocialAgent?
    let room: SocialRoom?
    let botAdmitted: Bool
}
struct SocialSetupReply: Decodable { let setup: SocialSetup? }

enum SocialStarterNames {
    static func stem(_ name: String) -> String {
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
    static func bot(_ name: String) -> String { stem(name) + "_bot" }
    static func home(_ name: String) -> String { stem(name) + "_Home" }
}
struct SocialAcknowledgement: Decodable { let ok: Bool }
struct SocialAgents: Decodable { let agents: [SocialAgent] }
struct SocialFriends: Decodable { let friends: [SocialFriend] }
struct SocialRequests: Decodable { let requests: [SocialFriendRequest] }
struct SocialInvitations: Decodable { let invitations: [SocialRoomInvitation] }
struct SocialRooms: Decodable { let rooms: [SocialRoom] }
struct SocialCreatedAgent: Decodable { let agent: SocialAgent; let apiKey: String }
struct SocialOAuthStart: Decodable { let requestId: String; let authorizationUrl: URL; let expiresIn: Int }
struct SocialOAuthExchange: Decodable {
    let status: String
    let deviceKey: String?
    let account: SocialAccount?
}

enum SocialAPIError: LocalizedError {
    case friendsUnavailable

    var errorDescription: String? {
        "This Cowchat server needs an update before Friends is available."
    }
}

final class SocialHTTPDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

struct SocialAPI {
    static let secureSession = URLSession(configuration: .ephemeral, delegate: SocialHTTPDelegate(), delegateQueue: nil)
    let endpoint: URL
    let key: String?
    var session: URLSession = secureSession

    init(endpoint: URL, key: String? = nil, session: URLSession = secureSession) throws {
        guard endpoint.scheme == "wss", endpoint.host != nil, endpoint.user == nil,
              endpoint.password == nil, endpoint.query == nil, endpoint.fragment == nil else {
            throw ConnectionProfileError.invalidCloudURL
        }
        self.endpoint = endpoint
        self.key = key
        self.session = session
    }

    func makeRequest(_ path: [String], auth: Bool = true, method: String = "GET", body: [String: Any]? = nil) throws -> URLRequest {
        var components = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)!
        components.scheme = "https"
        components.path = ""
        var url = components.url!
        for segment in path { url.appendPathComponent(segment) }
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

    func request<T: Decodable>(_ path: [String], auth: Bool = true, method: String = "GET", body: [String: Any]? = nil) async throws -> T {
        let request = try makeRequest(path, auth: auth, method: method, body: body)
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

    func social<T: Decodable>(_ path: [String], method: String = "GET", body: [String: Any]? = nil) async throws -> T {
        try await request(["social", "v1"] + path, method: method, body: body)
    }

    static func normalizedHandle(_ input: String) -> String {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.hasPrefix("@") ? String(trimmed.dropFirst()) : trimmed
    }

    static func verifier() throws -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
            throw CowchatConnectionError.transport("Could not create a secure sign-in request.")
        }
        return base64URL(Data(bytes))
    }

    static func challenge(_ verifier: String) -> String {
        base64URL(Data(SHA256.hash(data: Data(verifier.utf8))))
    }
    private static func base64URL(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }
}
