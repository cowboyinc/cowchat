import AppKit
import SwiftUI

private enum AppAppearance: String, CaseIterable, Identifiable {
    case system
    case light
    case dark

    var id: String { rawValue }

    var label: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }

    var colorScheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }
}

struct ContentView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @EnvironmentObject private var store: ChatStore
    let onShowOnboarding: () -> Void
    @AppStorage("CowchatMac.appearance") private var appearance = AppAppearance.system.rawValue
    @State private var isSidebarVisible = true
    @State private var isSettingsPresented = false
    @Environment(\.controlActiveState) private var controlActiveState

    init(onShowOnboarding: @escaping () -> Void = {}) {
        self.onShowOnboarding = onShowOnboarding
    }

    var body: some View {
        HStack(spacing: 0) {
            if isSidebarVisible {
                SidebarView(
                    isSidebarVisible: $isSidebarVisible,
                    isSettingsPresented: $isSettingsPresented
                )
                .frame(width: 280)
                .transition(.move(edge: .leading).combined(with: .opacity))
            }

            Group {
                if let room = store.selectedRoom {
                    if room.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame {
                        LobbyDashboardView(room: room, isSidebarVisible: $isSidebarVisible)
                            .id("lobby-\(room.id)")
                    } else {
                        ChatRoomView(room: room, isSidebarVisible: $isSidebarVisible)
                            .id(room.id)
                    }
                } else {
                    EmptyChatView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            // Dash "Page" card: surface500 content panel on the surface400
            // shell, radius 16, hairline border, 8pt gutter (Figma 4605:27623).
            .background(SemanticColor.surface500)
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                    .allowsHitTesting(false)
            }
            .padding([.top, .trailing, .bottom], 8)
            .padding(.leading, isSidebarVisible ? 0 : 8)
        }
        .background(SemanticColor.surface400)
        // The unified toolbar otherwise paints its own system strip; tint it
        // with the same surface400 shell so the nav reads as one tan surface.
        .toolbarBackground(SemanticColor.surface400, for: .windowToolbar)
        .navigationTitle(store.selectedRoom?.name ?? "Cowchat")
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Button {
                    withAnimation(.easeInOut(duration: 0.2)) { isSidebarVisible.toggle() }
                } label: {
                    GallopIconView(icon: .sidebar, fallbackSystemName: "sidebar.left", size: 17)
                        .foregroundStyle(SemanticColor.iconTertiary)
                        .frame(width: 36, height: 36)
                        .background(Circle().fill(SemanticColor.surface700))
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .help(isSidebarVisible ? "Hide sidebar" : "Show sidebar")
                .macAccessibleAction(label: "Toggle sidebar") {
                    withAnimation(.easeInOut(duration: 0.2)) { isSidebarVisible.toggle() }
                }
            }
            ToolbarItem(placement: .navigation) {
                Button {
                    store.presentCreateRoom()
                } label: {
                    GallopIconView(icon: .edit, fallbackSystemName: "square.and.pencil", size: 17)
                        .foregroundStyle(SemanticColor.iconSecondary)
                        .frame(width: 36, height: 36)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .help("New room (⌘N)")
                .macAccessibleAction(label: "Create room") { store.presentCreateRoom() }
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .cowchatToggleSidebar)) { _ in
            guard controlActiveState == .key || controlActiveState == .active else { return }
            withAnimation(.easeInOut(duration: 0.2)) { isSidebarVisible.toggle() }
        }
        .frame(minWidth: 900, minHeight: 600)
        .animation(.easeInOut(duration: 0.2), value: isSidebarVisible)
        .preferredColorScheme(AppAppearance(rawValue: appearance)?.colorScheme)
        .sheet(
            isPresented: $store.isCreateRoomPresented,
            onDismiss: { store.createRoomParentID = nil }
        ) {
            CreateRoomView(
                local: workspace.local,
                globalOrLocal: workspace.global ?? workspace.local,
                hasGlobal: workspace.global != nil
            )
                .environmentObject(store)
        }
        .sheet(item: $store.roomBeingRenamed) { room in
            RenameRoomView(room: room)
                .environmentObject(store)
        }
        .sheet(isPresented: $isSettingsPresented) {
            SettingsView(
                isPresented: $isSettingsPresented,
                onShowOnboarding: onShowOnboarding
            )
                .environmentObject(store)
        }
        .alert("Cowchat", isPresented: Binding(
            get: { store.errorMessage != nil },
            set: { if !$0 { store.errorMessage = nil } }
        )) {
            Button("OK", role: .cancel) { store.errorMessage = nil }
        } message: {
            Text(store.errorMessage ?? "")
        }
        .overlay(alignment: .bottomTrailing) {
            if let room = store.roomReadyNotice {
                RoomReadyNotice(room: room)
                    .environmentObject(store)
                    .padding(18)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            } else if let room = store.secondAgentHintRoom {
                SecondAgentHintNotice(room: room)
                    .id(room.id)
                    .environmentObject(store)
                    .padding(18)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.easeInOut(duration: 0.2), value: store.roomReadyNotice?.id)
        .animation(.easeInOut(duration: 0.2), value: store.secondAgentHintRoom?.id)
    }
}

private struct SidebarView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @Binding var isSidebarVisible: Bool
    @Binding var isSettingsPresented: Bool
    @State private var isArchiveExpanded = false
    @FocusState private var isSearchFocused: Bool
    /// Once per process: the launch-focus clear must not repeat on sidebar re-mounts.
    private static var didClearLaunchFocus = false

    // Subviews observe their ChatStore directly (@ObservedObject) so a change
    // in one store re-renders only the views that read it. The workspace
    // deliberately does NOT republish store changes — forwarding them turned
    // every message tick into a whole-window invalidation, which starved the
    // run loop during AppKit menu/hover tracking (2026-08-13 archive hang,
    // same loop signature as the 2026-08-06 freeze).
    var body: some View {
        VStack(spacing: 0) {
            LobbyNavRow(local: workspace.local)

            searchField
                .padding(.horizontal, 12)
                .padding(.bottom, 14)

            SidebarRoomLists(
                local: workspace.local,
                globalOrLocal: workspace.global ?? workspace.local,
                hasGlobal: workspace.global != nil
            )

            // Archive stays pinned above the footer instead of trailing the
            // room list (which floats it mid-sidebar when the list is short).
            ArchiveSection(
                local: workspace.local,
                globalOrLocal: workspace.global ?? workspace.local,
                hasGlobal: workspace.global != nil,
                isExpanded: $isArchiveExpanded
            )
            .padding(.horizontal, 8)

            SidebarFooter(
                local: workspace.local,
                globalOrLocal: workspace.global ?? workspace.local,
                hasGlobal: workspace.global != nil,
                isSettingsPresented: $isSettingsPresented
            )
        }
        .padding(.top, 14)
        .onAppear {
            // The pinned search field is the window's first focusable view, so
            // AppKit hands it first-responder at launch and stray keystrokes
            // land in the filter. Clear it ONCE per process — running on every
            // sidebar re-mount would blur whatever the user is typing in
            // (e.g. the composer) each time the sidebar reopens.
            guard !Self.didClearLaunchFocus else { return }
            Self.didClearLaunchFocus = true
            DispatchQueue.main.async {
                if isSearchFocused == false {
                    NSApp.keyWindow?.makeFirstResponder(nil)
                }
            }
        }
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            GallopIconView(icon: .search, fallbackSystemName: "magnifyingglass", size: 13)
                .foregroundStyle(SemanticColor.iconTertiary)
            TextField("Search rooms or messages", text: $workspace.searchText)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .focused($isSearchFocused)
                .onExitCommand {
                    // Escape clears the filter and returns focus to the list.
                    workspace.searchText = ""
                    isSearchFocused = false
                }
                .accessibilityLabel("Search rooms or messages")
            if !workspace.searchText.isEmpty {
                Button { workspace.searchText = "" } label: {
                    GallopIconView(icon: .dismiss, fallbackSystemName: "xmark.circle.fill", size: 13)
                        .foregroundStyle(SemanticColor.iconSubtle)
                }
                .buttonStyle(.plain)
                .macAccessibleAction(label: "Clear room search") { workspace.searchText = "" }
            }
        }
        .padding(.horizontal, 11)
        .frame(height: 36)
        .background(SemanticColor.textfieldDefault, in: Capsule())
        .overlay {
            Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
    }

}

/// Dash-style nav destination: Lobby is Home, above the conversations
/// table, not a row inside it (Patrick, 2026-08-06).
private struct LobbyNavRow: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var local: ChatStore
    @State private var isHovering = false

    private var lobby: Room? {
        local.rooms.first { $0.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame }
    }

    var body: some View {
        if let lobby {
            row(lobby)
                .padding(.horizontal, 12)
                .padding(.bottom, 10)
        }
    }

    private func row(_ lobby: Room) -> some View {
        let isSelected = workspace.isSelected(lobby, on: .local)
        return Button {
            Task { await workspace.select(room: lobby, on: .local) }
        } label: {
            HStack(spacing: 10) {
                GallopIconView(icon: .sunrise, fallbackSystemName: "sunrise", size: 18)
                    .foregroundStyle(
                        isSelected
                            ? SemanticColor.surfaceGlassOnIconDefault
                            : SemanticColor.iconSecondary
                    )
                Text("Lobby")
                    .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
                Spacer()
            }
            .padding(.horizontal, 10)
            .frame(height: 38)
            .background(
                SidebarRowBackground(
                    state: .init(isSelected: isSelected, isHovering: isHovering)
                )
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
        .macAccessibleAction(
            label: "Open Lobby",
            value: isSelected ? "selected" : nil
        ) {
            Task { await workspace.select(room: lobby, on: .local) }
        }
    }
}

/// One selectable room row, shared by the room lists and the archive.
private struct SidebarRoomRowButton: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var store: ChatStore
    let room: Room
    let server: WorkspaceStore.Server
    let now: Date
    var isArchived = false

    var body: some View {
        Button {
            Task { await workspace.select(room: room, on: server) }
        } label: {
            RoomRow(
                room: room,
                messagePreview: store.roomMessagePreviews[room.id],
                isSelected: workspace.isSelected(room, on: server),
                isUnread: store.isUnread(room),
                isWorking: store.isWorking(room, at: now),
                now: now
            )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            if isArchived {
                Button("Unarchive") { store.unarchive(room) }
            } else {
                Button("Rename") { store.presentRename(room) }
                    .disabled(!store.canRename(room))
                if room.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame {
                    Button("Archive") {
                        Task { await store.archive(room) }
                    }
                }
            }
        }
        .macAccessibleAction(label: "Open \(room.name)", value: accessibilityValue) {
            Task { await workspace.select(room: room, on: server) }
        }
    }

    /// Value announced by the AccessibleActionOverlay for a room row (see
    /// macAccessibleAction) — RoomRow's own accessibility subtree is hidden,
    /// so unread/selected state must be composed here to be announced at all.
    private var accessibilityValue: String? {
        let parts = [
            store.isWorking(room, at: now) ? "Agents working" : nil,
            store.isUnread(room) ? "Unread" : nil,
            workspace.isSelected(room, on: server) ? "selected" : nil,
        ].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }
}

