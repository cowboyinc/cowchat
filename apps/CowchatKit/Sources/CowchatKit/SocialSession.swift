import Foundation

public struct SocialSession: Codable, Equatable {
    public let endpoint: URL
    public let deviceKey: String
    public let account: SocialAccount
    public let scopeId: String

    public init(endpoint: URL, deviceKey: String, account: SocialAccount, scopeId: String = UUID().uuidString.lowercased()) {
        self.endpoint = endpoint
        self.deviceKey = deviceKey
        self.account = account
        self.scopeId = scopeId
    }
}
