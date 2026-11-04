import Combine
import Foundation

/// Where first run stands. The workspace keeps it, so the wizard picks up where it was, and the wait on its last step
/// runs whether or not the wizard is drawn. The steps run in the app's order: sign in, the name (which makes Home), the
/// agents, and then ready, which waits on Home for the person's first session and opens Home when one appears. An
/// account that signs in with its Home already made skips the rest, and the wizard opens Home.
@MainActor
final class FirstRun: ObservableObject {
    @Published var step: WizardStep
    /// Whether the person skipped the agents step, which leaves both agents unconnected.
    @Published private(set) var skippedAgents = false
    /// Whether the last step waits for the first session, until one joins.
    @Published private(set) var waitsForFirstSession = false
    /// Called with the first session to join Home while the last step waits.
    var onFirstSession: ((LiveSession) -> Void)?
    private weak var sessions: HomeSessions?
    private var seen: Set<String> = []
    private var watching: AnyCancellable?
    /// Where the app's window reads whether first run is done, to show the wizard or the rooms.
    private let defaults: UserDefaults?

    init(step: WizardStep = .login, sessions: HomeSessions? = nil, defaults: UserDefaults? = nil) {
        self.step = step
        self.defaults = defaults
        if let sessions { watch(sessions) }
    }

    /// Follows Home's sessions, for the last step's wait.
    func watch(_ sessions: HomeSessions) {
        self.sessions = sessions
        // The publisher sends on the main actor as the list changes, with the list about to be shown.
        watching = sessions.$sessions.sink { [weak self] list in self?.sessionsChanged(list) }
    }

    // MARK: Steps

    /// Back to the sign-in step, as signing out or into another account does.
    func reset() {
        step = .login
        skippedAgents = false
        waitsForFirstSession = false
    }

    /// The account's setup has loaded: the name step when it has no Home yet, and otherwise the step it was on. An
    /// account that comes back on the sign-in or name step with its Home already made skips first run, which this
    /// says by returning true, and the wizard then opens Home.
    @discardableResult
    func recovered(hasSetup: Bool) -> Bool {
        guard hasSetup else {
            step = .name
            return false
        }
        return step == .login || step == .name
    }

    /// Create my Home made the account's Home and bot.
    func homeCreated() { step = .agents }

    /// Skip on the agents step: neither agent is connected, and the last step or Settings can connect them later.
    func skipAgents() {
        skippedAgents = true
        waitForFirstSession()
    }

    /// Continue on the agents step, once an agent is connected or waits for the person to trust its hooks.
    func agentsDone() {
        skippedAgents = false
        waitForFirstSession()
    }

    /// The last step, which waits for a session that joins Home from now on.
    private func waitForFirstSession() {
        seen = Set(sessions?.sessions.map(\.id) ?? [])
        waitsForFirstSession = true
        step = .ready
    }

    /// Records first run as done, which gives the wizard's place to the rooms, or as not done, which brings it back.
    func recordOnboarding(done: Bool) {
        defaults?.set(done ? CowchatOnboarding.currentVersion : 0, forKey: CowchatOnboarding.completedVersionKey)
    }

    /// Home's sessions as they change. The first new one while the last step waits ends first run on it.
    func sessionsChanged(_ list: [LiveSession]) {
        let new = list.filter { !seen.contains($0.id) }
        seen.formUnion(list.map(\.id))
        guard step == .ready, waitsForFirstSession, let first = new.first else { return }
        waitsForFirstSession = false
        onFirstSession?(first)
    }
}

extension WorkspaceStore {
    /// First run ends on Home as its first session joins: Home opens with the composer started on the session's
    /// @name, as pressing its row does, and the wizard gives way to the window.
    func finishFirstRun(opening session: LiveSession) async {
        await openSession(session)
        firstRun.recordOnboarding(done: true)
    }

    /// Sign out from the account menu: the account goes, and the window shows the signed-out page, which is first
    /// run's sign-in step.
    func signOut() async {
        await social.signOut()
        firstRun.reset()
        firstRun.recordOnboarding(done: false)
    }
}