/// The scrolling room table: a Local section and, with global rooms on, a
/// Global section. `globalOrLocal` duplicates `local` when global is off —
/// @ObservedObject cannot hold an Optional, and observing local twice is free.
private struct SidebarRoomLists: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var local: ChatStore
    @ObservedObject var globalOrLocal: ChatStore
    let hasGlobal: Bool

    var body: some View {
        TimelineView(.periodic(from: .now, by: anyAgentsThinking ? 10 : 60)) { timeline in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if visibleRoomsEmpty {
                        if isSearchActive {
                            emptyRoomsState
                        }
                    } else {
                        section(for: .local, store: local, at: timeline.date)
                        if hasGlobal {
                            section(for: .global, store: globalOrLocal, at: timeline.date)
                        }
                    }
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 12)
            }
            .scrollIndicators(.hidden)
        }
    }

    private func baseRooms(in store: ChatStore, server: WorkspaceStore.Server) -> [Room] {
        // The local Lobby lives in its own nav row, so the idle table excludes
        // it — but an active search must still surface Lobby name/message
        // hits. The global server has no nav row; its lobby stays listed.
        let searching = !workspace.searchText
            .trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let source = searching || server == .global
            ? store.unarchivedRooms
            : store.unarchivedRooms.filter {
                $0.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame
            }
        return RoomSidebarPresentation.filteredRooms(
            from: source,
            query: workspace.searchText,
            matchingMessageRoomIDs: store.messageSearchRoomIDs
        )
    }

    private var anyAgentsThinking: Bool {
        !local.lastThinkingAt.isEmpty
            || (hasGlobal && !globalOrLocal.lastThinkingAt.isEmpty)
    }

    private var visibleRoomsEmpty: Bool {
        baseRooms(in: local, server: .local).isEmpty
            && (!hasGlobal || baseRooms(in: globalOrLocal, server: .global).isEmpty)
    }

    /// Item D (quiet empty state): the sidebar only shows an explicit empty
    /// message while a search is actually in flight or has text typed. A
    /// genuinely empty room list (no search) renders nothing above Archive.
    private var isSearchActive: Bool {
        isSearchingMessages || !workspace.searchText.isEmpty
    }

    private var isSearchingMessages: Bool {
        local.isSearchingMessages || (hasGlobal && globalOrLocal.isSearchingMessages)
    }

    @ViewBuilder
    private func section(
        for server: WorkspaceStore.Server,
        store: ChatStore,
        at now: Date
    ) -> some View {
        let rooms = baseRooms(in: store, server: server)
        if hasGlobal {
            // Always shown while two servers exist, even over an empty
            // list — a connecting/failed global server announces itself
            // here, not only in the footer.
            sectionHeader(for: server, store: store)
        }
        if !rooms.isEmpty {
            VStack(spacing: 0) {
                ForEach(RoomSidebarPresentation.sortedByRecency(rooms)) { room in
                    SidebarRoomRowButton(store: store, room: room, server: server, now: now)
                }
            }
            .padding(.bottom, 10)
        }
    }

    private func sectionHeader(for server: WorkspaceStore.Server, store: ChatStore) -> some View {
        HStack(spacing: 6) {
            Text(server.label)
                .gallopText(.caption, color: SemanticColor.textTertiary)
            if !store.connectionStatus.isConnected {
                Text(store.connectionStatus.label)
                    .gallopText(.dataLabel, color: SemanticColor.textTertiary)
                    .help(
                        store.connectionStatus.failureMessage
                            ?? store.connectionStatus.label
                    )
            }
            Spacer()
        }
        .padding(.leading, 16)
        .padding(.trailing, 10)
        .padding(.top, 6)
        .padding(.bottom, 4)
    }

    /// Only ever shown while `isSearchActive`, so the branch below always has
    /// search text (or an in-flight search) to react to — never the bare
    /// "no rooms at all" case, which item D renders as nothing instead.
    private var emptyRoomsState: some View {
        VStack(spacing: 8) {
            if isSearchingMessages {
                ProgressView()
                    .controlSize(.small)
            } else {
                GallopIconView(icon: .search, fallbackSystemName: "magnifyingglass", size: 20)
                    .foregroundStyle(SemanticColor.iconTertiary)
            }
            Text(isSearchingMessages ? "Searching messages…" : "No rooms or messages found")
                .gallopText(.bodyMStrong, color: SemanticColor.textSecondary)
            if !workspace.searchText.isEmpty {
                Text("Try another room, message, or agent name.")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    .multilineTextAlignment(.center)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 18)
        .padding(.top, 44)
    }
}

private struct ArchiveSection: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var local: ChatStore
    @ObservedObject var globalOrLocal: ChatStore
    let hasGlobal: Bool
    @Binding var isExpanded: Bool

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { timeline in
            content(at: timeline.date)
        }
    }

    private var isVisible: Bool {
        isExpanded || !workspace.searchText.isEmpty
    }

    private func archivedRooms(in store: ChatStore) -> [Room] {
        RoomSidebarPresentation.filteredRooms(
            from: store.archivedRooms,
            query: workspace.searchText,
            matchingMessageRoomIDs: store.messageSearchRoomIDs
        )
    }

    private var entries: [(server: WorkspaceStore.Server, store: ChatStore, room: Room)] {
        var all = archivedRooms(in: local).map { (server: WorkspaceStore.Server.local, store: local, room: $0) }
        if hasGlobal {
            all += archivedRooms(in: globalOrLocal).map { (server: .global, store: globalOrLocal, room: $0) }
        }
        return all
    }

    private func content(at now: Date) -> some View {
        let entries = entries
        return VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(.easeInOut(duration: 0.18)) { isExpanded.toggle() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "archivebox")
                        .font(.system(size: 13, weight: .medium))
                        .offset(y: 1)  // optical center against the label ink
                    Text("Archive")
                        .gallopText(.bodySStrong)
                    Spacer()
                    if !entries.isEmpty {
                        Text("\(entries.count)")
                            .gallopText(.caption)
                    }
                    Image(systemName: isVisible ? "chevron.down" : "chevron.right")
                        .font(.system(size: 10, weight: .semibold))
                }
                .foregroundStyle(SemanticColor.textTertiary)
                // Leading 16 matches RoomRow's avatar inset so the archive
                // glyph joins the same left column as the rows above it.
                .padding(.leading, 16)
                .padding(.trailing, 10)
                .frame(height: 36)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .macAccessibleAction(
                label: "Archive, \(entries.count) rooms",
                value: isVisible ? "expanded" : "collapsed"
            ) {
                withAnimation(.easeInOut(duration: 0.18)) { isExpanded.toggle() }
            }

            if isVisible {
                if entries.isEmpty {
                    Text("No rooms archived")
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                        .padding(.leading, 16)
                        .padding(.trailing, 10)
                        .padding(.bottom, 8)
                } else {
                    // Bounded: the archive sits OUTSIDE the room-list scroll
                    // view, so an unbounded expansion would crush the room
                    // list and push the footer offscreen at min window height.
                    // Rows are a fixed 54pt; cap the reveal at ~4.5 rows.
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(entries, id: \.room.id) { entry in
                                SidebarRoomRowButton(
                                    store: entry.store,
                                    room: entry.room,
                                    server: entry.server,
                                    now: now,
                                    isArchived: true
                                )
                            }
                        }
                    }
                    .scrollIndicators(.hidden)
                    .frame(height: min(CGFloat(entries.count) * 54, 244))
                }
            }
        }
    }
}

private struct SidebarFooter: View {
    @ObservedObject var local: ChatStore
    @ObservedObject var globalOrLocal: ChatStore
    let hasGlobal: Bool
    @Binding var isSettingsPresented: Bool

    var body: some View {
        HStack(spacing: 8) {
            Button {
                isSettingsPresented = true
            } label: {
                // A Grid keeps the status column aligned — "Local" and
                // "Global" set different label widths.
                Grid(alignment: .leading, horizontalSpacing: 6, verticalSpacing: 3) {
                    statusRow(label: "Local", store: local)
                    if hasGlobal {
                        statusRow(label: "Global", store: globalOrLocal)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Connection settings")
            .macAccessibleAction(label: "Connection settings") { isSettingsPresented = true }
            Spacer()
            if anyServerDisconnected {
                CircleIconButton(
                    icon: .retry,
                    fallbackSystemName: "arrow.clockwise",
                    help: "Reconnect",
                    action: reconnectDisconnectedServers
                )
            }
            CircleIconButton(
                icon: .settings,
                fallbackSystemName: "gearshape",
                help: "Settings",
                action: { isSettingsPresented = true }
            )
        }
        .padding(.horizontal, 12)
        .frame(height: 58)
    }

    private func statusRow(label: String, store: ChatStore) -> some View {
        GridRow {
            Circle()
                .fill(statusColor(for: store.connectionStatus))
                .frame(width: 7, height: 7)
            Text(label)
                .gallopText(.caption, color: SemanticColor.textSecondary)
            Text(store.connectionStatus.label)
                .gallopText(.dataLabel, color: SemanticColor.textTertiary)
                .help(store.connectionStatus.failureMessage ?? store.connectionStatus.label)
        }
    }

    private var anyServerDisconnected: Bool {
        !local.connectionStatus.isConnected
            || (hasGlobal && !globalOrLocal.connectionStatus.isConnected)
    }

    private func reconnectDisconnectedServers() {
        var stores = [local]
        if hasGlobal { stores.append(globalOrLocal) }
        for store in stores where !store.connectionStatus.isConnected {
            store.reconnect()
        }
    }

    private func statusColor(for status: ConnectionStatus) -> Color {
        switch status {
        case .connected: return SemanticColor.success
        case .connecting: return SemanticColor.warning
        case .disconnected, .failed: return SemanticColor.textError
        }
    }
}

enum SidebarRowState {
    case normal, selected, hover
    init(isSelected: Bool, isHovering: Bool) {
        self = isSelected ? .selected : (isHovering ? .hover : .normal)
    }
}

/// Cowboy SidebarRowPill recipe at cowchat's row geometry (radius 12 — the
/// 100pt pill reads as a blob on two-line 54pt rows; divergence logged in spec).
struct SidebarRowBackground: View {
    let state: SidebarRowState
    private var shape: RoundedRectangle { RoundedRectangle(cornerRadius: 12, style: .continuous) }

    var body: some View {
        switch state {
        case .normal:
            shape.fill(Color.clear)
        case .selected:
            // On the surface400 sidebar shell the lighter surface600 is what
            // reads as "lifted"; surface400 would vanish into the background.
            shape.fill(SemanticColor.surface600)
        case .hover:
            shape.fill(SemanticColor.surface500)
                .overlay(shape.strokeBorder(Color.black.opacity(0.08), lineWidth: 0.5))
                .shadow(color: Color.black.opacity(0.04), radius: 1.5, x: 0, y: 1)
        }
    }
}

private struct RoomRow: View {
    let room: Room
    let messagePreview: String?
    let isSelected: Bool
    let isUnread: Bool
    let isWorking: Bool
    let now: Date
    @State private var isHovering = false

    var body: some View {
        HStack(spacing: 10) {
            RoomAvatar(name: room.name, size: 40, accented: isSelected)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 5) {
                    Text(room.name)
                        .gallopText(isUnread ? .bodySStrong : .bodyS, color: SemanticColor.textPrimary)
                        .lineLimit(1)
                    if room.encrypted {
                        GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 12)
                            .foregroundStyle(SemanticColor.textPrimary)
                    }
                    Spacer(minLength: 6)
                    if isWorking {
                        GallopIconView(icon: .thinking, fallbackSystemName: "arrow.triangle.2.circlepath", size: 12)
                            .foregroundStyle(SemanticColor.buttonPrimaryDefault)
                    }
                    Text(
                        (room.lastActivity ?? room.createdAt)
                            .cowchatRelativeTime(relativeTo: now)
                    )
                        .gallopText(.dataLabel, color: SemanticColor.textTertiary)
                }

                Text(roomSummary)
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    .lineLimit(1)
            }
        }
        .padding(.trailing, 8)
        .padding(.leading, 16)
        .frame(height: 54)
        // Floats in the leading padding rather than occupying a flow column,
        // so read rows carry no reserved gutter and content never shifts
        // when the dot appears.
        .overlay(alignment: .leading) {
            if isUnread {
                Circle()
                    .fill(Palette.nugget500)
                    .frame(width: 7, height: 7)
                    .padding(.leading, 4)
            }
        }
        .background(SidebarRowBackground(state: .init(isSelected: isSelected, isHovering: isHovering)))
        .onHover { isHovering = $0 }
    }

