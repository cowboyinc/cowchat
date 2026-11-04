import SwiftUI

/// Where Cowchat's hook into one of the person's coding agents stands. An agent this Mac doesn't have is installed
/// first. Codex asks the person once, the next time it starts, whether to trust the hooks Cowchat added, so it waits
/// on that answer before it is connected.
enum AgentHookState: Equatable {
    case notInstalled, notConnected, connecting, waitingForTrust, connected
}

extension LiveSession.Harness {
    /// Where a person gets the agent when this Mac doesn't have it. Chad confirms these with task 13.
    var installPage: URL {
        switch self {
        case .claudeCode: return URL(string: "https://claude.com/product/claude-code")!
        case .codex: return URL(string: "https://developers.openai.com/codex")!
        }
    }
}

/// Installs Cowchat's hooks into the person's coding agents and says where each stands: the Claude Code plugin, and
/// Codex's session hooks, which the person trusts once in Codex. First run and Settings go through it and nothing
/// else. The real app fills it from task 13 of `docs/plans/cowchat-server-for-chad.md` ("Installing the Claude Code
/// plugin and walking through trusting the Codex hook on first run"), with the plugin and hooks of tasks 6 and 7.
@MainActor
protocol AgentHookInstaller: AnyObject {
    /// Where the agent's hook stands now.
    func state(of harness: LiveSession.Harness) async -> AgentHookState
    /// Installs the agent's hook and returns where it stands afterwards: Claude Code connected, Codex waiting for the
    /// person to trust its hooks.
    func connect(_ harness: LiveSession.Harness) async throws -> AgentHookState
}

/// Stands in until task 13 lands: nothing is connected, and Connect says so.
@MainActor
final class UnavailableAgentHookInstaller: AgentHookInstaller {
    func state(of harness: LiveSession.Harness) async -> AgentHookState { .notConnected }

    func connect(_ harness: LiveSession.Harness) async throws -> AgentHookState {
        // copy:
        throw CowchatConnectionError.transport("This version of Cowchat can't connect \(harness.appName) yet.")
    }
}

/// The person's agents as first run and Settings show them: each one's state, kept from the installer, and Connect.
@MainActor
final class AgentConnections: ObservableObject {
    static let harnesses: [LiveSession.Harness] = [.claudeCode, .codex]

    @Published private(set) var states: [LiveSession.Harness: AgentHookState] = [:]
    @Published private(set) var errorMessage: String?
    /// Set when the app is made; the prototype puts its own in.
    var installer: any AgentHookInstaller

    init(installer: (any AgentHookInstaller)? = nil) {
        self.installer = installer ?? SystemAgentHookInstaller()
    }

    func state(of harness: LiveSession.Harness) -> AgentHookState { states[harness] ?? .notConnected }

    /// Whether an agent is connected, or waits only for the person to trust its hooks, so first run can go on.
    var hasConnected: Bool { Self.harnesses.contains { [.waitingForTrust, .connected].contains(state(of: $0)) } }
    var isWaitingForTrust: Bool { Self.harnesses.contains { state(of: $0) == .waitingForTrust } }

    /// Reads every agent's state from the installer again.
    func refresh() async {
        for harness in Self.harnesses where state(of: harness) != .connecting {
            let state = await installer.state(of: harness)
            if self.state(of: harness) != .connecting { states[harness] = state }
        }
    }

    /// Connects one agent, as its row's Connect does.
    func connect(_ harness: LiveSession.Harness) async {
        guard state(of: harness) == .notConnected else { return }
        states[harness] = .connecting
        errorMessage = nil
        do {
            states[harness] = try await installer.connect(harness)
        } catch {
            states[harness] = .notConnected
            errorMessage = error.localizedDescription
        }
    }
}

/// Claude Code and Codex, each on a 60 row with its 24 mark and name, and at the row's end either where it stands or
/// the button that moves it on: Install, Connect, a spinner while it connects, what is left to do in Codex, or a tick
/// and Connected. A hairline sits between rows. First run puts the list in its card and Settings in its own. While
/// Codex waits for the person to trust its hooks, the list asks the installer again every few seconds.
struct AgentHookList: View {
    @ObservedObject var connections: AgentConnections

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(AgentConnections.harnesses.enumerated()), id: \.element) { index, harness in
                if index > 0 { AgentHookSeparator() }
                AgentHookRow(harness: harness, state: connections.state(of: harness)) {
                    Task { await connections.connect(harness) }
                }
            }
        }
        .frame(maxWidth: .infinity)
        .task(id: connections.isWaitingForTrust) {
            while connections.isWaitingForTrust, !Task.isCancelled {
                try? await Task.sleep(for: .seconds(3))
                await connections.refresh()
            }
        }
    }
}

