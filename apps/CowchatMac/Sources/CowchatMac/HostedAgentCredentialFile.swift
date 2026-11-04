import Darwin
import Foundation

struct HostedAgentCredentialFile {
    private let directory: URL

    init(directory: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".cowchat", isDirectory: true)) {
        self.directory = directory
    }

    func save(apiKey: String, endpoint: URL) throws {
        guard apiKey.hasPrefix("csoc_") else { throw CowchatConnectionError.invalidResponse }
        let components = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)
        guard components?.scheme == "wss", components?.host != nil,
              components?.path == "/ws", components?.user == nil,
              components?.password == nil, components?.query == nil,
              components?.fragment == nil else {
            throw CowchatConnectionError.invalidResponse
        }
        let data = try JSONEncoder().encode(Record(url: endpoint.absoluteString, apiKey: apiKey))
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

    private struct Record: Encodable {
        let url: String
        let apiKey: String

        enum CodingKeys: String, CodingKey {
            case url
            case apiKey = "api_key"
        }
    }
}