    private var roomSummary: String {
        if let messagePreview, !messagePreview.isEmpty { return messagePreview }
        if let description = room.description, !description.isEmpty { return description }
        return "Open conversation"
    }
}

private struct LobbyDashboardView: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room
    @Binding var isSidebarVisible: Bool

    private var dashboardRooms: [Room] {
        store.unarchivedRooms.filter { $0.id != room.id }
    }

    private var availableAgentCount: Int {
        LobbyPresentation.availableAgentCount(
            from: store.roomMembers,
            excluding: store.agentID
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 1) {
                    Text("Lobby")
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    Text("\(availableAgentCount) available agents")
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                }

                Spacer()
            }
            .padding(.top, 10)
            .padding(.leading, 18)
            .padding(.trailing, 14)
            .frame(height: 58)

            ScrollView {
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 210, maximum: 280), spacing: 14)],
                    alignment: .leading,
                    spacing: 14
                ) {
                    ForEach(dashboardRooms) { dashboardRoom in
                        DashboardRoomCard(room: dashboardRoom)
                    }

                    Button {
                        store.presentCreateRoom()
                    } label: {
                        VStack(alignment: .leading, spacing: 18) {
                            Image(systemName: "plus")
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(SemanticColor.buttonSecondaryIconDefault)
                                .frame(width: 34, height: 34)
                                .background(SemanticColor.buttonSecondaryDefault, in: Circle())
                            Spacer(minLength: 12)
                            Text("New Room")
                                .gallopText(.h4, color: SemanticColor.textPrimary)
                        }
                        .frame(maxWidth: .infinity, minHeight: 132, alignment: .topLeading)
                        .gallopCard()
                    }
                    .buttonStyle(.plain)
                    .macAccessibleAction(label: "Create room") {
                        store.presentCreateRoom()
                    }
                }
                .padding(20)
            }
            .scrollIndicators(.hidden)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .background(SemanticColor.surface500)
    }
}

private struct DashboardRoomCard: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room

    private var parentRoom: Room? {
        guard let parentID = room.parentID else { return nil }
        return store.rooms.first { $0.id == parentID }
    }

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Button {
                Task { await store.select(room: room) }
            } label: {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        RoomAvatar(name: room.name, size: 38, accented: false)
                        if room.encrypted {
                            GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 12)
                                .foregroundStyle(SemanticColor.iconTertiary)
                        }
                        Spacer()
                    }

                    Spacer(minLength: 8)
                    if let parentRoom {
                        Text("in \(parentRoom.name)")
                            .gallopText(.dataLabel, color: SemanticColor.textTertiary)
                            .lineLimit(1)
                    }
                    Text(room.name)
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                        .lineLimit(1)
                    Text(
                        store.roomMessagePreviews[room.id]
                            ?? room.description
                            ?? "Open conversation"
                    )
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                        .lineLimit(2)
                }
                .frame(maxWidth: .infinity, minHeight: 132, alignment: .topLeading)
                .gallopCard()
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Open \(room.name)") {
                Task { await store.select(room: room) }
            }

            Menu {
                Button("Rename") { store.presentRename(room) }
                    .disabled(!store.canRename(room))
                Button("Archive") {
                    Task { await store.archive(room) }
                }
            } label: {
                GallopIconView(icon: .ellipsis, fallbackSystemName: "ellipsis", size: 14)
                    .foregroundStyle(SemanticColor.iconTertiary)
                    .frame(width: 28, height: 28)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("Actions for \(room.name)")
            .padding(12)
        }
    }
}

private struct RoomReadyNotice: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room

    var body: some View {
        HStack(spacing: 14) {
            VStack(alignment: .leading, spacing: 2) {
                Text("\(room.name) is ready")
                    .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                Text("You can now begin chatting with your collaborator.")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
            }
            Button {
                Task { await store.openRoomReadyNotice() }
            } label: {
                // Chrome inside the label: the whole capsule is the hit area.
                Text("Open Room")
                    .gallopText(.bodySStrong, color: SemanticColor.buttonPrimaryTextDefault)
                    .padding(.horizontal, 14)
                    .frame(height: 34)
                    .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                    .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Open \(room.name)") {
                Task { await store.openRoomReadyNotice() }
            }
            Button {
                store.roomReadyNotice = nil
            } label: {
                GallopIconView(icon: .dismiss, fallbackSystemName: "xmark", size: 11)
                    .foregroundStyle(SemanticColor.iconTertiary)
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Dismiss room notice") {
                store.roomReadyNotice = nil
            }
        }
        .padding(14)
        .background {
            // Cowboy AppStatusBar glass recipe, kept at this notice's own
            // rounded-rectangle shape (it isn't a capsule) per the controller
            // amendment to Task 14 Step 2.
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(.ultraThinMaterial)
                .overlay {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(SemanticColor.surfaceGlass500)
                }
                .overlay {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .stroke(SemanticColor.surfaceGlassBorderHighlight, lineWidth: 1)
                }
        }
        .shadow(color: .black.opacity(0.08), radius: 2, y: 1)
        .shadow(color: .black.opacity(0.04), radius: 0, y: 0.5)
    }
}

private struct SecondAgentHintNotice: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room

    var body: some View {
        HStack(spacing: 14) {
            Text("Add more agents anytime — Copy connect prompt in the ⋯ menu.")
                .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
            Button {
                store.secondAgentHintRoom = nil
            } label: {
                GallopIconView(icon: .dismiss, fallbackSystemName: "xmark", size: 11)
                    .foregroundStyle(SemanticColor.iconTertiary)
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Dismiss hint") {
                store.secondAgentHintRoom = nil
            }
        }
        .padding(14)
        .background {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(.ultraThinMaterial)
                .overlay {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(SemanticColor.surfaceGlass500)
                }
                .overlay {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .stroke(SemanticColor.surfaceGlassBorderHighlight, lineWidth: 1)
                }
        }
        .shadow(color: .black.opacity(0.08), radius: 2, y: 1)
        .shadow(color: .black.opacity(0.04), radius: 0, y: 0.5)
        .task {
            try? await Task.sleep(nanoseconds: 8_000_000_000)
            if !Task.isCancelled { store.secondAgentHintRoom = nil }
        }
    }
}

