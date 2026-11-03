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

    var errorDescription: String? {
        switch self {
        // copy:
        case .branchInUse: return "That branch is in use."
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
