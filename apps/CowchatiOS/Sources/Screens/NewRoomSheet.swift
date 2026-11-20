import SwiftUI
import CowchatKit

struct NewRoomSheet: View {
    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    let onCreated: (RoomRef) -> Void

    @State private var name = ""
    @State private var connectionID = ""
    @State private var isCreating = false
    @State private var errorMessage: String?
    @State private var friendIDs: Set<String> = []

    private var choices: [ConnectionRecord] { store.connectedConnections }
    private var chosenID: String? { choices.contains { $0.id == connectionID } ? connectionID : choices.first?.id }
    private var trimmedName: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        NavigationStack {
            Form {
                TextField("Room name", text: $name)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .gallopText(.bodyL, color: SemanticColor.textPrimary)
                    .gallopFormRow()
                    .accessibilityIdentifier("newRoom.name")
                if choices.count > 1 {
                    Picker("Connection", selection: Binding(get: { chosenID ?? "" }, set: { connectionID = $0 })) {
                        ForEach(choices) { Text($0.name).tag($0.id) }
                    }
                    .gallopFormRow()
                    .accessibilityIdentifier("newRoom.connection")
                }
                if let chosenID, let account = store.account(for: chosenID) {
                    NewRoomFriends(account: account, picked: $friendIDs)
                }
                if let errorMessage {
                    Text(errorMessage)
                        .gallopText(.bodyS, color: SemanticColor.textError)
                        .gallopFormRow()
                        .accessibilityIdentifier("newRoom.error")
                }
            }
            .gallopForm()
            .navigationTitle("New room")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") { create() }
                        .disabled(trimmedName.isEmpty || chosenID == nil || isCreating)
                        .accessibilityIdentifier("newRoom.create")
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    private func create() {
        guard let chosenID else { return }
        isCreating = true
        Task {
            defer { isCreating = false }
            do {
                let friends = store.account(for: chosenID)?.friends.map(\.id).filter(friendIDs.contains) ?? []
                let ref = try await store.createRoom(connectionID: chosenID, name: name, friendIDs: friends)
                dismiss()
                onCreated(ref)
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }
}

/// The friends to invite with a new account room.
private struct NewRoomFriends: View {
    @ObservedObject var account: AccountStore
    @Binding var picked: Set<String>

    var body: some View {
        ForEach(account.friends) { friend in
            Button {
                if picked.contains(friend.id) { picked.remove(friend.id) } else { picked.insert(friend.id) }
            } label: {
                HStack {
                    Text(friend.account.displayName).gallopText(.bodyL, color: SemanticColor.textPrimary)
                    Spacer()
                    if picked.contains(friend.id) {
                        Image(systemName: "checkmark").foregroundStyle(Palette.nugget500).accessibilityLabel("Invited")
                    }
                }
            }
            .gallopFormRow()
            .accessibilityIdentifier("newRoom.friend")
        }
    }
}