private struct ChatRoomView: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room
    @Binding var isSidebarVisible: Bool
    @State private var isComposerExpanded = false
    @State private var isFieldHovering = false
    @State private var isDestroyConfirmationPresented = false
    @State private var isDestroyingRoom = false
    @State private var isMessageListNearBottom = true
    @State private var newMessageCount = 0
    @State private var hasCopiedQuietRoomPrompt = false
    /// The list is revealed only after the initial bottom-anchor scroll, so a
    /// room switch never paints top-anchored content and animates it away.
    @State private var hasPositionedInitialScroll = false

    private var parentRoom: Room? {
        guard let parentID = room.parentID else { return nil }
        return store.rooms.first { $0.id == parentID }
    }

    private func paneState(at now: Date) -> RoomPaneState {
        RoomPaneState.state(
            connectionStatus: store.connectionStatus,
            isLoadingMessages: store.isLoadingMessages,
            hasMessages: !store.messages.isEmpty,
            hasOtherMembers: ChatPresencePresentation.hasCollaboratorSignal(
                members: store.roomMembers,
                currentAgentID: store.agentID,
                fallbackMemberCount: room.memberCount,
                fallbackMemberCountIncludesCurrentAgent:
                    store.fallbackMemberCountIncludesCurrentAgent(in: room.id),
                recentActivityByAgent: store.recentAgentActivityAt[room.id],
                now: now
            )
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            chatHeader

            ZStack(alignment: .bottomTrailing) {
                messageList
                if store.isLoadingMessages && !hasPositionedInitialScroll {
                    ProgressView()
                        .controlSize(.small)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                TimelineView(.periodic(from: .now, by: 10)) { timeline in
                    let state = paneState(at: timeline.date)
                    ZStack {
                        if case .connectPrompt(let variant) = state {
                            RoomConnectStateView(
                                roomName: room.name,
                                prompt: store.connectPrompt(for: room),
                                variant: variant,
                                makeCopyPrompt: { await store.copyableConnectPrompt(for: room) }
                            )
                            .onAppear { store.ensurePromptInvite(for: room) }
                        } else if state == .quiet {
                            quietRoom
                                .allowsHitTesting(true)
                        }
                    }
                }
                composer
            }
        }
        .background(SemanticColor.surface500)
        .alert("Destroy \(room.name)?", isPresented: $isDestroyConfirmationPresented) {
            Button("Cancel", role: .cancel) {}
            Button("Destroy Room", role: .destructive) {
                isDestroyingRoom = true
                Task {
                    _ = await store.destroy(room)
                    isDestroyingRoom = false
                }
            }
        } message: {
            Text("This irreversibly removes the room, its messages, tasks, votes, and subscriptions from Cowchat's active server state. This cannot be undone in Cowchat; storage snapshots or backups may retain copies.")
        }
        .toolbar {
            // With the unified toolbar's title hidden there is no flexible
            // space in the strip, so primaryAction items pack against the
            // leading cluster; the Spacer pins the menu to the window's
            // trailing edge.
            ToolbarItemGroup(placement: .primaryAction) {
                Spacer()
                Menu {
                    Button("Copy connect prompt") { copyConnectPrompt() }
                    Divider()
                    // Rename and destroy are creator-only, so for other
                    // agents' rooms they are hidden rather than left as
                    // permanently disabled clutter.
                    if store.canRename(room) {
                        Button("Rename room") { store.presentRename(room) }
                    }
                    Button("Archive room") { Task { await store.archive(room) } }
                    Button("Create nested room…") { store.presentCreateRoom(parentID: room.id) }
                    if !store.connectionStatus.isConnected {
                        Button("Reconnect") { store.start() }
                    }
                    if store.canDestroy(room) {
                        Divider()
                        Button("Destroy room…", role: .destructive) {
                            isDestroyConfirmationPresented = true
                        }
                        .disabled(isDestroyingRoom)
                    }
                } label: {
                    GallopIconView(icon: .ellipsis, fallbackSystemName: "ellipsis", size: 17)
                        .foregroundStyle(SemanticColor.iconSecondary)
                }
                .menuIndicator(.hidden)
                .accessibilityLabel("Room actions")
            }
        }
    }

    private var chatHeader: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 1) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    if let parentRoom {
                        Button(parentRoom.name) {
                            Task { await store.select(room: parentRoom) }
                        }
                        .buttonStyle(.plain)
                        .gallopText(.bodySStrong, color: SemanticColor.textTertiary)
                        .lineLimit(1)
                        .macAccessibleAction(label: "Open parent room \(parentRoom.name)") {
                            Task { await store.select(room: parentRoom) }
                        }
                        GallopIconView(icon: .chevronRightExtraSmall, fallbackSystemName: "chevron.right", size: 10)
                            .foregroundStyle(SemanticColor.iconSubtle)
                    }
                    Text(room.name)
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    if room.encrypted {
                        GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 12)
                            .foregroundStyle(SemanticColor.iconTertiary)
                    }
                }
                TimelineView(.periodic(from: .now, by: 10)) { timeline in
                    let summary = presenceSummary(at: timeline.date)
                    Text(summary)
                        .gallopText(
                            .caption,
                            color: summary.contains("active")
                                ? SemanticColor.warning : SemanticColor.textTertiary
                        )
                        .lineLimit(1)
                }
            }

            Spacer()
        }
        .padding(.top, 10)
        .padding(.leading, 18)
        .padding(.trailing, 14)
        .frame(height: 58)
    }

    private func copyConnectPrompt() {
        Task {
            // Every copy carries its own single-use invite; the mint is
            // awaited when no cached one is ready.
            let prompt = await store.copyableConnectPrompt(for: room)
            let pasteboard = NSPasteboard.general
            pasteboard.clearContents()
            pasteboard.setString(prompt, forType: .string)
        }
    }

    private func presenceSummary(at now: Date) -> String {
        ChatPresencePresentation.summary(
            members: store.roomMembers,
            currentAgentID: store.agentID,
            fallbackMemberCount: room.memberCount,
            fallbackMemberCountIncludesCurrentAgent:
                store.fallbackMemberCountIncludesCurrentAgent(in: room.id),
            recentActivityByAgent: store.recentAgentActivityAt[room.id],
            now: now,
            connectionStatus: store.connectionStatus
        )
    }

    private var messageList: some View {
        TimelineView(.periodic(from: .now, by: 60)) { timeline in
            ScrollViewReader { proxy in
                ZStack(alignment: .bottom) {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 22) {

                        ForEach(store.messages) { message in
                            MessageFeedRow(
                                message: message,
                                isMine: message.agentID == store.agentID,
                                now: timeline.date
                            )
                            .id(message.id)
                        }

                            if let thinkingText {
                                HStack(spacing: 8) {
                                    GallopIconView(icon: .thinking, fallbackSystemName: "arrow.triangle.2.circlepath", size: 16)
                                        .foregroundStyle(SemanticColor.buttonPrimaryDefault)
                                    Text(thinkingText)
                                        .gallopText(.bodyL, color: SemanticColor.textTertiary)
                                }
                                .id("thinking-indicator")
                            }

                            Color.clear
                                .frame(height: 1)
                                .id("message-list-bottom")
                                .onAppear {
                                    isMessageListNearBottom = true
                                    newMessageCount = 0
                                }
                                .onDisappear { isMessageListNearBottom = false }
                        }
                        .padding(.horizontal, 20)
                        .padding(.top, 18)
                        .padding(.bottom, isComposerExpanded ? 86 : 72)
                    }
                    .scrollIndicators(.hidden)
                    .opacity(hasPositionedInitialScroll ? 1 : 0)
                    .animation(.easeOut(duration: 0.15), value: hasPositionedInitialScroll)

                    if newMessageCount > 0 {
                        Button(newMessageCount == 1 ? "1 new message" : "\(newMessageCount) new messages") {
                            withAnimation(.easeOut(duration: 0.2)) {
                                proxy.scrollTo("message-list-bottom", anchor: .bottom)
                            }
                            newMessageCount = 0
                        }
                        .buttonStyle(.plain)
                        .gallopText(.bodySStrong, color: SemanticColor.buttonPrimaryTextDefault)
                        .padding(.horizontal, 14)
                        .frame(height: 34)
                        .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                        .padding(.bottom, isComposerExpanded ? 92 : 78)
                        .macAccessibleAction(label: "Show new messages") {
                            proxy.scrollTo("message-list-bottom", anchor: .bottom)
                            newMessageCount = 0
                        }
                    }
                }
                .onChange(of: MessageArrivalIdentity.latest(in: store.messages)) { _ in
                    if !hasPositionedInitialScroll {
                        // First population after a room switch: jump to the
                        // bottom unanimated while the list is still hidden,
                        // then fade it in already in place.
                        DispatchQueue.main.async {
                            proxy.scrollTo("message-list-bottom", anchor: .bottom)
                            hasPositionedInitialScroll = true
                        }
                    } else if isMessageListNearBottom {
                        withAnimation(.easeOut(duration: 0.2)) {
                            proxy.scrollTo("message-list-bottom", anchor: .bottom)
                        }
                    } else {
                        newMessageCount += 1
                    }
                }
                .onChange(of: store.isLoadingMessages) { loading in
                    // A room with no history has nothing to position — reveal
                    // straight to the quiet-room state.
                    if !loading && store.messages.isEmpty {
                        hasPositionedInitialScroll = true
                    }
                }
                .onAppear {
                    if !store.messages.isEmpty {
                        proxy.scrollTo("message-list-bottom", anchor: .bottom)
                        hasPositionedInitialScroll = true
                    } else if !store.isLoadingMessages {
                        hasPositionedInitialScroll = true
                    }
                }
            }
        }
    }

    private var quietRoom: some View {
        VStack(spacing: 10) {
            GallopIconView(icon: .message, fallbackSystemName: "bubble.left", size: 24)
                .foregroundStyle(SemanticColor.iconTertiary)
            Text("This room is quiet")
                .gallopText(.h5, color: SemanticColor.textPrimary)
            Text("Bring an agent in with the connect prompt, or open the composer and say hello.")
                .gallopText(.bodyM, color: SemanticColor.textTertiary)
                .multilineTextAlignment(.center)

            Button {
                copyConnectPrompt()
                hasCopiedQuietRoomPrompt = true
            } label: {
                // Reserve the wider label's width (no resize on click) and keep
                // the chrome inside the label so the whole capsule is clickable.
                ZStack {
                    Text("Copy connect prompt").hidden()
                    Text(hasCopiedQuietRoomPrompt ? "Copied" : "Copy connect prompt")
                }
                .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                .padding(.horizontal, 18)
                .frame(height: 38)
                .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .padding(.top, 6)
            .macAccessibleAction(label: "Copy connect prompt") {
                copyConnectPrompt()
                hasCopiedQuietRoomPrompt = true
            }
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var thinkingText: String? {
        let names = store.roomMembers.filter {
            ($0.status ?? "").localizedCaseInsensitiveContains("thinking")
        }.map(\.name)
        guard !names.isEmpty else { return nil }
        return names.count == 1 ? "\(names[0]) is thinking…" : "\(names.joined(separator: ", ")) are thinking…"
    }

    @ViewBuilder
    private var composer: some View {
        if isComposerExpanded {
            expandedComposer
                .transition(.move(edge: .bottom).combined(with: .opacity))
        } else {
            Button {
                withAnimation(.easeInOut(duration: 0.18)) { isComposerExpanded = true }
            } label: {
                GallopIconView(icon: .edit, fallbackSystemName: "pencil", size: 16)
                    .foregroundStyle(SemanticColor.buttonSecondaryIconDefault)
                    .frame(width: 42, height: 42)
                    .background(SemanticColor.buttonSecondaryDefault, in: Circle())
                    .overlay {
                        Circle().stroke(SemanticColor.borderDefault, lineWidth: 1)
                    }
            }
            .buttonStyle(.plain)
            .help("Write a message")
            .macAccessibleAction(label: "Write a message") {
                withAnimation(.easeInOut(duration: 0.18)) { isComposerExpanded = true }
            }
            .padding(16)
            .transition(.scale.combined(with: .opacity))
        }
    }

    private var expandedComposer: some View {
        VStack(spacing: 0) {
            if room.encrypted {
                Label {
                    Text("Encrypted rooms are read-only in the macOS app.")
                } icon: {
                    GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 12)
                }
                .gallopText(.caption, color: SemanticColor.textError)
                .padding(.bottom, 8)
            } else if !store.connectionStatus.isConnected {
                Label("Offline — reconnect before sending.", systemImage: "wifi.slash")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    .padding(.bottom, 8)
            }

            HStack(spacing: 8) {
                ComposerTextField(
                    text: $store.draft,
                    placeholder: "Message \(room.name)",
                    isEnabled: !room.encrypted,
                    onSubmit: store.sendDraft,
                    onCancel: {
                        withAnimation(.easeInOut(duration: 0.18)) { isComposerExpanded = false }
                    }
                )
                .frame(height: ComposerTextField.naturalHeight)
                .padding(.horizontal, 16)
                .frame(height: 44)
                .background(
                    isFieldHovering ? SemanticColor.textfieldHover : SemanticColor.textfieldDefault,
                    in: RoundedRectangle(cornerRadius: 22, style: .continuous)
                )
                .overlay {
                    RoundedRectangle(cornerRadius: 22, style: .continuous)
                        .stroke(isFieldHovering ? SemanticColor.borderHover : SemanticColor.borderDefault, lineWidth: 1)
                }
                .onHover { isFieldHovering = $0 }

                Button { store.sendDraft() } label: {
                    GallopIconView(icon: .send, fallbackSystemName: "paperplane.fill", size: 18)
                        .foregroundStyle(SemanticColor.buttonPrimaryIconDefault)
                        .frame(width: 36, height: 36)
                        .background(SemanticColor.buttonPrimaryDefault, in: Circle())
                        .overlay { Circle().stroke(Palette.nugget300, lineWidth: 1) }
                }
                .buttonStyle(.plain)
                .disabled(!canSend)
                .opacity(canSend ? 1 : 0.4)
                .macAccessibleAction(
                    label: "Send message",
                    isEnabled: canSend,
                    action: store.sendDraft
                )
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity)
        .background(SemanticColor.surface500)
    }

    private var canSend: Bool {
        store.connectionStatus.isConnected
            && !room.encrypted
            && !store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

private struct MessageFeedRow: View {
    let message: ChatMessage
    let isMine: Bool
    let now: Date
    @State private var isHovering = false

    var body: some View {
        if isMine {
            HStack(alignment: .bottom) {
                Spacer(minLength: 120)
                VStack(alignment: .leading, spacing: 7) {
                    ExpandableMessageText(content: message.content, textColor: SemanticColor.textPrimary)
                    Text(relativeTimestamp)
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
                    .padding(.horizontal, 20)
                    .padding(.vertical, 16)
                    .background(
                        LinearGradient(
                            colors: [SemanticColor.surface300, SemanticColor.surface400],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        ),
                        in: UnevenRoundedRectangle(
                            topLeadingRadius: 24, bottomLeadingRadius: 24,
                            bottomTrailingRadius: 8, topTrailingRadius: 24, style: .continuous
                        )
                    )
                    .overlay {
                        UnevenRoundedRectangle(
                            topLeadingRadius: 24, bottomLeadingRadius: 24,
                            bottomTrailingRadius: 8, topTrailingRadius: 24, style: .continuous
                        )
                        .stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                    }
                    .frame(maxWidth: 720, alignment: .trailing)
            }
            .frame(maxWidth: .infinity)
        } else {
            HStack(alignment: .top, spacing: 11) {
                AgentAvatar(name: message.agentName, size: 24)
                VStack(alignment: .leading, spacing: 7) {
                    HStack(spacing: 8) {
                        Text(message.agentName)
                            .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                        Text(relativeTimestamp)
                            .gallopText(.caption, color: SemanticColor.textTertiary)
                        if let app = AgentAppResolver.resolvedApp(forAgentNamed: message.agentName),
                           AgentAppResolver.applicationURL(for: app) != nil {
                            OpenInAgentAppChip(app: app, isVisible: isHovering)
                        }
                    }
                    .modifier(OpenInAppAccessibility(label: openInLabel, value: relativeTimestamp, action: openInApp))
                    ExpandableMessageText(content: message.content)
                }
                .frame(maxWidth: 760, alignment: .leading)
                Spacer(minLength: 24)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .onHover { isHovering = $0 }
            .animation(.easeOut(duration: 0.12), value: isHovering)
        }
    }

    private var relativeTimestamp: String {
        let value = message.timestamp.cowchatRelativeTime(relativeTo: now)
        return value.isEmpty ? message.timestamp.cowchatTime : value
    }

    private var resolvedApp: AgentAppResolver.ResolvedApp? {
        guard !isMine,
              let app = AgentAppResolver.resolvedApp(forAgentNamed: message.agentName),
              AgentAppResolver.applicationURL(for: app) != nil else { return nil }
        return app
    }
    private var openInLabel: String? {
        // Keep the specific agent name in the announcement — several
        // claude-* agents can share a room, and the overlay replaces the
        // visual name/timestamp pair for VoiceOver users.
        resolvedApp.map { "\(message.agentName), open in \($0.displayName)" }
    }
    private func openInApp() { if let resolvedApp { AgentAppResolver.open(resolvedApp) } }
}

/// Cowboy hover pattern: layout-reserved, opacity-faded, hit-test-gated —
/// siblings never jump, VoiceOver gets a persistent action instead.
private struct OpenInAgentAppChip: View {
    let app: AgentAppResolver.ResolvedApp
    let isVisible: Bool
    @State private var isChipHovering = false

    var body: some View {
        Button {
            AgentAppResolver.open(app)
        } label: {
            HStack(spacing: 4) {
                Text("Open in \(app.displayName)")
                    .gallopText(.caption, color: SemanticColor.textSecondary)
                GallopIconView(icon: .arrowUpRight, fallbackSystemName: "arrow.up.right", size: 10)
                    .foregroundStyle(SemanticColor.iconSecondary)
            }
            .padding(.horizontal, 9)
            .frame(height: 22)
            .background(
                isChipHovering ? SemanticColor.buttonSecondaryHover : SemanticColor.surface600,
                in: Capsule()
            )
            .overlay { Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1) }
        }
        .buttonStyle(.plain)
        .onHover { isChipHovering = $0 }
        .opacity(isVisible ? 1 : 0)
        .allowsHitTesting(isVisible)
        .accessibilityHidden(!isVisible)
        .help("Open \(app.displayName)")
    }
}

/// `.macAccessibleAction` (`AccessibleActionOverlay.swift`) hides its entire
/// receiver subtree from VoiceOver and substitutes one overlay element, so
/// it may only wrap the name/timestamp row here — never the outer message
/// row, which also carries `ExpandableMessageText`'s body text and its own
/// "Show full response" accessible action. Separately, `isEnabled: false`
/// does not omit that overlay element (`ActionView.isAccessibilityElement()`
/// is unconditional — see `AccessibleActionOverlayTests`), so it would still
/// expose a disabled control with a placeholder label when no app resolves.
/// Skipping the modifier entirely avoids registering that phantom control.
private struct OpenInAppAccessibility: ViewModifier {
    let label: String?
    var value: String?
    let action: () -> Void

    func body(content: Content) -> some View {
        if let label {
            content.macAccessibleAction(label: label, value: value, action: action)
        } else {
            content
        }
    }
}

enum MessageRenderPlan: Equatable {
    // Rich block rendering is intentionally bounded. Heading-heavy or
    // machine-generated output can otherwise create tens of thousands of
    // SwiftUI children in one message row.
    static let richTextByteLimit = 128 * 1_024
    static let richTextLineBreakLimit = 320

    case structured([MessageContentSegment])
    case plainText(String)

    static func make(for source: String) -> MessageRenderPlan {
        if source.utf8.count > richTextByteLimit
            || reachesLineBreakLimit(in: source) {
            return .plainText(source)
        }
        return .structured(MessageContentParser.segments(in: source))
    }

    var renderedElementCount: Int {
        switch self {
        case .structured(let segments): segments.count
        case .plainText: 1
        }
    }

    private static func reachesLineBreakLimit(in source: String) -> Bool {
        var count = 0
        for character in source where character.isNewline {
            count += 1
            if count >= richTextLineBreakLimit { return true }
        }
        return false
    }
}

struct ExpandableMessageText: View {
    let content: String
    var textColor: Color
    @State private var isExpanded = false
    @State private var expandedPlan: MessageRenderPlan?
    private let collapsedPreview: MessagePreview.CollapsedPreview
    private let collapsedPlan: MessageRenderPlan
    private let makeRenderPlan: (String) -> MessageRenderPlan

    init(
        content: String,
        textColor: Color = SemanticColor.textSecondary,
        makeRenderPlan: @escaping (String) -> MessageRenderPlan = MessageRenderPlan.make
    ) {
        self.content = content
        self.textColor = textColor
        self.makeRenderPlan = makeRenderPlan

        let preview = MessagePreview.collapsedPreview(for: content)
        collapsedPreview = preview
        collapsedPlan = makeRenderPlan(preview.source)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if showsResponseControl {
                Button {
                    toggleExpansion()
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "bubble.left")
                            .font(.system(size: 10, weight: .medium))
                        Text(isExpanded ? "Hide full response" : "Show full response")
                            .gallopText(.caption)
                        GallopIconView(
                            icon: isExpanded ? .chevronDownExtraSmall : .chevronRightExtraSmall,
                            fallbackSystemName: isExpanded ? "chevron.down" : "chevron.right",
                            size: 10
                        )
                    }
                    .foregroundStyle(SemanticColor.textTertiary)
                }
                .buttonStyle(.plain)
                .macAccessibleAction(
                    label: isExpanded ? "Hide full response" : "Show full response",
                    value: isExpanded ? "expanded" : "collapsed"
                ) {
                    toggleExpansion()
                }
            }

            renderedContent(isExpanded ? (expandedPlan ?? collapsedPlan) : collapsedPlan)

            if !isExpanded, collapsedPreview.isTruncated {
                Text("…")
                    .gallopText(.bodyL, color: textColor)
                    .accessibilityLabel("Content continues")
            }
        }
    }

    @ViewBuilder
    private func renderedContent(_ plan: MessageRenderPlan) -> some View {
        switch plan {
        case .plainText(let source):
            Text(source)
                .textSelection(.enabled)
                .gallopText(.bodyL, color: textColor)
                .fixedSize(horizontal: false, vertical: true)
        case .structured(let segments):
            VStack(alignment: .leading, spacing: 8) {
                ForEach(segments) { segment in
                    switch segment.kind {
                    case .prose:
                        Text(MessageContentParser.attributedInline(segment.text))
                            .textSelection(.enabled)
                            .gallopText(.bodyL, color: textColor)
                            .fixedSize(horizontal: false, vertical: true)
                    case .heading(let level):
                        Text(MessageContentParser.attributedInline(segment.text))
                            .textSelection(.enabled)
                            .gallopText(headingStyle(for: level), color: textColor)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityAddTraits(.isHeader)
                    case .unorderedList, .orderedList:
                        Text(MessageContentParser.attributedInline(segment.text))
                            .textSelection(.enabled)
                            .gallopText(.bodyL, color: textColor)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.leading, 4)
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel(
                                segment.kind == .unorderedList ? "Bulleted list" : "Numbered list"
                            )
                            .accessibilityValue(accessibleText(segment.text))
                    case .blockQuote:
                        HStack(alignment: .top, spacing: 10) {
                            RoundedRectangle(cornerRadius: 1)
                                .fill(SemanticColor.borderDefault)
                                .frame(width: 3)
                            Text(MessageContentParser.attributedInline(segment.text))
                                .textSelection(.enabled)
                                .gallopText(.bodyL, color: textColor)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel("Block quote")
                        .accessibilityValue(accessibleText(segment.text))
                    case .table:
                        monospacedBlock(
                            segment.text,
                            style: .codeSm,
                            accessibilityLabel: "Table"
                        )
                    case .code:
                        monospacedBlock(
                            segment.text,
                            style: .code,
                            accessibilityLabel: "Code block"
                        )
                    }
                }
            }
        }
    }

    private var showsResponseControl: Bool {
        MessagePreview.needsDisclosure(for: content)
    }

    private func toggleExpansion() {
        if !isExpanded, expandedPlan == nil {
            expandedPlan = makeRenderPlan(content)
        }
        withAnimation(.easeInOut(duration: 0.16)) { isExpanded.toggle() }
    }

    private func headingStyle(for level: Int) -> GallopTextStyle {
        level <= 2 ? .h4 : .h5
    }

    private func accessibleText(_ source: String) -> String {
        String(MessageContentParser.attributedInline(source).characters)
    }

    private func monospacedBlock(
        _ source: String,
        style: GallopTextStyle,
        accessibilityLabel: String
    ) -> some View {
        ScrollView(.horizontal) {
            Text(source.isEmpty ? " " : source)
                .textSelection(.enabled)
                .gallopText(style, color: SemanticColor.textSecondary)
                .fixedSize(horizontal: true, vertical: true)
                .padding(12)
        }
        .scrollIndicators(.hidden)
        .background(
            SemanticColor.surface400,
            in: RoundedRectangle(cornerRadius: 10, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
        .accessibilityValue(source.isEmpty ? "Empty" : source)
    }
}

private struct AgentAvatar: View {
    let name: String
    let size: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
            .fill(SemanticColor.surfaceGlassOnDefault)
            .overlay {
                if let appIcon {
                    Image(nsImage: appIcon)
                        .resizable()
                        .scaledToFit()
                        .padding(size * 0.06)
                } else {
                    Text(initial)
                        .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
                        .foregroundStyle(SemanticColor.surfaceGlassOnTextDefault)
                }
            }
            .overlay {
                RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
                    .stroke(SemanticColor.borderDefault, lineWidth: 0.5)
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }

    private var appIcon: NSImage? {
        guard let app = AgentAppResolver.resolvedApp(forAgentNamed: name),
              let appURL = AgentAppResolver.applicationURL(for: app) else { return nil }
        return NSWorkspace.shared.icon(forFile: appURL.path)
    }

    private var initial: String {
        name.first.map { String($0).uppercased() } ?? "#"
    }
}

private struct RoomAvatar: View {
    let name: String
    let size: CGFloat
    let accented: Bool
    /// Settings previews pin a style instead of following the stored one.
    var styleOverride: RoomIconStyle?
    @AppStorage(RoomIconStyle.storageKey) private var iconStyle = RoomIconStyle.fallback.rawValue

    var body: some View {
        let style = styleOverride ?? RoomIconStyle(rawValue: iconStyle) ?? .fallback
        Group {
            if style == .initials {
                lettered
            } else {
                // Procedural styles carry their own colour, so selection reads
                // as a ring rather than a fill that would erase the icon.
                RoomIconView(name: name, style: style, size: size)
                    .clipShape(Circle())
                    .overlay {
                        Circle().stroke(
                            accented
                                ? SemanticColor.buttonPrimaryDefault
                                : SemanticColor.borderDefault.opacity(0.8),
                            lineWidth: accented ? 2 : 0.5
                        )
                    }
            }
        }
        .frame(width: size, height: size)
        .accessibilityElement()
        .accessibilityLabel(name)
    }

    private var lettered: some View {
        Circle()
            .fill(accented ? SemanticColor.buttonPrimaryDefault : avatarFill)
            .overlay {
                Text(initials)
                    .font(.system(size: size * 0.31, weight: .bold, design: .rounded))
                    .foregroundStyle(
                        accented
                            ? SemanticColor.buttonPrimaryTextDefault
                            : SemanticColor.textSecondary
                    )
            }
            .overlay {
                Circle().stroke(SemanticColor.borderDefault.opacity(0.8), lineWidth: 0.5)
            }
    }

    private var avatarFill: Color {
        let values = [
            SemanticColor.buttonSecondaryDefault,
            SemanticColor.surface400,
            SemanticColor.surface600,
            SemanticColor.surfaceGlassOnDefault,
        ]
        return values[index]
    }

    private var index: Int {
        abs(name.unicodeScalars.reduce(0) { $0 + Int($1.value) }) % 4
    }

    private var initials: String {
        let words = name.split(separator: " ").prefix(2)
        let value = words.compactMap(\.first).map(String.init).joined()
        return value.isEmpty ? "#" : value.uppercased()
    }
}

private struct CircleIconButton: View {
    let icon: GallopIcon?
    let fallbackSystemName: String
    let help: String
    var isEnabled = true
    let action: () -> Void
    @State private var isHovering = false

    init(
        icon: GallopIcon?,
        fallbackSystemName: String,
        help: String,
        isEnabled: Bool = true,
        action: @escaping () -> Void
    ) {
        self.icon = icon
        self.fallbackSystemName = fallbackSystemName
        self.help = help
        self.isEnabled = isEnabled
        self.action = action
    }

    /// Pre-Gallop call sites keep compiling unchanged.
    init(
        systemName: String,
        help: String,
        isEnabled: Bool = true,
        action: @escaping () -> Void
    ) {
        self.init(
            icon: nil,
            fallbackSystemName: systemName,
            help: help,
            isEnabled: isEnabled,
            action: action
        )
    }

    var body: some View {
        Button(action: action) {
            iconView
                .foregroundStyle(
                    isEnabled
                        ? SemanticColor.buttonSecondaryIconDefault
                        : SemanticColor.iconSubtle
                )
                .frame(width: 32, height: 32)
                .background(Circle().fill(backgroundFill))
                .overlay {
                    Circle().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                }
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .help(help)
        .onHover { isHovering = $0 }
        .macAccessibleAction(label: help, isEnabled: isEnabled, action: action)
    }

    private var backgroundFill: Color {
        isHovering ? SemanticColor.buttonGhostHover : .clear
    }

    @ViewBuilder
    private var iconView: some View {
        if let icon {
            GallopIconView(icon: icon, fallbackSystemName: fallbackSystemName, size: 15)
        } else {
            Label(help, systemImage: fallbackSystemName)
                .labelStyle(.iconOnly)
                .font(.system(size: 13, weight: .semibold))
        }
    }
}

private struct EmptyChatView: View {
    @EnvironmentObject private var store: ChatStore

    var body: some View {
        Group {
            if store.rooms.isEmpty {
                if case .failed = store.connectionStatus {
                    connectionFailedState
                } else {
                    // Centered welcome IS the empty state, with a direct path to
                    // the first room (Patrick, 2026-08-06).
                    VStack(spacing: 20) {
                        welcome(alignment: .center)

                        Button {
                            store.presentCreateRoom()
                        } label: {
                            Text("New room")
                                .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                                .padding(.horizontal, 20)
                                .frame(height: 38)
                                .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                        }
                        .buttonStyle(.plain)
                        .keyboardShortcut(.defaultAction)
                        .macAccessibleAction(label: "Create room") { store.presentCreateRoom() }
                    }
                    .padding(24)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            } else {
                VStack(alignment: .leading, spacing: 18) {
                    welcome(alignment: .leading)

                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), spacing: 12)], spacing: 12) {
                        ForEach(store.rooms.prefix(6)) { room in
                            Button {
                                Task { await store.select(room: room) }
                            } label: {
                                VStack(alignment: .leading, spacing: 12) {
                                    RoomAvatar(name: room.name, size: 38, accented: false)
                                    Text(room.name)
                                        .gallopText(.h4, color: SemanticColor.textPrimary)
                                    Text(room.description ?? "Open conversation")
                                        .gallopText(.caption, color: SemanticColor.textTertiary)
                                        .lineLimit(2)
                                }
                                .frame(maxWidth: .infinity, minHeight: 116, alignment: .topLeading)
                                .padding(16)
                                .background(SemanticColor.surface600, in: RoundedRectangle(cornerRadius: 12))
                                .overlay {
                                    RoundedRectangle(cornerRadius: 12)
                                        .stroke(SemanticColor.borderDefault, lineWidth: 1)
                                }
                            }
                            .buttonStyle(.plain)
                            .macAccessibleAction(label: "Open \(room.name)") {
                                Task { await store.select(room: room) }
                            }
                        }
                    }
                    Spacer()
                }
                .padding(24)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
        }
        .background(SemanticColor.surface500)
    }

    /// One source of truth for the welcome copy; alignment differs per branch.
    @ViewBuilder
    private func welcome(alignment: HorizontalAlignment) -> some View {
        VStack(alignment: alignment, spacing: 4) {
            Text("Howdy, welcome to Cowchat")
                .gallopText(.h4, color: SemanticColor.textPrimary)
            Text("Choose a local room or start a new conversation.")
                .gallopText(.bodyM, color: SemanticColor.textTertiary)
        }
        .multilineTextAlignment(alignment == .center ? .center : .leading)
    }

    private var connectionFailedState: some View {
        VStack(spacing: 18) {
            GallopIconView(icon: .retry, fallbackSystemName: "wifi.slash", size: 24)
                .foregroundStyle(SemanticColor.iconTertiary)
            Text("Can't reach the local server")
                .gallopText(.h5, color: SemanticColor.textPrimary)
            Text(store.connectionStatus.failureMessage ?? "Connection failed")
                .gallopText(.caption, color: SemanticColor.textTertiary)
                .multilineTextAlignment(.center)

            Button {
                store.reconnect()
            } label: {
                Text("Reconnect")
                    .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                    .padding(.horizontal, 20)
                    .frame(height: 38)
                    .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Reconnect", action: store.reconnect)

            ConnectTroubleshootingView()
                .frame(maxWidth: 420)
                .padding(.top, 8)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private enum SettingsPage {
    case connection
    case theme
    case roomIcons

    var title: String {
        switch self {
        case .connection: return "Connection"
        case .theme: return "Theme"
        case .roomIcons: return "Room icons"
        }
    }

    var subtitle: String {
        switch self {
        case .connection: return "Choose where Cowchat stores and syncs your rooms."
        case .theme: return "Choose how Cowchat appears on this Mac."
        case .roomIcons: return "Every room gets the same icon on every Mac — it is drawn from the room name."
        }
    }
}

enum ThemePreview {
    /// Freezes an adaptive token to a concrete color under the forced
    /// appearance. NSColor(token) alone stays dynamic — converting to a
    /// concrete color space inside the forced block is what snapshots it.
    static func color(_ token: Color, dark: Bool) -> Color {
        var resolved = token
        NSAppearance(named: dark ? .darkAqua : .aqua)!.performAsCurrentDrawingAppearance {
            if let concrete = NSColor(token).usingColorSpace(.sRGB) {
                resolved = Color(nsColor: concrete)
            }
        }
        return resolved
    }
}

private struct SettingsView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @EnvironmentObject private var store: ChatStore
    @Binding var isPresented: Bool
    let onShowOnboarding: () -> Void
    @AppStorage("CowchatMac.appearance") private var appearance = AppAppearance.system.rawValue
    @AppStorage(RoomIconStyle.storageKey) private var roomIconStyle = RoomIconStyle.fallback.rawValue
    @State private var selectedPage = SettingsPage.connection
    @State private var cloudURL = ""
    @State private var cloudAPIKey = ""
    @State private var isConnectingGlobal = false

    var body: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                Text("Preferences")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    .padding(.horizontal, 16)
                    .padding(.top, 20)
                    .padding(.bottom, 8)
                settingsNavigationRow(
                    "Connection",
                    systemName: "network",
                    page: .connection
                )
                settingsNavigationRow(
                    "Theme",
                    systemName: "circle.lefthalf.filled",
                    page: .theme
                )
                settingsNavigationRow(
                    "Room icons",
                    systemName: "circle.grid.2x2",
                    page: .roomIcons
                )
                Spacer()
            }
            .frame(width: 230)
            .background(SemanticColor.surface400)

            Rectangle().fill(SemanticColor.borderDefault).frame(width: 1)

            VStack(alignment: .leading, spacing: 24) {
                settingsHeader

                switch selectedPage {
                case .connection:
                    ScrollView {
                        connectionSettings
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                case .theme:
                    themeSettings
                case .roomIcons:
                    roomIconSettings
                }

                Spacer()
            }
            .padding(28)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(SemanticColor.surface600)
        }
        .frame(width: 780, height: 580)
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
        .onAppear(perform: loadCloudConfiguration)
    }

    private var settingsHeader: some View {
        HStack {
            VStack(alignment: .leading, spacing: 3) {
                Text(selectedPage.title)
                    .gallopText(.h4, color: SemanticColor.textPrimary)
                Text(selectedPage.subtitle)
                    .gallopText(.bodyM, color: SemanticColor.textTertiary)
            }
            Spacer()
            Button("Close") { isPresented = false }
                .buttonStyle(.plain)
                .gallopText(.bodySStrong, color: SemanticColor.buttonSecondaryTextDefault)
                .padding(.horizontal, 14)
                .frame(height: 32)
                .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
                .overlay {
                    Capsule().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                }
                .macAccessibleAction(label: "Close settings") { isPresented = false }
        }
    }

    private var connectionSettings: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text("Local server")
                        .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                    Spacer()
                    ServerStatusBadge(store: workspace.local)
                }
                Text("Always on. Cowchat starts its bundled server when needed, and your room database stays on this Mac.")
                    .gallopText(.bodyM, color: SemanticColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(16)
            .background(SemanticColor.surface500, in: RoundedRectangle(cornerRadius: 14))
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(SemanticColor.borderDefault, lineWidth: 1)
            }

            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text("Global server")
                        .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                    Spacer()
                    if let global = workspace.global {
                        ServerStatusBadge(store: global)
                    }
                }
                Text("Cowboy runs a shared Cowchat server for everyone. Connect and its rooms appear in the sidebar alongside your local rooms — a key is created for you automatically, or paste your own.")
                    .gallopText(.bodyM, color: SemanticColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
                TextField(ConnectionProfile.defaultGlobalURLString, text: $cloudURL)
                    .textFieldStyle(.plain)
                    .gallopText(.bodyM, color: SemanticColor.textPrimary)
                    .padding(.horizontal, 13)
                    .frame(height: 40)
                    .background(SemanticColor.textfieldDefault, in: Capsule())
                    .overlay {
                        Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
                    }
                SecureField("API key (optional — created automatically)", text: $cloudAPIKey)
                    .textFieldStyle(.plain)
                    .gallopText(.bodyM, color: SemanticColor.textPrimary)
                    .padding(.horizontal, 13)
                    .frame(height: 40)
                    .background(SemanticColor.textfieldDefault, in: Capsule())
                    .overlay {
                        Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
                    }
                GlobalServerNotice(
                    store: workspace.global ?? workspace.local,
                    hasGlobal: workspace.global != nil
                )
                HStack(spacing: 10) {
                    Spacer()
                    if workspace.isGlobalEnabled {
                        Button("Leave global rooms") { workspace.disableGlobalRooms() }
                            .buttonStyle(.plain)
                            .gallopText(.bodyMStrong, color: SemanticColor.buttonSecondaryTextDefault)
                            .padding(.horizontal, 20)
                            .frame(height: 42)
                            .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
                            .overlay {
                                Capsule().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                            }
                            .fixedSize()
                            .macAccessibleAction(label: "Leave global rooms") {
                                workspace.disableGlobalRooms()
                            }
                    }
                    Button(connectButtonTitle, action: saveCloudConfiguration)
                        .buttonStyle(.plain)
                        .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                        .padding(.horizontal, 20)
                        .frame(height: 42)
                        .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                        .disabled(!canSaveCloudConfiguration)
                        .opacity(canSaveCloudConfiguration ? 1 : 0.45)
                        .fixedSize()
                        .macAccessibleAction(
                            label: connectButtonTitle,
                            isEnabled: canSaveCloudConfiguration,
                            action: saveCloudConfiguration
                        )
                }
            }
            .padding(16)
            .background(SemanticColor.surface500, in: RoundedRectangle(cornerRadius: 14))
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(SemanticColor.borderDefault, lineWidth: 1)
            }
        }
    }


    private var themeSettings: some View {
        VStack(alignment: .leading, spacing: 24) {
            Picker("Appearance", selection: $appearance) {
                ForEach(AppAppearance.allCases) { option in
                    Text(option.label).tag(option.rawValue)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 360)

            HStack(spacing: 12) {
                themePreview(title: "Light", dark: false)
                themePreview(title: "Dark", dark: true)
            }

            Button("Show onboarding again") {
                isPresented = false
                onShowOnboarding()
            }
            .buttonStyle(.plain)
            .gallopText(.bodyMStrong, color: SemanticColor.buttonSecondaryTextDefault)
            .padding(.horizontal, 16)
            .frame(height: 38)
            .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
            .overlay {
                Capsule().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
            }
            .macAccessibleAction(label: "Show onboarding again") {
                isPresented = false
                onShowOnboarding()
            }
        }
    }

    private var roomIconSettings: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(RoomIconStyle.allCases) { style in
                    roomIconChoice(style)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func roomIconChoice(_ style: RoomIconStyle) -> some View {
        let selected = (RoomIconStyle(rawValue: roomIconStyle) ?? .fallback) == style
        return Button { roomIconStyle = style.rawValue } label: {
            HStack(spacing: 14) {
                roomIconSamples(style)
                VStack(alignment: .leading, spacing: 2) {
                    Text(style.label).gallopText(.bodyMStrong)
                    Text(style.blurb).gallopText(.caption)
                }
                Spacer(minLength: 0)
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(
                        selected ? SemanticColor.buttonPrimaryDefault : SemanticColor.iconSubtle
                    )
            }
            .foregroundStyle(SemanticColor.textSecondary)
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity)
            .frame(height: 74)
            .background(
                selected ? SemanticColor.surfaceGlassOnDefault : SemanticColor.surface500,
                in: RoundedRectangle(cornerRadius: 14)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(
                        selected ? SemanticColor.buttonPrimaryDefault : SemanticColor.borderDefault,
                        lineWidth: 1
                    )
            }
        }
        .buttonStyle(.plain)
        .macAccessibleAction(label: "Use \(style.label) room icons") {
            roomIconStyle = style.rawValue
        }
    }

    private func roomIconSamples(_ style: RoomIconStyle) -> some View {
        HStack(spacing: -8) {
            ForEach(iconSampleNames, id: \.self) { name in
                RoomAvatar(name: name, size: 44, accented: false, styleOverride: style)
                    .overlay {
                        Circle().stroke(SemanticColor.surface500, lineWidth: 2)
                    }
            }
        }
    }

    /// Previews use this Mac's own rooms so the choice is made against the
    /// names that will actually sit in the sidebar.
    private var iconSampleNames: [String] {
        let live = (workspace.local.rooms + (workspace.global?.rooms ?? []))
            .map(\.name).prefix(3)
        return live.isEmpty ? ["Lobby", "harness-signing", "canyon-deploy"] : Array(live)
    }

    private func settingsNavigationRow(
        _ title: String,
        systemName: String,
        page: SettingsPage
    ) -> some View {
        Button { selectedPage = page } label: {
            HStack(spacing: 9) {
                Image(systemName: systemName)
                    .font(.system(size: 13, weight: .medium))
                    .frame(width: 16)
                Text(title)
                    .gallopText(.bodyM)
                    .lineLimit(1)
                Spacer()
            }
            .foregroundStyle(
                selectedPage == page
                    ? SemanticColor.textPrimary
                    : SemanticColor.textSecondary
            )
            .padding(.horizontal, 12)
            .frame(height: 36)
            .background(
                selectedPage == page ? SemanticColor.surfaceGlassOnDefault : Color.clear,
                in: RoundedRectangle(cornerRadius: 10)
            )
            // Unselected rows have a clear background, which plain buttons
            // exclude from hit-testing — without this only the ink is tappable.
            .contentShape(RoundedRectangle(cornerRadius: 10))
            .padding(.horizontal, 8)
        }
        .buttonStyle(.plain)
    }

    private var canSaveCloudConfiguration: Bool {
        !cloudURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !isConnectingGlobal
    }

    private var connectButtonTitle: String {
        if isConnectingGlobal { return "Connecting…" }
        return workspace.isGlobalEnabled ? "Save and reconnect" : "Connect"
    }

    private func loadCloudConfiguration() {
        let configured = workspace.configuredGlobalValues()
        cloudURL = configured.url
        cloudAPIKey = configured.apiKey
    }

    private func saveCloudConfiguration() {
        guard canSaveCloudConfiguration else { return }
        isConnectingGlobal = true
        Task {
            if await workspace.connectToGlobal(url: cloudURL, apiKey: cloudAPIKey) {
                loadCloudConfiguration()
            }
            isConnectingGlobal = false
        }
    }

    private func themePreview(title: String, dark: Bool) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            RoundedRectangle(cornerRadius: 9)
                .fill(ThemePreview.color(SemanticColor.surface500, dark: dark))
                .overlay(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 7)
                        .fill(ThemePreview.color(SemanticColor.surface700, dark: dark))
                        .frame(width: 42)
                        .padding(6)
                }
                .frame(height: 90)
                .overlay {
                    RoundedRectangle(cornerRadius: 9)
                        .stroke(SemanticColor.borderDefault, lineWidth: 1)
                }
            Text(title)
                .gallopText(.bodySStrong, color: SemanticColor.textSecondary)
        }
        .frame(maxWidth: 190)
    }
}

