import SwiftUI
import CowchatKit

struct RoomListView: View {
    @EnvironmentObject private var store: ConnectionsStore
    @EnvironmentObject private var navigator: Navigator
    @State private var isAddingConnection = false
    @State private var removing: MergedRoom?
    @State private var showsArchived = false
    @State private var isCreatingRoom = false
    // The open room lives in the navigator, so the Inbox and People tabs can open one.
    private var path: [RoomRef] {
        get { navigator.roomPath }
        nonmutating set { navigator.roomPath = newValue }
    }
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var query = ""
    @State private var messageMatches: Set<RoomRef> = []
    @State private var joinable: [JoinableRoom] = []
    @State private var isSearching = false
    @State private var errorMessage: String?

    private var trimmedQuery: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var matchingRooms: [MergedRoom] {
        store.rooms.filter {
            $0.summary.name.localizedCaseInsensitiveContains(trimmedQuery) || messageMatches.contains($0.ref)
        }
    }
    private var matchingJoinable: [JoinableRoom] {
        joinable.filter { $0.name.localizedCaseInsensitiveContains(trimmedQuery) }
    }

    private var activeRooms: [MergedRoom] { store.rooms.filter { !$0.summary.isArchived } }
    private var archivedRooms: [MergedRoom] { store.rooms.filter(\.summary.isArchived) }

    private var failedConnections: [ConnectionRecord] {
        store.connections.filter {
            if case .failed = store.phases[$0.id] { return true }
            return false
        }
    }

    private var isReconnecting: Bool { store.phases.values.contains(.reconnecting) }

    // On an iPad the room list sits beside the open room, as in the Mac app. On an iPhone, and in a narrow iPad
    // window, opening a room pushes it over the list.
    var body: some View {
        if sizeClass == .regular {
            // Always side by side. Left to itself an iPad in portrait hides the list, and the app opens on a blank pane.
            NavigationSplitView(columnVisibility: .constant(.all)) {
                listColumn
                    .navigationSplitViewColumnWidth(min: 280, ideal: 320, max: 380)
                    .toolbar(removing: .sidebarToggle)
            } detail: {
                NavigationStack { openRoom }
            }
            .navigationSplitViewStyle(.balanced)
        } else {
            NavigationStack(path: $navigator.roomPath) {
                listColumn.navigationDestination(for: RoomRef.self) { ref in
                    if let session = store.session(for: ref.connectionID) {
                        RoomView(roomID: ref.roomID).environmentObject(session)
                    }
                }
            }
            // A room whose connection was removed closes.
            .onChange(of: store.connections.map(\.id)) { _, ids in
                navigator.roomPath.removeAll { !ids.contains($0.connectionID) }
            }
        }
    }

    @ViewBuilder private var openRoom: some View {
        // A room that was removed or left closes by itself, since there is no list to pop back to.
        if let ref = path.last, store.rooms.contains(where: { $0.ref == ref }),
           let session = store.session(for: ref.connectionID) {
            RoomView(roomID: ref.roomID).environmentObject(session).id(ref)
        } else {
            SemanticColor.surface500.ignoresSafeArea().accessibilityIdentifier("rooms.noneOpen")
        }
    }

    private var listColumn: some View {
            List {
                ForEach(failedConnections) { connection in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 8) {
                            GallopIconView(icon: .networkError, fallbackSystemName: "wifi.exclamationmark", size: 16)
                                .foregroundStyle(SemanticColor.iconError)
                            Text(connection.name).gallopText(.bodyLStrong, color: SemanticColor.textPrimary)
                        }
                        if case .failed(let message) = store.phases[connection.id] {
                            Text(message).gallopText(.bodyS, color: SemanticColor.textError)
                        }
                        Button("Try again") { Task { await store.retry(connectionID: connection.id) } }
                            .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
                            .accessibilityIdentifier("rooms.retry")
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .gallopCard(cornerRadius: 12)
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
                    .accessibilityIdentifier("rooms.failedConnection")
                }
                if trimmedQuery.isEmpty {
                    ForEach(activeRooms) { room in
                        row(room)
                    }
                    if !archivedRooms.isEmpty {
                        Button { showsArchived.toggle() } label: {
                            HStack {
                                Text("Archived").gallopText(.bodyS, color: SemanticColor.textTertiary)
                                Spacer()
                                Image(systemName: showsArchived ? "chevron.down" : "chevron.right")
                                    .font(.caption)
                                    .foregroundStyle(SemanticColor.iconTertiary)
                            }
                        }
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                        .accessibilityIdentifier("rooms.archived")
                        if showsArchived {
                            ForEach(archivedRooms) { room in
                                row(room)
                            }
                        }
                    }
                } else {
                    ForEach(matchingRooms) { room in
                        row(room)
                    }
                    if !matchingJoinable.isEmpty {
                        Text("To join")
                            .gallopText(.bodyS, color: SemanticColor.textTertiary)
                            .listRowBackground(Color.clear)
                            .listRowSeparator(.hidden)
                        ForEach(matchingJoinable) { room in
                            Button { join(room) } label: {
                                VStack(alignment: .leading, spacing: 0) {
                                    Text(room.name).gallopText(.bodyL, color: SemanticColor.textPrimary).lineLimit(1)
                                    if store.connections.count > 1 {
                                        Text(room.connectionName)
                                            .gallopText(.caption, color: SemanticColor.textTertiary)
                                            .lineLimit(1)
                                    }
                                }
                            }
                            .listRowBackground(Color.clear)
                            .listRowSeparator(.hidden)
                            .accessibilityIdentifier("rooms.join")
                        }
                    }
                    if matchingRooms.isEmpty && matchingJoinable.isEmpty {
                        Text(isSearching ? "Searching…" : "Nothing matches.")
                            .gallopText(.bodyL, color: SemanticColor.textTertiary)
                            .listRowBackground(Color.clear)
                            .listRowSeparator(.hidden)
                            .accessibilityIdentifier("rooms.searchState")
                    }
                }
            }
            .listStyle(.plain)
            .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search")
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .task(id: trimmedQuery) { await search(trimmedQuery) }
            .alert(
                errorMessage ?? "",
                isPresented: Binding(get: { errorMessage != nil }, set: { if !$0 { errorMessage = nil } })
            ) {
                Button("OK", role: .cancel) {}
            }
            .confirmationDialog(
                "Remove \(removing?.summary.name ?? "")?",
                isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                titleVisibility: .visible,
                presenting: removing
            ) { room in
                Button("Remove", role: .destructive) { Task { await store.remove(room: room.ref) } }
                    .accessibilityIdentifier("rooms.confirmRemove")
            }
            .scrollContentBackground(.hidden)
            .background(SemanticColor.surface400.ignoresSafeArea())
            .accessibilityIdentifier("rooms.list")
            .navigationTitle("Rooms")
            .refreshable { await store.refreshRooms() }
            .safeAreaInset(edge: .top) {
                if isReconnecting {
                    HStack(spacing: 6) {
                        GallopIconView(icon: .retry, fallbackSystemName: "arrow.triangle.2.circlepath", size: 14)
                        Text("Reconnecting").gallopText(.caption)
                    }
                    .foregroundStyle(SemanticColor.textSecondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
                    .background(SemanticColor.surface600)
                }
            }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        if !store.connectedConnections.isEmpty {
                            Button("New room") { isCreatingRoom = true }
                        }
                        Button("Add connection") { isAddingConnection = true }
                    } label: {
                        Image(systemName: "plus").foregroundStyle(SemanticColor.iconSecondary)
                    }
                    .accessibilityLabel("Add")
                    .accessibilityIdentifier("rooms.add")
                }
            }
            .sheet(isPresented: $isAddingConnection) { SignInView() }
            .sheet(isPresented: $isCreatingRoom) { NewRoomSheet { path = [$0] } }
    }
}

