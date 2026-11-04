import Foundation
import XCTest
@testable import CowchatMac

final class SystemAgentHookInstallerTests: XCTestCase {
    @MainActor
    func testCodexConnectStartsItsSelectedBinaryAndReportsFailure() async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("cowchat-connect-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: home) }
        let bin = home.appendingPathComponent(".local/bin")
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        let log = home.appendingPathComponent("calls.txt")
        let bridge = bin.appendingPathComponent("cowchat-codex")
        try "#!/bin/sh\nprintf 'bridge %s\\n' \"$*\" >> '\(log.path)'\n"
            .write(to: bridge, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: bridge.path)
        let codex = bin.appendingPathComponent("codex")
        try "#!/bin/sh\nprintf 'codex %s\\n' \"$*\" >> '\(log.path)'\nexit 7\n"
            .write(to: codex, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: codex.path)
        let installer = SystemAgentHookInstaller(home: home, systemSearchDirectories: [])
        do {
            _ = try await installer.connect(.codex)
            XCTFail("A failed daemon start must reach the user")
        } catch {
            XCTAssertTrue(error.localizedDescription.contains("Could not connect"))
        }
        let calls = try String(contentsOf: log, encoding: .utf8)
        XCTAssertEqual(calls, "bridge install\ncodex app-server daemon start\n")
    }

    @MainActor
    func testInstalledHooksAndFirstCodexSessionDriveConnectionState() async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("cowchat-hooks-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: home) }
        let bin = home.appendingPathComponent(".local/bin")
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        let installer = SystemAgentHookInstaller(home: home, systemSearchDirectories: [])
        var claudeState = await installer.state(of: .claudeCode)
        var codexState = await installer.state(of: .codex)
        XCTAssertEqual(claudeState, .notInstalled)
        XCTAssertEqual(codexState, .notInstalled)
        for name in ["claude", "codex"] {
            let binary = bin.appendingPathComponent(name)
            try "#!/bin/sh\nexit 0\n".write(to: binary, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: binary.path)
        }
        claudeState = await installer.state(of: .claudeCode)
        codexState = await installer.state(of: .codex)
        XCTAssertEqual(claudeState, .notConnected)
        XCTAssertEqual(codexState, .notConnected)

        let plugins = home.appendingPathComponent(".claude/plugins")
        try FileManager.default.createDirectory(at: plugins, withIntermediateDirectories: true)
        try "{\"plugins\":{\"cowchat@cowchat\":[{}]}}"
            .write(to: plugins.appendingPathComponent("installed_plugins.json"), atomically: true, encoding: .utf8)
        claudeState = await installer.state(of: .claudeCode)
        XCTAssertEqual(claudeState, .connected)
        let settings = home.appendingPathComponent(".claude/settings.json")
        try "{\"enabledPlugins\":{\"cowchat@cowchat\":false}}".write(to: settings, atomically: true, encoding: .utf8)
        claudeState = await installer.state(of: .claudeCode)
        XCTAssertEqual(claudeState, .notConnected)

        let codex = home.appendingPathComponent(".codex")
        try FileManager.default.createDirectory(at: codex, withIntermediateDirectories: true)
        try "{\"hooks\":{\"SessionStart\":[{\"hooks\":[{\"command\":\"python3 ~/.cowchat/codex-hook/session.py\"}]}]}}"
            .write(to: codex.appendingPathComponent("hooks.json"), atomically: true, encoding: .utf8)
        codexState = await installer.state(of: .codex)
        XCTAssertEqual(codexState, .waitingForTrust)
        let mappings = home.appendingPathComponent(".cowchat/codex-sessions")
        try FileManager.default.createDirectory(at: mappings, withIntermediateDirectories: true)
        try "{}".write(to: mappings.appendingPathComponent("session.json"), atomically: true, encoding: .utf8)
        codexState = await installer.state(of: .codex)
        XCTAssertEqual(codexState, .connected)
    }
}