private struct RenameRoomView: View {
    @EnvironmentObject private var store: ChatStore
    @Environment(\.dismiss) private var dismiss
    let room: Room
    @State private var name: String
    @State private var isRenaming = false

    init(room: Room) {
        self.room = room
        _name = State(initialValue: room.name)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Rename room")
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    Text("The new name is shared with everyone who can see this room.")
                        .gallopText(.bodyM, color: SemanticColor.textTertiary)
                }
                Spacer()
                CircleIconButton(icon: .dismiss, fallbackSystemName: "xmark", help: "Close", action: cancel)
            }

            TextField("Room name", text: $name)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .padding(.horizontal, 14)
                .frame(height: 42)
                .background(SemanticColor.textfieldDefault, in: Capsule())
                .overlay {
                    Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
                }
                .onSubmit(renameRoom)

            if let validationMessage {
                Text(validationMessage)
                    .gallopText(.caption, color: SemanticColor.textError)
            }

            Spacer()

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel", action: cancel)
                    .keyboardShortcut(.cancelAction)
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: SemanticColor.textSecondary)
                    .padding(.horizontal, 16)
                    .frame(height: 38)
                    .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
                    .macAccessibleAction(label: "Cancel", action: cancel)

                Button(isRenaming ? "Renaming…" : "Rename", action: renameRoom)
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.plain)
                    .gallopText(
                        .bodyMStrong,
                        color: canRename ? SemanticColor.buttonPrimaryTextDefault : SemanticColor.textDisabled
                    )
                    .padding(.horizontal, 18)
                    .frame(height: 38)
                    .background(
                        canRename ? SemanticColor.buttonPrimaryDefault : SemanticColor.buttonSecondaryDefault,
                        in: Capsule()
                    )
                    .disabled(!canRename)
                    .macAccessibleAction(
                        label: "Rename room",
                        isEnabled: canRename,
                        action: renameRoom
                    )
            }
        }
        .padding(26)
        .frame(width: 480, height: 260)
        .background(SemanticColor.surface600)
        .interactiveDismissDisabled(isRenaming)
    }

    private var trimmedName: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canRename: Bool {
        validationMessage == nil
            && trimmedName != room.name
            && !isRenaming
            && store.connectionStatus.isConnected
    }

    private var validationMessage: String? {
        if trimmedName.isEmpty { return "Enter a room name." }
        if trimmedName.unicodeScalars.count > 100 {
            return "Room names can contain at most 100 characters."
        }
        if trimmedName.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) {
            return "Room names cannot contain control characters."
        }
        return nil
    }

    private func renameRoom() {
        guard canRename else { return }
        isRenaming = true
        Task {
            if await store.rename(room, to: trimmedName) { dismiss() }
            isRenaming = false
        }
    }

    private func cancel() {
        guard !isRenaming else { return }
        store.roomBeingRenamed = nil
        dismiss()
    }
}