/// The hairline between the agents' rows, in Surface/Glass/Border/Shadow.
struct AgentHookSeparator: View {
    var body: some View {
        Rectangle().fill(SemanticColor.surfaceGlassBorderShadow).frame(height: 1)
    }
}

/// Connect's error, under Settings' agents. First run puts it above the button under its card.
struct AgentHookError: View {
    @ObservedObject var connections: AgentConnections

    var body: some View {
        if let error = connections.errorMessage {
            Text(error)
                .gallopText(.bodyS, color: SemanticColor.textError)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A done mark at 16 in Notification/Success/Icon/Primary. Gallop's set here has no tick, so the system's is drawn.
struct SuccessTick: View {
    var body: some View {
        Image(systemName: "checkmark")
            .font(.system(size: 12, weight: .bold))
            .frame(width: 16, height: 16)
            .foregroundStyle(SemanticColor.notificationSuccessIconPrimary)
            .accessibilityHidden(true)
    }
}

/// One agent's row: 12 above and below a 36 line holding the mark, 12, the name in Body M, and the row's end, which
/// holds words or a button, not both. The one exception is Codex waiting for trust, which offers to open Codex.
private struct AgentHookRow: View {
    let harness: LiveSession.Harness
    let state: AgentHookState
    let connect: () -> Void
    @State private var confirmCodexService = false

    var body: some View {
        HStack(spacing: 12) {
            HarnessMark(harness: harness, size: 24)
            Text(harness.appName).gallopText(.bodyM, color: SemanticColor.textPrimary).lineLimit(1)
            Spacer(minLength: 12)
            end
        }
        .frame(height: 36)
        .padding(.vertical, 12)
        .confirmationDialog("Start Codex service?", isPresented: $confirmCodexService) {
            Button("Install hooks and start service") { connect() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Cowchat will install Codex hooks and start a local service that uses about 400 MB of memory. Codex will ask you to trust the hooks when it next starts.")
        }
    }

    @ViewBuilder private var end: some View {
        switch state {
        case .notInstalled:
            // copy:
            Button("Install") { NSWorkspace.shared.open(harness.installPage) }
                .buttonStyle(CowchatPillButtonStyle())
                .fixedSize()
                .macAccessibleAction(label: "Install \(harness.appName)") { NSWorkspace.shared.open(harness.installPage) }
        case .notConnected:
            // copy:
            Button("Connect") {
                if harness == .codex { confirmCodexService = true } else { connect() }
            }
                .buttonStyle(CowchatPillButtonStyle(prominent: true))
                .fixedSize()
                .macAccessibleAction(label: "Connect \(harness.appName)") {
                    if harness == .codex { confirmCodexService = true } else { connect() }
                }
                .help(harness == .codex ? "Installs Cowchat hooks and starts a local Codex service using about 400 MB of memory." : "Installs the Cowchat Claude Code plugin.")
        case .connecting:
            HStack(spacing: 6) {
                ProgressView().controlSize(.small).frame(width: 16, height: 16)
                // copy:
                Text("Connecting").gallopText(.bodyS, color: SemanticColor.textTertiary)
            }
        case .waitingForTrust:
            HStack(spacing: 8) {
                // copy: the person trusts the hooks in the agent's app the next time it starts.
                Text(app == nil ? "Start \(harness.appName) to trust" : "Trust in \(harness.appName)")
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
                    .lineLimit(1)
                if let app {
                    // copy:
                    Button("Open \(harness.appName)") { AgentAppResolver.open(app) }
                        .buttonStyle(CowchatPillButtonStyle())
                        .fixedSize()
                }
            }
        case .connected:
            HStack(spacing: 6) {
                SuccessTick()
                // copy:
                Text("Connected").gallopText(.bodyS, color: SemanticColor.textSecondary)
            }
        }
    }

    /// The agent's app, when this Mac has it.
    private var app: AgentAppResolver.ResolvedApp? {
        guard let app = AgentAppResolver.resolvedApp(forAgentNamed: harness.rawValue),
              AgentAppResolver.applicationURL(for: app) != nil else { return nil }
        return app
    }
}
