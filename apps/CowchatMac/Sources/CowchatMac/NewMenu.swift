import SwiftUI

/// The ways to start something (navigation note, section 7.1), as the toolbar's New menu and the File menu list them:
/// "New session…" with ⇧⌘N while signed in, since a session registers under the account, "New room…" with ⌘N, a
/// divider, and "Join with invite code…", enabled while the API-key connection is connected. Join stays listed when it
/// is disabled, so the items never move.
struct NewMenuItems: View {
    @ObservedObject var workspace: WorkspaceStore
    @ObservedObject var social: SocialStore

    init(workspace: WorkspaceStore) {
        self.workspace = workspace
        social = workspace.social
    }

    var body: some View {
        if social.phase == .signedIn {
            // copy:
            Button("New session…") { workspace.presentNewSession() }
                .keyboardShortcut("n", modifiers: [.command, .shift])
        }
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