extension RoomListView {
    private func session(_ room: MergedRoom) -> ChatSession? { store.session(for: room.ref.connectionID) }

    private func search(_ text: String) async {
        messageMatches = []
        guard !text.isEmpty else {
            isSearching = false
            return
        }
        isSearching = true
        try? await Task.sleep(for: .milliseconds(250))
        guard !Task.isCancelled else { return }
        let rooms = await store.joinableRooms()
        let matches = await store.messageMatches(query: text)
        guard !Task.isCancelled else { return }
        joinable = rooms
        messageMatches = matches
        isSearching = false
    }

    private func join(_ room: JoinableRoom) {
        Task {
            do {
                try await store.join(room.ref)
                query = ""
                path = [room.ref]
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    @ViewBuilder private func rowLink(_ room: MergedRoom) -> some View {
        let content = RoomRow(
            room: room,
            server: store.connections.first { $0.id == room.ref.connectionID }?.server,
            showsConnection: store.connections.count > 1
        )
        if sizeClass == .regular {
            Button { path = [room.ref] } label: { content.contentShape(Rectangle()) }.buttonStyle(.plain)
        } else {
            NavigationLink(value: room.ref) { content }
        }
    }

    private func row(_ room: MergedRoom) -> some View {
        rowLink(room)
        .listRowBackground(sizeClass == .regular && path.last == room.ref ? SemanticColor.surface600 : Color.clear)
        .listRowSeparator(.hidden)
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            if store.canRemove(room.ref) {
                Button("Remove") { removing = room }
                    .tint(SemanticColor.buttonPrimaryDestructiveDefault)
                    .accessibilityIdentifier("rooms.remove")
            }
            if room.summary.isArchived {
                Button("Unarchive") { session(room)?.setArchived(false, roomID: room.ref.roomID) }
                    .accessibilityIdentifier("rooms.unarchive")
            } else if room.ref.roomID != "lobby" {
                Button("Archive") { session(room)?.setArchived(true, roomID: room.ref.roomID) }
                    .accessibilityIdentifier("rooms.archive")
            }
        }
    }
}

private struct RoomRow: View {
    let room: MergedRoom
    let server: ServerChoice?
    let showsConnection: Bool

    var body: some View {
        HStack(spacing: 12) {
            ServerMark(server: server)
            VStack(alignment: .leading, spacing: 0) {
                Text(room.summary.name)
                    .gallopText(room.summary.isUnread ? .bodyLStrong : .bodyL, color: SemanticColor.textPrimary)
                    .lineLimit(1)
                if showsConnection {
                    Text(room.connectionName)
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            if room.summary.isEncrypted && !room.summary.hasSecret {
                GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 16)
                    .foregroundStyle(SemanticColor.iconTertiary)
                    .accessibilityLabel("Locked")
            }
            if let state = room.summary.sessionState, state == "prompt" || state == "working" {
                Text(state == "prompt" ? "Waiting" : "Working")
                    .gallopText(.caption, color: state == "prompt" ? SemanticColor.textPrimary : SemanticColor.textTertiary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(state == "prompt" ? Palette.nugget500 : SemanticColor.surface600, in: Capsule())
            }
            if room.summary.isUnread {
                Circle()
                    .fill(Palette.nugget500)
                    .frame(width: 8, height: 8)
                    .accessibilityLabel("Unread")
            } else if let date = room.summary.lastActivity {
                Text(GallopTime.short(date)).gallopText(.caption, color: SemanticColor.textTertiary)
            }
        }
        .padding(.vertical, 4)
    }
}
