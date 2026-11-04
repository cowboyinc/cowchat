import Foundation

/// Connects the installed coding agents to the Cowchat hooks. Process arguments
/// are passed directly; neither a room credential nor user text enters a shell.
@MainActor
final class SystemAgentHookInstaller: AgentHookInstaller {
    private let home: URL
    private let systemSearchDirectories: [URL]

    init(home: URL = FileManager.default.homeDirectoryForCurrentUser,
         systemSearchDirectories: [URL] = [URL(fileURLWithPath: "/opt/homebrew/bin"),
                                           URL(fileURLWithPath: "/usr/local/bin"),
                                           URL(fileURLWithPath: "/usr/bin")]) {
        self.home = home
        self.systemSearchDirectories = systemSearchDirectories
    }

    func state(of harness: LiveSession.Harness) async -> AgentHookState {
        guard executable(for: harness) != nil else { return .notInstalled }
        switch harness {
        case .claudeCode:
            let manifest = home.appendingPathComponent(".claude/plugins/installed_plugins.json")
            guard let data = try? Data(contentsOf: manifest),
                  let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let plugins = root["plugins"] as? [String: Any],
                  plugins["cowchat@cowchat"] != nil else { return .notConnected }
            let settings = home.appendingPathComponent(".claude/settings.json")
            if let data = try? Data(contentsOf: settings),
               let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let enabled = root["enabledPlugins"] as? [String: Bool],
               enabled["cowchat@cowchat"] == false { return .notConnected }
            return .connected
        case .codex:
            let hooks = home.appendingPathComponent(".codex/hooks.json")
            guard let data = try? Data(contentsOf: hooks),
                  let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let groups = root["hooks"] as? [String: [[String: Any]]],
                  let start = groups["SessionStart"],
                  start.contains(where: { group in
                      (group["hooks"] as? [[String: Any]])?.contains(where: {
                          ($0["command"] as? String)?.contains(".cowchat/codex-hook/session.py") == true
                      }) == true
                  }) else { return .notConnected }
            let mappings = home.appendingPathComponent(".cowchat/codex-sessions")
            let files = (try? FileManager.default.contentsOfDirectory(at: mappings, includingPropertiesForKeys: nil)) ?? []
            return files.contains { $0.pathExtension == "json" && !$0.lastPathComponent.hasSuffix("cursor.json") }
                ? .connected : .waitingForTrust
        }
    }

    func connect(_ harness: LiveSession.Harness) async throws -> AgentHookState {
        guard let binary = executable(for: harness) else {
            throw CowchatConnectionError.transport("Install \(harness.appName) before connecting it.")
        }
        let arguments: [String]
        switch harness {
        case .claudeCode:
            arguments = ["plugin", "install", "cowchat", "--marketplace", "cowboyinc/cowchat", "--json"]
        case .codex:
            guard let bridge = executable(named: "cowchat-codex") else {
                throw CowchatConnectionError.transport("Install the Cowchat Codex bridge before connecting Codex.")
            }
            try await run(bridge, ["install", "--start-daemon"])
            return await state(of: harness)
        }
        try await run(binary, arguments)
        return await state(of: harness)
    }

    private func executable(for harness: LiveSession.Harness) -> URL? {
        executable(named: harness == .claudeCode ? "claude" : "codex")
    }

    private func executable(named name: String) -> URL? {
        if name == "cowchat-codex" {
            let bundled = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/cowchat-codex")
            if FileManager.default.isExecutableFile(atPath: bundled.path) { return bundled }
        }
        let paths = [".local/bin", ".cargo/bin", "bin"].map { home.appendingPathComponent($0) }
            + systemSearchDirectories
        return paths.map { $0.appendingPathComponent(name) }
            .first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }

    private func run(_ binary: URL, _ arguments: [String]) async throws {
        let status = try await Task.detached {
            let process = Process()
            process.executableURL = binary
            process.arguments = arguments
            process.standardOutput = FileHandle.nullDevice
            process.standardError = FileHandle.nullDevice
            try process.run()
            process.waitUntilExit()
            return process.terminationStatus
        }.value
        guard status == 0 else {
            throw CowchatConnectionError.transport("Could not connect the agent. Check its installation and try again.")
        }
    }
}
