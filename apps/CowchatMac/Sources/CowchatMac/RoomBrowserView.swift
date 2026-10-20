import SwiftUI

struct RoomBrowserView: View {
    @ObservedObject var workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var connecting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                Text("Find a room").gallopText(.h3)
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            Text("Open an existing room, including rooms you archived on this Mac.")
                .gallopText(.bodyM, color: SemanticColor.textSecondary)
            TextField("Search rooms, e.g. cowboy-junction", text: $query)
                .textFieldStyle(.roundedBorder)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if let store = workspace.accountGlobal {
                        RoomBrowserSection(workspace: workspace, store: store, server: .accountGlobal, query: query, onOpen: { dismiss() })
                    }
                    if let store = workspace.global {
                        RoomBrowserSection(workspace: workspace, store: store, server: .global, query: query, onOpen: { dismiss() })
                    } else {
                        Text("Community rooms").gallopText(.bodyMStrong)
                        Text("Connect to browse rooms such as cowboy-junction. Your friend account and Home stay connected.")
                            .gallopText(.bodyS, color: SemanticColor.textSecondary)
                        Button(connecting ? "Connecting…" : "Browse community rooms") { Task { await connect() } }
                            .buttonStyle(CowchatPillButtonStyle(prominent: true)).disabled(connecting)
                        if let error = workspace.globalSetupError { Text(error).foregroundStyle(.red) }
                    }
                    RoomBrowserSection(workspace: workspace, store: workspace.local, server: .local, query: query, onOpen: { dismiss() })
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(28).frame(width: 560, height: 560)
    }

    private func connect() async {
        connecting = true
        defer { connecting = false }
        let saved = workspace.configuredGlobalValues()
        _ = await workspace.connectToGlobal(url: saved.url, apiKey: saved.apiKey)
    }
}

enum RoomDiscovery {
    static func matches(_ room: Room, query: String) -> Bool {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if query.isEmpty { return true }
        return room.name.replacingOccurrences(of: "_", with: "-")
            .localizedCaseInsensitiveContains(query.replacingOccurrences(of: "_", with: "-"))
    }
}

private struct RoomBrowserSection: View {
    let workspace: WorkspaceStore
    @ObservedObject var store: ChatStore
    let server: WorkspaceStore.Server
    let query: String
    let onOpen: () -> Void
    @State private var error: String?
    @State private var invite = ""
    @State private var busy = false

    private var matchingRooms: [Room] {
        store.rooms.filter { RoomDiscovery.matches($0, query: query) }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(server == .local ? "Local" : server == .global ? "Community rooms" : "Your rooms")
                    .gallopText(.bodyMStrong)
                Spacer()
                Button("Refresh") { Task { await refresh() } }.disabled(busy || !store.connectionStatus.isConnected)
            }
            if !store.connectionStatus.isConnected {
                Text(store.connectionStatus.failureMessage ?? store.connectionStatus.label)
                    .gallopText(.bodyS, color: SemanticColor.textSecondary)
                Button("Reconnect") { store.reconnect() }
            } else if matchingRooms.isEmpty {
                Text(query.isEmpty ? "No rooms available." : "No matching rooms on this connection.")
                    .gallopText(.bodyS, color: SemanticColor.textSecondary)
            } else {
                ForEach(matchingRooms) { room in
                    Button { Task { await open(room) } } label: {
                        HStack {
                            Image(systemName: "number")
                            Text(room.name).lineLimit(1)
                            Spacer()
                            if store.isArchived(room) { Text("Archived").gallopText(.caption) }
                            Image(systemName: "arrow.up.right")
                        }.padding(10).contentShape(Rectangle())
                    }.buttonStyle(.plain).disabled(busy || !store.connectionStatus.isConnected)
                        .accessibilityLabel("Open \(room.name) from \(server.label)")
                }
            }
            if server == .global {
                Text("Missing a room? Invite-only rooms need a room invite or the API key you used before. Manage API keys in Connection settings.")
                    .gallopText(.bodyS, color: SemanticColor.textSecondary)
                HStack {
                    SecureField("Room invite token (cinv_…)", text: $invite).textFieldStyle(.roundedBorder)
                    Button("Add room") { Task { await redeem() } }
                        .disabled(busy || !store.connectionStatus.isConnected || invite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            if let error { Text(error).gallopText(.bodyS).foregroundStyle(.red) }
        }
        .task(id: store.connectionStatus.isConnected) { if store.connectionStatus.isConnected { await refresh() } }
    }

    private func refresh() async {
        do { try await store.refreshRooms(); error = nil }
        catch { self.error = error.localizedDescription }
    }

    private func open(_ room: Room) async {
        guard store.connectionStatus.isConnected, workspace.store(for: server) === store else { return }
        busy = true
        defer { busy = false }
        await workspace.select(room: room, on: server)
        guard workspace.store(for: server) === store else { return }
        if let failure = store.errorMessage { error = failure; return }
        guard store.connectionStatus.isConnected, store.selectedRoomID == room.id else { return }
        store.unarchive(room)
        workspace.searchText = ""
        onOpen()
    }

    private func redeem() async {
        busy = true
        do {
            guard let room = try await store.redeemInvite(token: invite) else {
                error = "Invite redeemed. Refresh the room list to see the room."
                busy = false
                return
            }
            invite = ""
            busy = false
            await open(room)
        } catch { self.error = error.localizedDescription; busy = false }
    }
}
