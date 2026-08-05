import Foundation
import XCTest
@testable import CowchatMac

final class CowchatLocalAPIKeyTests: XCTestCase {
    func testLoadTrimsServerKeyFileWhitespace() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let keyURL = directory.appendingPathComponent("auth.key", isDirectory: false)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: directory) }
        try Data("  local-server-key\n".utf8).write(to: keyURL)

        XCTAssertEqual(CowchatLocalAPIKey.load(from: keyURL), "local-server-key")
    }

    func testLoadFallsBackToKeylessWhenServerKeyFileIsMissing() {
        let missingURL = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: false)

        XCTAssertEqual(CowchatLocalAPIKey.load(from: missingURL), "")
    }
}
