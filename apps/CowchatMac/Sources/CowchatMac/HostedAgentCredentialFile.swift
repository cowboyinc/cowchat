import Darwin
import Foundation

struct HostedAgentCredentialFile {
    private let directory: URL

    init(directory: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".cowchat", isDirectory: true)) {
        self.directory = directory
    }

    func save(apiKey: String, endpoint: URL, accountID: String, agentID: String) throws {
        guard apiKey.hasPrefix("csoc_") else { throw CowchatConnectionError.invalidResponse }
        let components = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)
        guard components?.scheme == "wss", components?.host != nil,
              components?.path == "/ws", components?.user == nil,
              components?.password == nil, components?.query == nil,
              components?.fragment == nil else {
            throw CowchatConnectionError.invalidResponse
        }
        let data = try JSONEncoder().encode(Record(url: endpoint.absoluteString, apiKey: apiKey, accountID: accountID, agentID: agentID, kind: "mint_only"))
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let temporary = directory.appendingPathComponent(".hosted-bot-\(UUID().uuidString).tmp")
        let destination = directory.appendingPathComponent("hosted-bot.json")
        guard FileManager.default.createFile(atPath: temporary.path, contents: data, attributes: [.posixPermissions: 0o600]) else {
            throw CocoaError(.fileWriteUnknown)
        }
        defer { try? FileManager.default.removeItem(at: temporary) }
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: temporary.path)
        let result = temporary.path.withCString { source in
            destination.path.withCString { target in rename(source, target) }
        }
        guard result == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
    }

    func remove() throws {
        let path = directory.appendingPathComponent("hosted-bot.json")
        if FileManager.default.fileExists(atPath: path.path) {
            try FileManager.default.removeItem(at: path)
        }
    }

    func load() throws -> Record? {
        let path = directory.appendingPathComponent("hosted-bot.json")
        guard FileManager.default.fileExists(atPath: path.path) else { return nil }
        return try JSONDecoder().decode(Record.self, from: Data(contentsOf: path))
    }

    struct Record: Codable {
        let url: String
        let apiKey: String
        let accountID: String?
        let agentID: String?
        let kind: String?

        enum CodingKeys: String, CodingKey {
            case url
            case apiKey = "api_key"
            case accountID = "account_id"
            case agentID = "agent_id"
            case kind
        }
    }
}
