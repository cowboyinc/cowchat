import SwiftUI

/// Join with invite code (navigation note, section 7.2), from the New menu and the File menu, in place of Settings ›
/// Connection's invite field and Find a room's token field. One secure field takes the code, since the code grants a
/// room, and Join redeems it for the API key and opens the room it grants. It uses the starting sheets' frame and
/// buttons, with nothing under the title.
struct JoinWithCodeSheet: View {
    @ObservedObject var workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var code = ""
    @State private var busy = false
    @State private var failed = false

    private var canJoin: Bool {
        !code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && workspace.canJoinWithCode && !busy
    }

    var body: some View {
        // copy:
        SheetFrame(title: "Join with invite code", error: failed ? "That code didn't work." : nil) {
            // copy:
            CapsuleField(placeholder: "Invite code", text: $code, isSecure: true, onSubmit: join)
        } buttons: {
            // copy:
            SheetButtons(act: "Join", isEnabled: canJoin, busy: busy, cancel: { dismiss() }, perform: join)
        }
        .onChange(of: code) { _ in failed = false }
    }

    private func join() {
        guard canJoin else { return }
        busy = true
        failed = false
        Task {
            do {
                try await workspace.joinWithCode(code)
                busy = false
                dismiss()
            } catch {
                busy = false
                failed = true
            }
        }
    }
}