private struct ServerStatusBadge: View {
    @ObservedObject var store: ChatStore

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(statusColor)
                .frame(width: 7, height: 7)
            Text(store.connectionStatus.label)
                .gallopText(.caption, color: SemanticColor.textTertiary)
        }
    }

    private var statusColor: Color {
        switch store.connectionStatus {
        case .connected: return SemanticColor.success
        case .connecting: return SemanticColor.warning
        case .disconnected, .failed: return SemanticColor.textError
        }
    }
}

/// Inline warning in the Global settings card: setup problems from the
/// workspace, or the global store's own connection failure.
private struct GlobalServerNotice: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var store: ChatStore
    let hasGlobal: Bool

    var body: some View {
        if let message = noticeMessage {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(SemanticColor.warning)
                Text(message)
                    .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .padding(12)
            .background(
                SemanticColor.surfaceGlassOnDefault,
                in: RoundedRectangle(cornerRadius: 12)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .stroke(SemanticColor.borderDefault, lineWidth: 1)
            }
        }
    }

    private var noticeMessage: String? {
        workspace.globalSetupError
            ?? (hasGlobal ? store.connectionStatus.failureMessage : nil)
    }
}

private struct CreateRoomView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @EnvironmentObject private var store: ChatStore
    @ObservedObject var local: ChatStore
    @ObservedObject var globalOrLocal: ChatStore
    let hasGlobal: Bool
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var description = ""
    @State private var isCreating = false
    @State private var chosenServer: WorkspaceStore.Server?
    @State private var creationError: String?
    /// Ready-made name shown as the field's grey placeholder; an empty field
    /// creates the room under this name.
    @State private var suggestedName = RoomNameSuggestion.generate()

    private var effectiveName: String {
        let typed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return typed.isEmpty ? suggestedName : typed
    }

    private var parentRoom: Room? {
        guard let parentID = store.createRoomParentID else { return nil }
        return store.rooms.first { $0.id == parentID }
    }

    /// Nested rooms stay on their parent's server; top-level rooms default to
    /// whichever server owns the current selection.
    private var targetServer: WorkspaceStore.Server {
        if parentRoom != nil { return workspace.activeServer }
        return chosenServer ?? workspace.activeServer
    }

    private var targetStore: ChatStore {
        switch targetServer {
        case .local: return local
        case .global: return hasGlobal ? globalOrLocal : local
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text(parentRoom == nil ? "New room" : "New nested room")
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    Text(
                        parentRoom.map {
                            "Create a separate conversation inside \($0.name). Membership and history stay independent."
                        }
                            ?? (targetServer == .local
                                ? "Create a conversation on this Mac's local Cowchat server."
                                : "Create a conversation on the global Cowchat server.")
                    )
                        .gallopText(.bodyM, color: SemanticColor.textTertiary)
                }
                Spacer()
                CircleIconButton(
                    icon: .dismiss,
                    fallbackSystemName: "xmark",
                    help: "Close",
                    action: cancel
                )
            }

            VStack(spacing: 12) {
                if hasGlobal, parentRoom == nil {
                    Picker("Server", selection: serverSelection) {
                        ForEach(WorkspaceStore.Server.allCases) { server in
                            Text(server.label).tag(server)
                        }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                }
                styledField(suggestedName, text: $name)
                if !name.isEmpty, let nameValidationMessage {
                    Text(nameValidationMessage)
                        .gallopText(.caption, color: SemanticColor.textError)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 4)
                }
                styledField("Description (optional)", text: $description)
                if let creationError {
                    Text(creationError)
                        .gallopText(.caption, color: SemanticColor.textError)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 4)
                }
            }
            .onChange(of: name) { _ in creationError = nil }
            .onChange(of: chosenServer) { _ in creationError = nil }

            Spacer()

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel", action: cancel)
                    .keyboardShortcut(.cancelAction)
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: SemanticColor.textSecondary)
                    .padding(.horizontal, 16)
                    .frame(height: 38)
                    .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
                    .macAccessibleAction(label: "Cancel", action: cancel)

                Button(createButtonTitle) {
                    createRoom()
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.plain)
                .gallopText(
                    .bodyMStrong,
                    color: canCreate ? SemanticColor.buttonPrimaryTextDefault : SemanticColor.textDisabled
                )
                .padding(.horizontal, 18)
                .frame(height: 38)
                .background(
                    canCreate ? SemanticColor.buttonPrimaryDefault : SemanticColor.buttonSecondaryDefault,
                    in: Capsule()
                )
                .disabled(!canCreate)
                .macAccessibleAction(
                    label: "Create room",
                    isEnabled: canCreate,
                    action: createRoom
                )
            }
        }
        .padding(26)
        .frame(width: 480, height: 350)
        .background(SemanticColor.surface600)
    }

    private func styledField(_ placeholder: String, text: Binding<String>) -> some View {
        TextField(placeholder, text: text)
            .textFieldStyle(.plain)
            .gallopText(.bodyM, color: SemanticColor.textPrimary)
            .padding(.horizontal, 14)
            .frame(height: 42)
            .background(SemanticColor.textfieldDefault, in: Capsule())
            .overlay {
                Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
            }
    }

    private var serverSelection: Binding<WorkspaceStore.Server> {
        Binding(
            get: { targetServer },
            set: { chosenServer = $0 }
        )
    }

    private var canCreate: Bool {
        nameValidationMessage == nil
            && !isCreating
            && targetStore.connectionStatus.isConnected
    }

    private var nameValidationMessage: String? {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        // Empty is fine — the suggested placeholder name is used instead.
        if trimmedName.isEmpty { return nil }
        if trimmedName.unicodeScalars.count > 100 {
            return "Room names can contain at most 100 characters."
        }
        if trimmedName.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) {
            return "Room names cannot contain control characters."
        }
        return nil
    }

    private var createButtonTitle: String {
        if isCreating { return "Creating…" }
        return targetStore.connectionStatus.isConnected ? "Create Room" : "Connecting…"
    }

    private func createRoom() {
        guard canCreate else { return }
        isCreating = true
        creationError = nil
        let server = targetServer
        let target = targetStore
        let presenting = store
        let usedSuggestion = name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let creatingName = effectiveName
        Task {
            let outcome = await target.createRoom(
                name: creatingName,
                description: description,
                isPublic: true
            )
            isCreating = false
            switch outcome {
            case .created:
                // The room was created and selected on `server`; make that
                // server active so the chat pane shows it, and close the
                // sheet on whichever store presented it.
                workspace.activate(server: server)
                closeSheet(presenting: presenting)
            case .nameTaken:
                if let existing = target.rooms.first(where: {
                    $0.name.localizedCaseInsensitiveCompare(creatingName) == .orderedSame
                }) {
                    // The room already exists and this key can see it — just
                    // go there instead of arguing about the name.
                    await workspace.select(room: existing, on: server)
                    closeSheet(presenting: presenting)
                } else if usedSuggestion {
                    // Suggested name collided with a room this key can't
                    // see; deal a fresh suggestion rather than arguing.
                    suggestedName = RoomNameSuggestion.generate()
                    creationError = "\u{201C}\(creatingName)\u{201D} was already taken — here's a fresh suggestion."
                } else {
                    creationError = "A room named \u{201C}\(creatingName)\u{201D} already exists on this server, but it isn't visible to you. Choose another name."
                }
            case let .failed(message):
                creationError = message
            }
        }
    }

    private func closeSheet(presenting: ChatStore) {
        presenting.isCreateRoomPresented = false
        presenting.createRoomParentID = nil
        dismiss()
    }

    private func cancel() {
        store.createRoomParentID = nil
        dismiss()
    }
}
