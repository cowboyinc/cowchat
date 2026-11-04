import Foundation

/// A folder New session can start a session in: its name, as a session's hook sends it, and its path on this Mac when
/// the app knows it.
struct SessionFolder: Hashable, Identifiable {
    let name: String
    let path: URL?

    var id: String { path?.path ?? name }

    init(name: String, path: URL? = nil) {
        self.name = name
        self.path = path
    }

    init(path: URL) {
        self.init(name: path.lastPathComponent, path: path)
    }
}

/// What New session's Start asks for (add-new-company note, section 2): the folder, the agent, the branch of a new
/// worktree or none, and the first message, which may be empty.
struct SessionRequest: Equatable {
    let folder: SessionFolder
    let harness: LiveSession.Harness
    let branch: String?
    let firstMessage: String
}

/// Why a session could not start.
enum SessionLaunchError: LocalizedError, Equatable {
    /// A live session in the folder already has the branch checked out, so no worktree can take it.
    case branchInUse
    case folderUnavailable

    var errorDescription: String? {
        switch self {
        // copy:
        case .branchInUse: return "That branch is in use."
        case .folderUnavailable: return "Choose a folder on this Mac before starting a session."
        }
    }
}

/// Starts a coding agent for New session: it makes the worktree when asked and opens Terminal in the folder running
/// `claude` or `codex` with the first message, and the session registers through its hook, where the server makes its
/// room. The real app fills it from task 13 of `docs/plans/cowchat-server-for-chad.md` ("Opening Terminal in a folder
/// or a new worktree running `claude` or `codex` (spec 16)").
@MainActor
protocol SessionLauncher: AnyObject {
    /// Where a folder a session ran in is on this Mac, or nil when the app doesn't know. The hook sends only the
    /// folder's name today (a question for Chad beside his task 6).
    func path(ofFolder name: String) -> URL?
    /// Starts the session. It returns once the agent is started, before the session registers.
    func start(_ request: SessionRequest) async throws
}

/// Stands in until task 13 lands: no folder has a known path, and Start says it can't start a session.
@MainActor
final class UnavailableSessionLauncher: SessionLauncher {
    func path(ofFolder name: String) -> URL? { nil }

    func start(_ request: SessionRequest) async throws {
        // copy:
        throw CowchatConnectionError.transport("This version of Cowchat can't start a session yet.")
    }
}

/// Opens an installed coding agent in Terminal. The generated command file is
/// private, short lived, and quotes every path and first message as one shell
/// argument; none of the user's text is interpreted as shell code.
@MainActor
final class SystemSessionLauncher: SessionLauncher {
    private static let foldersKey = "CowchatMac.sessionFolderPaths"
    private let home: URL
    private let defaults: UserDefaults
    private let openSession: @MainActor (URL) async throws -> Void
    private var knownFolders: [String: URL]

    init(home: URL = FileManager.default.homeDirectoryForCurrentUser,
         defaults: UserDefaults = .standard,
         openSession: @escaping @MainActor (URL) async throws -> Void = { script in
             try await SystemSessionLauncher.run(URL(fileURLWithPath: "/usr/bin/open"), ["-a", "Terminal", script.path])
         }) {
        self.home = home
        self.defaults = defaults
        self.openSession = openSession
        let paths = defaults.dictionary(forKey: Self.foldersKey) as? [String: String] ?? [:]
        self.knownFolders = paths.mapValues { URL(fileURLWithPath: $0) }
    }

    func path(ofFolder name: String) -> URL? { knownFolders[name] }

    func start(_ request: SessionRequest) async throws {
        guard let folder = request.folder.path, folder.isFileURL,
              (try? folder.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true else {
            throw SessionLaunchError.folderUnavailable
        }
        let executableName = request.harness == .claudeCode ? "claude" : "codex"
        guard let binary = executable(named: executableName) else {
            throw CowchatConnectionError.transport("Install \(request.harness.appName) before starting a session.")
        }
        let workingFolder: URL
        if let branch = request.branch {
            workingFolder = try await makeWorktree(from: folder, branch: branch)
        } else {
            workingFolder = folder
        }
        let launchDirectory = home.appendingPathComponent(".cowchat/session-launches", isDirectory: true)
        try FileManager.default.createDirectory(at: launchDirectory, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
        let script = launchDirectory.appendingPathComponent("\(UUID().uuidString).command")
        let message = request.firstMessage.isEmpty ? "" : " -- \(Self.shellQuote(request.firstMessage))"
        let content = "#!/bin/zsh\n/bin/rm -- \(Self.shellQuote(script.path))\ncd -- \(Self.shellQuote(workingFolder.path)) || exit 1\nexec \(Self.shellQuote(binary.path))\(message)\n"
        try content.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        do {
            try await openSession(script)
        } catch {
            try? FileManager.default.removeItem(at: script)
            throw error
        }
        knownFolders[request.folder.name] = folder
        defaults.set(knownFolders.mapValues(\.path), forKey: Self.foldersKey)
    }

    private func makeWorktree(from folder: URL, branch: String) async throws -> URL {
        let git = URL(fileURLWithPath: "/usr/bin/git")
        guard (try? await Self.run(git, ["-C", folder.path, "check-ref-format", "--branch", branch])) != nil else {
            throw CowchatConnectionError.transport("Enter a valid Git branch name.")
        }
        let slug = branch.replacingOccurrences(of: "/", with: "-")
        let destination = folder.deletingLastPathComponent()
            .appendingPathComponent("\(folder.lastPathComponent)-\(slug)-\(UUID().uuidString.prefix(8))")
        do {
            try await Self.run(git, ["-C", folder.path, "worktree", "add", "-b", branch, destination.path])
        } catch {
            throw SessionLaunchError.branchInUse
        }
        return destination
    }

    private func executable(named name: String) -> URL? {
        let paths = [".local/bin", ".cargo/bin", "bin"].map { home.appendingPathComponent($0) }
            + [URL(fileURLWithPath: "/opt/homebrew/bin"), URL(fileURLWithPath: "/usr/local/bin")]
        return paths.map { $0.appendingPathComponent(name) }
            .first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }

    private static func shellQuote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    private static func run(_ binary: URL, _ arguments: [String]) async throws {
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
            throw CowchatConnectionError.transport("Could not start the session. Check the folder and agent installation.")
        }
    }
}
