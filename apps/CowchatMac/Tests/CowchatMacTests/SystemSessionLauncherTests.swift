import Foundation
import XCTest
@testable import CowchatMac

final class SystemSessionLauncherTests: XCTestCase {
    @MainActor
    func testLaunchPassesTheFirstMessageAsOneArgumentInTheChosenFolder() async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("cowchat-launch-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: home) }
        let bin = home.appendingPathComponent(".local/bin")
        let folder = home.appendingPathComponent("project")
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let result = home.appendingPathComponent("arguments.txt")
        let marker = home.appendingPathComponent("injected")
        let agent = bin.appendingPathComponent("claude")
        try "#!/bin/zsh\nprintf '%s\\n' \"$PWD\" \"$1\" > '\(result.path)'\n"
            .write(to: agent, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: agent.path)
        let launcher = SystemSessionLauncher(home: home) { script in
            let process = Process()
            process.executableURL = script
            try process.run()
            process.waitUntilExit()
            XCTAssertEqual(process.terminationStatus, 0)
        }
        let message = "hello'; touch \(marker.path); echo 'world"
        try await launcher.start(SessionRequest(folder: SessionFolder(path: folder), harness: .claudeCode,
                                                branch: nil, firstMessage: message))
        let lines = try String(contentsOf: result, encoding: .utf8).split(separator: "\n", omittingEmptySubsequences: false)
        XCTAssertEqual(String(lines[0]), folder.path)
        XCTAssertEqual(String(lines[1]), message)
        XCTAssertFalse(FileManager.default.fileExists(atPath: marker.path))
        XCTAssertEqual(launcher.path(ofFolder: folder.lastPathComponent), folder)
    }
}
