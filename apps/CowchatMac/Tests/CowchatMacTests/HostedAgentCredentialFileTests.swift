import Foundation
import XCTest
@testable import CowchatMac

final class HostedAgentCredentialFileTests: XCTestCase {
    func testSaveBindsCredentialToHostedURLAndRemovesIt() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = HostedAgentCredentialFile(directory: directory)
        try file.save(apiKey: "csoc_test", endpoint: URL(string: "wss://chat.example/ws")!)

        let path = directory.appendingPathComponent("hosted-bot.json")
        let values = try JSONSerialization.jsonObject(with: Data(contentsOf: path)) as! [String: String]
        XCTAssertEqual(values["url"], "wss://chat.example/ws")
        XCTAssertEqual(values["api_key"], "csoc_test")
        let attributes = try FileManager.default.attributesOfItem(atPath: path.path)
        XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, 0o600)

        try file.remove()
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
    }
}
