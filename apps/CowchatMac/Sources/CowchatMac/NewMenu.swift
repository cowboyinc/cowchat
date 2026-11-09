import SwiftUI

/// The ways to start something (navigation note, section 7.1), as the toolbar's New menu and the File menu list them:
/// "New room…" with ⌘N, a divider, and "Join with invite code…", enabled while the API-key connection is connected.
/// Join stays listed when it is disabled, so the items never move.
struct NewMenuItems: View {
    @ObservedObject var workspace: WorkspaceStore

    var body: some View {
        // copy:
        Button("New room…") { workspace.presentNewRoom() }
            .keyboardShortcut("n", modifiers: .command)
        Divider()
        if let global = workspace.global {
            JoinWithCodeItem(workspace: workspace, global: global)
        } else {
            // copy:
            Button("Join with invite code…") {}
                .disabled(true)
        }
    }
}

/// Join with invite code's item, which follows the API-key connection's status.
private struct JoinWithCodeItem: View {
    let workspace: WorkspaceStore
    @ObservedObject var global: ChatStore

    var body: some View {
        // copy:
        Button("Join with invite code…") { workspace.presentJoinWithCode() }
            .disabled(!global.connectionStatus.isConnected)
    }
}
