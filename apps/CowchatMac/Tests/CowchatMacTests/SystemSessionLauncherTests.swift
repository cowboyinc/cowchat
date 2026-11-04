import Foundation
import XCTest
@testable import CowchatMac

final class SystemSessionLauncherTests: XCTestCase {
    private func git(_ arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = arguments
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
    }

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
        let suite = "CowchatMac.LauncherTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let agent = bin.appendingPathComponent("claude")
        try "#!/bin/zsh\nprintf '%s\\n' \"$PWD\" \"$1\" > '\(result.path)'\n"
            .write(to: agent, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: agent.path)
        let launcher = SystemSessionLauncher(home: home, defaults: defaults) { script in
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
        XCTAssertEqual(SystemSessionLauncher(home: home, defaults: defaults).path(ofFolder: folder.lastPathComponent)?.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")),
                       folder.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")))
    }

    @MainActor
    func testNewWorktreeStartsInItsOwnFolderAndRefusesAnInUseBranch() async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("cowchat-worktree-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: home) }
        let bin = home.appendingPathComponent(".local/bin")
        let folder = home.appendingPathComponent("project")
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try git(["init", folder.path])
        try git(["-C", folder.path, "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                 "commit", "--allow-empty", "-m", "init"])
        let agent = bin.appendingPathComponent("codex")
        let result = home.appendingPathComponent("worktree-path.txt")
        try "#!/bin/zsh\nprintf '%s\\n' \"$PWD\" > '\(result.path)'\n"
            .write(to: agent, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: agent.path)
        let suite = "CowchatMac.WorktreeTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let launcher = SystemSessionLauncher(home: home, defaults: defaults) { script in
            let process = Process()
            process.executableURL = script
            try process.run()
            process.waitUntilExit()
            XCTAssertEqual(process.terminationStatus, 0)
        }
        let request = SessionRequest(folder: SessionFolder(path: folder), harness: .codex,
                                     branch: "jw/probe", firstMessage: "")
        try await launcher.start(request)
        let path = try String(contentsOf: result, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
        XCTAssertTrue(path.hasPrefix(home.path))
        XCTAssertNotEqual(path, folder.path)
        XCTAssertTrue(FileManager.default.fileExists(atPath: URL(fileURLWithPath: path).appendingPathComponent(".git").path))
        do {
            try await launcher.start(request)
            XCTFail("Branch already checked out in the first worktree")
        } catch {
            XCTAssertEqual(error as? SessionLaunchError, .branchInUse)
        }
    }
}
