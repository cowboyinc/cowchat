import SwiftUI
import CowchatKit

struct RoomPeopleSheet: View {
    @EnvironmentObject private var session: ChatSession
    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    let roomID: String

    private var people: [RoomMember] {
        (session.members[roomID] ?? []).sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    var body: some View {
        NavigationStack {
            List {
                if let account = store.account(for: session.connectionID), account.isAccountRoom(roomID) {
                    AccountRoomPeople(account: account, roomID: roomID)
                    Text("Connected now").gallopText(.bodyS, color: SemanticColor.textTertiary)
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                }
                if people.isEmpty {
                    Text("No one is connected.")
                        .gallopText(.bodyL, color: SemanticColor.textTertiary)
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                }
                ForEach(people) { person in
                    HStack(spacing: 12) {
                        AgentAvatar(name: person.name, size: 32)
                        Text(person.name)
                            .gallopText(.bodyL, color: SemanticColor.textPrimary)
                            .lineLimit(1)
                    }
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(SemanticColor.surface400.ignoresSafeArea())
            .accessibilityIdentifier("people.list")
            .navigationTitle("People")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }.accessibilityIdentifier("people.done")
                }
            }
            .refreshable { await session.refreshPeople(roomID: roomID) }
            .task { await session.refreshPeople(roomID: roomID) }
        }
        .presentationDetents([.medium, .large])
    }
}

/// A new invite token, held only long enough to hand it to the share sheet.
struct InviteToShare: Identifiable {
    let id = UUID()
    let token: String
}

struct ShareSheet: UIViewControllerRepresentable {
    let text: String

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [text], applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
