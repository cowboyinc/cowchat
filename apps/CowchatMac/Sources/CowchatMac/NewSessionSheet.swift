import AppKit
import SwiftUI

/// New session (add-new-company note, section 2, with the navigation note's sections 7.3 and 7.5), from the New menu,
/// the File menu and ⇧⌘N while signed in: the folder, the agent, a new worktree with its branch, and an optional first
/// message. Start hands them to the `SessionLauncher` and closes. When the session registers, the server makes its room,
/// the room's cell appears at the top of the list and the room opens.
struct NewSessionSheet: View {
    @ObservedObject var workspace: WorkspaceStore
    @ObservedObject var agents: AgentConnections
    @Environment(\.dismiss) private var dismiss
    @State private var folder: SessionFolder?
    /// Folders chosen with Choose… while the sheet is open, which the menu lists first.
    @State private var chosenFolders: [SessionFolder] = []
    @State private var harness: LiveSession.Harness?
    @State private var newWorktree = false
    @State private var branch = ""
    @State private var firstMessage = ""
    @State private var busy = false
    @State private var error: String?

    init(workspace: WorkspaceStore) {
        self.workspace = workspace
        agents = workspace.agentConnections
        _folder = State(initialValue: workspace.newestSessionFolder)
    }

    private var folders: [SessionFolder] {
        var seen: Set<String> = []
        return (chosenFolders + workspace.sessionFolders).filter { seen.insert($0.name).inserted }
    }

    private var typedBranch: String { branch.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var canStart: Bool {
        folder != nil && harness.map(isConnected) == true && (!newWorktree || !typedBranch.isEmpty) && !busy
    }

    var body: some View {
        // copy:
        SheetFrame(title: "New session", error: error) {
            // copy:
            SheetGroup(label: "Folder") { folderMenu }
            // copy:
            SheetGroup(label: "Agent") {
                VStack(spacing: 0) {
                    ForEach(Array(AgentConnections.harnesses.enumerated()), id: \.element) { index, option in
                        if index > 0 { PageRowSeam() }
                        // copy:
                        SheetChoiceRow(name: option.appName, fact: isConnected(option) ? nil : "Not connected",
                                       isChosen: harness == option, isEnabled: isConnected(option)) {
                            harness = option
                        } mark: {
                            HarnessMark(harness: option, size: 24)
                        }
                    }
                }
            }
            VStack(alignment: .leading, spacing: 8) {
                Toggle(isOn: $newWorktree) {
                    // copy:
                    Text("New worktree").gallopText(.bodyM, color: SemanticColor.textPrimary)
                }
                .toggleStyle(.checkbox)
                // copy:
                .macAccessibleAction(label: "New worktree", value: newWorktree ? "on" : "off") { newWorktree.toggle() }
                if newWorktree {
                    // copy:
                    CapsuleField(placeholder: "Branch", text: $branch, onSubmit: start)
                }
            }
            // copy:
            SheetGroup(label: "First message") {
                CapsuleField(placeholder: "Message", text: $firstMessage, onSubmit: start)
            }
        } buttons: {
            // copy:
            SheetButtons(act: "Start", isEnabled: canStart, busy: busy, cancel: { dismiss() }, perform: start)
        }
        .task {
            await agents.refresh()
            if harness == nil { harness = startingHarness }
        }
        .onChange(of: branch) { _ in error = nil }
        .onChange(of: folder) { _ in error = nil }
    }

    // copy:
    private var folderMenu: some View {
        SheetMenuField(title: folder?.name ?? "Choose a folder", isPlaceholder: folder == nil,
                       help: folder?.path?.path ?? folder?.name, accessibilityLabel: "Folder") {
            GallopIconView(icon: .folder, fallbackSystemName: "folder", size: 16)
                .foregroundStyle(SemanticColor.iconSecondary)
        } items: {
            ForEach(folders) { option in
                Toggle(option.name, isOn: Binding(get: { folder == option }, set: { if $0 { folder = option } }))
            }
            if !folders.isEmpty { Divider() }
            Button("Choose…", action: chooseFolder)
        }
    }

    private func isConnected(_ harness: LiveSession.Harness) -> Bool { agents.state(of: harness) == .connected }

    /// The agent used last on this Mac, or else the first that is connected.
    private var startingHarness: LiveSession.Harness? {
        if let last = workspace.lastSessionHarness, isConnected(last) { return last }
        return AgentConnections.harnesses.first(where: isConnected)
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let chosen = SessionFolder(path: url)
        chosenFolders.removeAll { $0.name == chosen.name }
        chosenFolders.insert(chosen, at: 0)
        folder = chosen
    }

    private func start() {
        guard canStart, let folder, let harness else { return }
        busy = true
        error = nil
        let request = SessionRequest(folder: folder, harness: harness, branch: newWorktree ? typedBranch : nil,
                                     firstMessage: firstMessage.trimmingCharacters(in: .whitespacesAndNewlines))
        Task {
            do {
                try await workspace.startSession(request)
                busy = false
                dismiss()
            } catch {
                busy = false
                self.error = error.localizedDescription
            }
        }
    }
}
