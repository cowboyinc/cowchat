import SwiftUI

/// Add person (add-new-company note, section 1, with the navigation note's section 7.5): friends are invited to one of
/// the person's live sessions' rooms. The title is the room's name. Under "Add people" the friend search lists the
/// person's friends who are neither in the room nor invited to it, and Add invites the ones picked. It makes no room
/// and posts nothing, since the session already listens in its room.
struct AddPersonSheet: View {
    @ObservedObject var workspace: WorkspaceStore
    @ObservedObject var social: SocialStore
    let session: LiveSession
    @Environment(\.dismiss) private var dismiss
    @State private var picked: [String] = []
    /// The room's people, once the server has listed them.
    @State private var members: Set<String> = []
    @State private var busy = false
    @State private var error: String?

    init(workspace: WorkspaceStore, session: LiveSession) {
        self.workspace = workspace
        social = workspace.social
        self.session = session
    }

    private var roomName: String {
        workspace.accountGlobal?.rooms.first { $0.id == session.roomID }?.name ?? workspace.liveSessions.name(of: session)
    }

    /// The friends who can be added: not in the room and not invited to it, in the server's order.
    private var candidates: [SocialAccount] {
        let invited = Set(social.sentInvitations.filter { $0.roomId == session.roomID && $0.status == "pending" }
            .map(\.to.id))
        return social.friends.map(\.account).filter { !members.contains($0.id) && !invited.contains($0.id) }
    }

    private var canAdd: Bool { !picked.isEmpty && !busy && social.phase == .signedIn }

    var body: some View {
        SheetFrame(title: roomName, error: error) {
            // copy:
            SheetGroup(label: "Add people") {
                PeoplePicker(people: candidates, picked: $picked, placeholder: "Search friends",
                             emptyLine: social.friends.isEmpty ? "No friends yet" : "No friends to add",
                             submitEmpty: add)
            }
        } buttons: {
            // copy:
            SheetButtons(act: "Add", isEnabled: canAdd, busy: busy, cancel: { dismiss() }, perform: add)
        }
        .task { await loadMembers() }
        .onChange(of: social.phase) { if $0 != .signedIn { dismiss() } }
    }

    private func loadMembers() async {
        let people = await workspace.people(of: session.roomID)
        members = Set(people.map(\.id))
    }

    private func add() {
        guard canAdd else { return }
        busy = true
        error = nil
        let chosen = picked
        Task {
            let failed = await workspace.addPeople(chosen, to: session)
            busy = false
            guard let first = failed.first else {
                dismiss()
                return
            }
            picked = failed
            let name = social.friends.first { $0.id == first }?.account.displayName ?? first
            // copy:
            error = "Could not add \(name). Try again."
        }
    }
}
