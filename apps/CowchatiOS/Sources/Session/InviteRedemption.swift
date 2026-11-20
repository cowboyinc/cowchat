import Foundation
import CowchatKit

typealias BlobFetcher = (URLRequest) async throws -> (Data, URLResponse)

typealias InviteRedeemer = (_ endpoint: URL, _ token: String) async throws -> String

enum InviteRedemption {
    static func endpoint(forWebSocketURL url: URL) -> URL? {
        guard var components = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return nil }
        components.scheme = "https"
        components.path = "/api/invites/redeem"
        return components.url
    }

    static func redeem(endpoint: URL, token: String) async throws -> String {
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: ["token": token])
        let (data, response) = try await SocialAPI.secureSession.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        if status == 201, let key = body?["api_key"] as? String, !key.isEmpty {
            return key
        }
        switch status {
        case 404:
            throw SessionError("This invite has already been used or is no longer valid. Ask for a new one.")
        case 429:
            throw SessionError("Too many invite attempts from this network. Wait a few minutes and try again.")
        default:
            let detail = body?["error"] as? String ?? "HTTP \(status)"
            throw SessionError("The server could not redeem this invite (\(detail)).")
        }
    }
}

struct SessionError: LocalizedError, Equatable {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}
