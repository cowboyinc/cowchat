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
                    } else if store.roomSetupScreenIDs.contains(room.id) {
                        RoomSetupView(room: room, isSidebarVisible: $isSidebarVisible)
                            .id("setup-\(room.id)")
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
            CreateRoomView()
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
            }
        }
        .animation(.easeInOut(duration: 0.2), value: store.roomReadyNotice?.id)
    }
}

private struct SidebarView: View {
    @EnvironmentObject private var store: ChatStore
    @Binding var isSidebarVisible: Bool
    @Binding var isSettingsPresented: Bool
    @State private var isArchiveExpanded = false
    @FocusState private var isSearchFocused: Bool
    @State private var isLobbyHovering = false
    /// Fixed anchor for the relative-time schedules below. `.now` here is
    /// evaluated on every body evaluation, so the schedule is rebuilt and its
    /// interval restarted each time rather than ticking on a stable cadence.
    @State private var clockAnchor = Date()
    /// Once per process: the launch-focus clear must not repeat on sidebar re-mounts.
    private static var didClearLaunchFocus = false

    private var lobbyRoom: Room? {
        store.rooms.first { $0.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame }
    }

    var body: some View {
        VStack(spacing: 0) {
            if let lobby = lobbyRoom {
                lobbyNavRow(lobby)
                    .padding(.horizontal, 12)
                    .padding(.bottom, 10)
            }

            searchField
                .padding(.horizontal, 12)
                .padding(.bottom, 14)

            TimelineView(.periodic(from: clockAnchor, by: store.lastThinkingAt.isEmpty ? 60 : 10)) { timeline in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if baseRooms.isEmpty {
                            if isSearchActive {
                                emptyRoomsState
                            }
                        } else {
                            VStack(spacing: 0) {
                                ForEach(RoomSidebarPresentation.sortedByRecency(baseRooms)) { room in
                                    Button {
                                        Task { await store.select(room: room) }
                                    } label: {
                                        RoomRow(
                                            room: room,
                                            messagePreview: store.roomMessagePreviews[room.id],
                                            isSelected: store.selectedRoomID == room.id,
                                            isUnread: store.isUnread(room),
                                            isWorking: store.isWorking(room, at: timeline.date),
                                            now: timeline.date
                                        )
                                            .contentShape(Rectangle())
                                    }
                                    .buttonStyle(.plain)
                                    .contextMenu { roomContextMenu(for: room) }
                                    .macAccessibleAction(
                                        label: "Open \(room.name)",
                                        value: roomAccessibilityValue(for: room, now: timeline.date)
                                    ) {
                                        Task { await store.select(room: room) }
                                    }
                                }
                            }
                            .padding(.bottom, 10)
                        }

                    }
                    .padding(.horizontal, 8)
                    .padding(.bottom, 12)
                }
                .scrollIndicators(.hidden)
            }

            // Archive stays pinned above the footer instead of trailing the
            // room list (which floats it mid-sidebar when the list is short).
            TimelineView(.periodic(from: clockAnchor, by: 60)) { timeline in
                archiveSection(at: timeline.date)
                    .padding(.horizontal, 8)
            }

            sidebarFooter
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

    private var baseRooms: [Room] {
        // Lobby lives in its own nav row, so the idle table excludes it — but
        // an active search must still surface Lobby name/message hits.
        let searching = !store.searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let source = searching
            ? store.unarchivedRooms
            : store.unarchivedRooms.filter {
                $0.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame
            }
        return RoomSidebarPresentation.filteredRooms(
            from: source,
            query: store.searchText,
            matchingMessageRoomIDs: store.messageSearchRoomIDs
        )
    }

    /// Item D (quiet empty state): the sidebar only shows an explicit empty
    /// message while a search is actually in flight or has text typed. A
    /// genuinely empty room list (no search) renders nothing above Archive.
    private var isSearchActive: Bool {
        store.isSearchingMessages || !store.searchText.isEmpty
    }

    private func archivedRooms(at now: Date) -> [Room] {
        RoomSidebarPresentation.filteredRooms(
            from: store.archivedRooms,
            query: store.searchText,
            matchingMessageRoomIDs: store.messageSearchRoomIDs
        )
    }

    /// Dash-style nav destination: Lobby is Home, above the conversations
    /// table, not a row inside it (Patrick, 2026-08-06).
    private func lobbyNavRow(_ lobby: Room) -> some View {
        let isSelected = store.selectedRoomID == lobby.id
        return Button {
            Task { await store.select(room: lobby) }
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
                    state: .init(isSelected: isSelected, isHovering: isLobbyHovering)
                )
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isLobbyHovering = $0 }
        .macAccessibleAction(
            label: "Open Lobby",
            value: isSelected ? "selected" : nil
        ) {
            Task { await store.select(room: lobby) }
        }
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            GallopIconView(icon: .search, fallbackSystemName: "magnifyingglass", size: 13)
                .foregroundStyle(SemanticColor.iconTertiary)
            TextField("Search rooms or messages", text: $store.searchText)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .focused($isSearchFocused)
                .onExitCommand {
                    // Escape clears the filter and returns focus to the list.
                    store.searchText = ""
                    isSearchFocused = false
                }
                .accessibilityLabel("Search rooms or messages")
            if !store.searchText.isEmpty {
                Button { store.searchText = "" } label: {
                    GallopIconView(icon: .dismiss, fallbackSystemName: "xmark.circle.fill", size: 13)
                        .foregroundStyle(SemanticColor.iconSubtle)
                }
                .buttonStyle(.plain)
                .macAccessibleAction(label: "Clear room search") { store.searchText = "" }
            }
        }
        .padding(.horizontal, 11)
        .frame(height: 36)
        .background(SemanticColor.textfieldDefault, in: Capsule())
        .overlay {
            Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
    }

    private func archiveSection(at now: Date) -> some View {
        let rooms = archivedRooms(at: now)
        return VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(.easeInOut(duration: 0.18)) { isArchiveExpanded.toggle() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "archivebox")
                        .font(.system(size: 13, weight: .medium))
                        .offset(y: 1)  // optical center against the label ink
                    Text("Archive")
                        .gallopText(.bodySStrong)
                    Spacer()
                    if !rooms.isEmpty {
                        Text("\(rooms.count)")
                            .gallopText(.caption)
                    }
                    Image(systemName: isArchiveVisible ? "chevron.down" : "chevron.right")
                        .font(.system(size: 10, weight: .semibold))
                }
                .foregroundStyle(SemanticColor.textTertiary)
                .padding(.horizontal, 10)
                .frame(height: 36)
            }
            .buttonStyle(.plain)
            .macAccessibleAction(
                label: "Archive, \(rooms.count) rooms",
                value: isArchiveVisible ? "expanded" : "collapsed"
            ) {
                withAnimation(.easeInOut(duration: 0.18)) { isArchiveExpanded.toggle() }
            }

            if isArchiveVisible {
                if rooms.isEmpty {
                    Text("No rooms archived")
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                        .padding(.horizontal, 10)
                        .padding(.bottom, 8)
                } else {
                    // Bounded: the archive sits OUTSIDE the room-list scroll
                    // view, so an unbounded expansion would crush the room
                    // list and push the footer offscreen at min window height.
                    // Rows are a fixed 54pt; cap the reveal at ~4.5 rows.
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(rooms) { room in
                                Button {
                                    Task { await store.select(room: room) }
                                } label: {
                                    RoomRow(
                                        room: room,
                                        messagePreview: store.roomMessagePreviews[room.id],
                                        isSelected: store.selectedRoomID == room.id,
                                        isUnread: store.isUnread(room),
                                        isWorking: store.isWorking(room, at: now),
                                        now: now
                                    )
                                        .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .contextMenu {
                                    Button("Unarchive") { store.unarchive(room) }
                                }
                                .macAccessibleAction(
                                    label: "Open \(room.name)",
                                    value: roomAccessibilityValue(for: room, now: now)
                                ) {
                                    Task { await store.select(room: room) }
                                }
                            }
                        }
                    }
                    .scrollIndicators(.hidden)
                    .frame(height: min(CGFloat(rooms.count) * 54, 244))
                }
            }
        }
    }

    /// Only ever shown while `isSearchActive`, so the branch below always has
    /// search text (or an in-flight search) to react to — never the bare
    /// "no rooms at all" case, which item D renders as nothing instead.
    private var emptyRoomsState: some View {
        VStack(spacing: 8) {
            if store.isSearchingMessages {
                ProgressView()
                    .controlSize(.small)
            } else {
                GallopIconView(icon: .search, fallbackSystemName: "magnifyingglass", size: 20)
                    .foregroundStyle(SemanticColor.iconTertiary)
            }
            Text(emptyRoomsTitle)
                .gallopText(.bodyMStrong, color: SemanticColor.textSecondary)
            if !store.searchText.isEmpty {
                Text("Try another room, message, or agent name.")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    .multilineTextAlignment(.center)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 18)
        .padding(.top, 44)
    }

    private var emptyRoomsTitle: String {
        store.isSearchingMessages ? "Searching messages…" : "No rooms or messages found"
    }

    private var isArchiveVisible: Bool {
        isArchiveExpanded || !store.searchText.isEmpty
    }

    private var sidebarFooter: some View {
        HStack(spacing: 8) {
            Menu {
                Button {
                    store.useLocalConnection()
                } label: {
                    Label(
                        "Local",
                        systemImage: store.isLocalConnection ? "checkmark.circle.fill" : "desktopcomputer"
                    )
                }
                Button {
                    if store.isCowchatCloudConfigured {
                        store.useCowchatCloud()
                    } else {
                        isSettingsPresented = true
                    }
                } label: {
                    Label(
                        "Cowchat Cloud",
                        systemImage: !store.isLocalConnection ? "checkmark.circle.fill" : "cloud"
                    )
                }
                Divider()
                Button("Connection settings…") { isSettingsPresented = true }
            } label: {
                HStack(spacing: 8) {
                    Circle()
                        .fill(statusColor)
                        .frame(width: 7, height: 7)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(store.connectionTargetDescription)
                            .gallopText(.caption, color: SemanticColor.textSecondary)
                        Text(store.connectionStatus.label)
                            .gallopText(.dataLabel, color: SemanticColor.textTertiary)
                            .help(store.connectionStatus.failureMessage ?? store.connectionStatus.label)
                    }
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(SemanticColor.iconTertiary)
                }
                .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .help("Choose Local or Cowchat Cloud")
            Spacer()
            if !store.connectionStatus.isConnected {
                CircleIconButton(
                    icon: .retry,
                    fallbackSystemName: "arrow.clockwise",
                    help: "Reconnect",
                    action: store.reconnect
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

    private var statusColor: Color {
        switch store.connectionStatus {
        case .connected: return SemanticColor.success
        case .connecting: return SemanticColor.warning
        case .disconnected, .failed: return SemanticColor.textError
        }
    }

    @ViewBuilder
    private func roomContextMenu(for room: Room) -> some View {
        Button("Rename") { store.presentRename(room) }
            .disabled(!store.canRename(room))
        if room.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame {
            Button("Archive") {
                Task { await store.archive(room) }
            }
        }
    }

    /// Value announced by the AccessibleActionOverlay for a room row (see
    /// macAccessibleAction) — RoomRow's own accessibility subtree is hidden,
    /// so unread/selected state must be composed here to be announced at all.
    private func roomAccessibilityValue(for room: Room, now: Date) -> String? {
        let parts = [
            store.isWorking(room, at: now) ? "Agents working" : nil,
            store.isUnread(room) ? "Unread" : nil,
            store.selectedRoomID == room.id ? "selected" : nil,
        ].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
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
            Circle()
                .fill(isUnread ? Palette.nugget500 : Color.clear)
                .frame(width: 7, height: 7)
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
        .padding(.leading, 4)
        .frame(height: 54)
        .background(SidebarRowBackground(state: .init(isSelected: isSelected, isHovering: isHovering)))
        .onHover { isHovering = $0 }
    }

    private var roomSummary: String {
        if let messagePreview, !messagePreview.isEmpty { return messagePreview }
        if let description = room.description, !description.isEmpty { return description }
        return room.ephemeral ? "Temporary room" : "Open conversation"
    }
}

private struct LobbyDashboardView: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room
    @Binding var isSidebarVisible: Bool

    private var dashboardRooms: [Room] {
        store.unarchivedRooms.filter {
            $0.id != room.id && !store.setupRoomIDs.contains($0.id)
        }
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
                            ?? (room.ephemeral ? "Temporary room" : "Open conversation")
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

private struct RoomSetupView: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room
    @Binding var isSidebarVisible: Bool
    @State private var hasCopiedPrompt = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(room.name)
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    Text("Waiting for your first collaborator")
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                }
                Spacer()
            }
            .padding(.top, 10)
            .padding(.leading, 18)
            .padding(.trailing, 14)
            .frame(height: 58)

            VStack(spacing: 22) {
                HStack(spacing: 14) {
                    Image(systemName: "list.bullet.rectangle")
                    Image(systemName: "arrow.right")
                    Image(systemName: "sparkles")
                }
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(SemanticColor.iconPrimary)

                Text("Paste this prompt into one AI chatbot")
                    .gallopText(.h5, color: SemanticColor.textPrimary)

                HStack(alignment: .bottom, spacing: 14) {
                    Text(roomPrompt)
                        .textSelection(.enabled)
                        .gallopText(.bodyM, color: SemanticColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)

                    Button(hasCopiedPrompt ? "Copied" : "Copy") { copyPrompt() }
                        .buttonStyle(.plain)
                        .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                        .padding(.horizontal, 18)
                        .frame(height: 38)
                        .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                        .macAccessibleAction(label: "Copy setup prompt", action: copyPrompt)
                }
                .padding(18)
                .frame(maxWidth: 620)
                .background(
                    SemanticColor.surface600,
                    in: RoundedRectangle(cornerRadius: 16, style: .continuous)
                )
                .overlay {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .stroke(SemanticColor.borderDefault, lineWidth: 1)
                }

                Button("Continue") {
                    Task { await store.completeRoomSetup(room) }
                }
                .buttonStyle(.plain)
                .gallopText(.bodyMStrong, color: SemanticColor.buttonSecondaryTextDefault)
                .padding(.horizontal, 18)
                .frame(height: 38)
                .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
                .overlay {
                    Capsule().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                }
                .macAccessibleAction(label: "Finish room setup") {
                    Task { await store.completeRoomSetup(room) }
                }
            }
            .padding(28)
            // Center the unit against the full card, compensating the header
            // strip above (Patrick, 2026-08-06).
            .padding(.bottom, 68)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(SemanticColor.surface500)
    }

    private var roomPrompt: String { store.connectPrompt(for: room) }

    private func copyPrompt() {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(roomPrompt, forType: .string)
        hasCopiedPrompt = true
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
            Button("Open Room") {
                Task { await store.openRoomReadyNotice() }
            }
            .buttonStyle(.plain)
            .gallopText(.bodySStrong, color: SemanticColor.buttonPrimaryTextDefault)
            .padding(.horizontal, 14)
            .frame(height: 34)
            .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
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
    /// Fixed anchor for the message-feed relative-time schedule; see the note
    /// on `SidebarView.clockAnchor`.
    @State private var clockAnchor = Date()

    private var parentRoom: Room? {
        guard let parentID = room.parentID else { return nil }
        return store.rooms.first { $0.id == parentID }
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
                if !store.isLoadingMessages && store.messages.isEmpty {
                    quietRoom
                        .allowsHitTesting(true)
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
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button("Copy connect prompt") { copyConnectPrompt() }
                    Divider()
                    Button("Rename room") { store.presentRename(room) }
                        .disabled(!store.canRename(room))
                    Button("Archive room") { Task { await store.archive(room) } }
                    Divider()
                    Button("Create nested room…") { store.presentCreateRoom(parentID: room.id) }
                    if !store.connectionStatus.isConnected {
                        Button("Reconnect") { store.start() }
                    }
                    Divider()
                    Text(room.ephemeral ? "Temporary room" : "Persistent room")
                    Text(room.visibility.capitalized)
                    Divider()
                    Button("Destroy room…", role: .destructive) {
                        isDestroyConfirmationPresented = true
                    }
                    .disabled(!store.canDestroy(room) || isDestroyingRoom)
                } label: {
                    GallopIconView(icon: .ellipsis, fallbackSystemName: "ellipsis", size: 17)
                        .foregroundStyle(SemanticColor.iconSecondary)
                        .frame(width: 36, height: 36)
                        .contentShape(Circle())
                }
                .menuStyle(.borderlessButton)
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
                Text(presenceSummary)
                    .gallopText(.caption, color: presenceSummary.contains("active") ? SemanticColor.warning : SemanticColor.textTertiary)
                    .lineLimit(1)
            }

            Spacer()
        }
        .padding(.top, 10)
        .padding(.leading, 18)
        .padding(.trailing, 14)
        .frame(height: 58)
    }

    private func copyConnectPrompt() {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(store.connectPrompt(for: room), forType: .string)
    }

    private var presenceSummary: String {
        ChatPresencePresentation.summary(
            members: store.roomMembers,
            currentAgentID: store.agentID,
            fallbackMemberCount: room.memberCount,
            isConnected: store.connectionStatus.isConnected
        )
    }

    private var messageList: some View {
        TimelineView(.periodic(from: clockAnchor, by: 60)) { timeline in
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

            Button(hasCopiedQuietRoomPrompt ? "Copied" : "Copy connect prompt") {
                copyConnectPrompt()
                hasCopiedQuietRoomPrompt = true
            }
            .buttonStyle(.plain)
            .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
            .padding(.horizontal, 18)
            .frame(height: 38)
            .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
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
                CircleIconButton(
                    icon: .add,
                    fallbackSystemName: "plus",
                    help: "Attachments are coming soon",
                    isEnabled: false,
                    action: {}
                )

                ComposerTextField(
                    text: $store.draft,
                    placeholder: "Message \(room.name)",
                    isEnabled: !room.encrypted,
                    onSubmit: store.sendDraft
                )
                .frame(height: 22)
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

                Button {
                    withAnimation(.easeInOut(duration: 0.18)) { isComposerExpanded = false }
                } label: {
                    GallopIconView(icon: .dismiss, fallbackSystemName: "xmark", size: 12)
                        .foregroundStyle(SemanticColor.iconTertiary)
                        .frame(width: 24, height: 38)
                }
                .buttonStyle(.plain)
                .help("Close composer")
                .macAccessibleAction(label: "Close composer") {
                    withAnimation(.easeInOut(duration: 0.18)) { isComposerExpanded = false }
                }
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

private struct ExpandableMessageText: View {
    let content: String
    var textColor: Color = SemanticColor.textSecondary
    @State private var isExpanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if showsResponseControl {
                Button {
                    withAnimation(.easeInOut(duration: 0.16)) { isExpanded.toggle() }
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
                    withAnimation(.easeInOut(duration: 0.16)) { isExpanded.toggle() }
                }
            }

            ForEach(MessageContentParser.segments(in: displayedContent)) { segment in
                switch segment.kind {
                case .prose:
                    Text(markdown(segment.text))
                        .textSelection(.enabled)
                        .gallopText(.bodyL, color: textColor)
                        .fixedSize(horizontal: false, vertical: true)
                case .code:
                    ScrollView(.horizontal) {
                        Text(segment.text)
                            .textSelection(.enabled)
                            .gallopText(.code, color: SemanticColor.textSecondary)
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
                }
            }
        }
    }

    private var showsResponseControl: Bool {
        MessagePreview.needsDisclosure(for: content)
    }

    private var displayedContent: String {
        isExpanded ? content : MessagePreview.collapsedContent(for: content)
    }

    private func markdown(_ source: String) -> AttributedString {
        (try? AttributedString(markdown: source)) ?? AttributedString(source)
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

    var body: some View {
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
            .frame(width: size, height: size)
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
}

private enum SettingsPage {
    case connection
    case theme
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
    @EnvironmentObject private var store: ChatStore
    @Binding var isPresented: Bool
    let onShowOnboarding: () -> Void
    @AppStorage("CowchatMac.appearance") private var appearance = AppAppearance.system.rawValue
    @State private var selectedPage = SettingsPage.connection
    @State private var cloudURL = ""
    @State private var cloudAPIKey = ""

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
                Text(selectedPage == .connection ? "Connection" : "Theme")
                    .gallopText(.h4, color: SemanticColor.textPrimary)
                Text(
                    selectedPage == .connection
                        ? "Choose where Cowchat stores and syncs your rooms."
                        : "Choose how Cowchat appears on this Mac."
                )
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
            HStack(spacing: 12) {
                connectionChoice(
                    title: "Local",
                    detail: "Runs on this Mac",
                    systemName: "desktopcomputer",
                    selected: store.isLocalConnection,
                    action: store.useLocalConnection
                )
                connectionChoice(
                    title: "Cowchat Cloud",
                    detail: "Secure WebSocket",
                    systemName: "cloud",
                    selected: !store.isLocalConnection,
                    action: {
                        if store.isCowchatCloudConfigured {
                            store.useCowchatCloud()
                        }
                    }
                )
            }

            if let failureMessage = store.connectionStatus.failureMessage {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(SemanticColor.warning)
                    Text(failureMessage)
                        .gallopText(.bodyS, color: SemanticColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                .padding(12)
                .background(SemanticColor.surfaceGlassOnDefault, in: RoundedRectangle(cornerRadius: 12))
                .overlay {
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(SemanticColor.borderDefault, lineWidth: 1)
                }
            }

            VStack(alignment: .leading, spacing: 12) {
                Text("Local server")
                    .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                Text("Local is the default. Cowchat starts its bundled server when needed, and your room database stays on this Mac.")
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
                Text("Cowchat Cloud")
                    .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                TextField("wss://your-cowchat.example/ws", text: $cloudURL)
                    .textFieldStyle(.plain)
                    .gallopText(.bodyM, color: SemanticColor.textPrimary)
                    .padding(.horizontal, 13)
                    .frame(height: 40)
                    .background(SemanticColor.textfieldDefault, in: Capsule())
                    .overlay {
                        Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
                    }
                SecureField("API key", text: $cloudAPIKey)
                    .textFieldStyle(.plain)
                    .gallopText(.bodyM, color: SemanticColor.textPrimary)
                    .padding(.horizontal, 13)
                    .frame(height: 40)
                    .background(SemanticColor.textfieldDefault, in: Capsule())
                    .overlay {
                        Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
                    }
                HStack {
                    Label {
                        Text("Stored only in this Mac's Keychain")
                    } icon: {
                        GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 12)
                    }
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    Spacer()
                    Button("Save and connect", action: saveCloudConfiguration)
                        .buttonStyle(.plain)
                        .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                        .padding(.horizontal, 16)
                        .frame(height: 36)
                        .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                        .disabled(!canSaveCloudConfiguration)
                        .opacity(canSaveCloudConfiguration ? 1 : 0.45)
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
            .padding(.horizontal, 8)
        }
        .buttonStyle(.plain)
    }

    private func connectionChoice(
        title: String,
        detail: String,
        systemName: String,
        selected: Bool,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 11) {
                Image(systemName: systemName)
                    .font(.system(size: 18, weight: .medium))
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).gallopText(.bodyMStrong)
                    Text(detail).gallopText(.caption)
                }
                Spacer()
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(
                        selected
                            ? SemanticColor.buttonPrimaryDefault
                            : SemanticColor.iconSubtle
                    )
            }
            .foregroundStyle(SemanticColor.textSecondary)
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity)
            .frame(height: 64)
            .background(
                selected ? SemanticColor.surfaceGlassOnDefault : SemanticColor.surface500,
                in: RoundedRectangle(cornerRadius: 14)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(
                        selected
                            ? SemanticColor.buttonPrimaryDefault
                            : SemanticColor.borderDefault,
                        lineWidth: 1
                    )
            }
        }
        .buttonStyle(.plain)
    }

    private var canSaveCloudConfiguration: Bool {
        !cloudURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !cloudAPIKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func loadCloudConfiguration() {
        let configured = store.configuredCowchatCloudValues()
        cloudURL = configured.url
        cloudAPIKey = configured.apiKey
    }

    private func saveCloudConfiguration() {
        guard canSaveCloudConfiguration else { return }
        if store.saveAndUseCowchatCloud(url: cloudURL, apiKey: cloudAPIKey) {
            loadCloudConfiguration()
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

private struct CreateRoomView: View {
    @EnvironmentObject private var store: ChatStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var description = ""
    @State private var ephemeral = false
    @State private var isPublic = false
    @State private var isCreating = false

    private var parentRoom: Room? {
        guard let parentID = store.createRoomParentID else { return nil }
        return store.rooms.first { $0.id == parentID }
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
                            ?? "Create a conversation on your local Cowchat server."
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
                styledField("Room name", text: $name)
                if !name.isEmpty, let nameValidationMessage {
                    Text(nameValidationMessage)
                        .gallopText(.caption, color: SemanticColor.textError)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 4)
                }
                styledField("Description (optional)", text: $description)

                VStack(alignment: .leading, spacing: 14) {
                    VStack(alignment: .leading, spacing: 4) {
                        Toggle("Temporary room", isOn: $ephemeral)
                            .toggleStyle(.checkbox)
                            .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                        Text("Lives only while agents are connected — the room and its messages vanish when the last agent leaves.")
                            .gallopText(.caption, color: SemanticColor.textTertiary)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.leading, 20)
                    }
                    VStack(alignment: .leading, spacing: 4) {
                        Toggle("Public room", isOn: $isPublic)
                            .toggleStyle(.checkbox)
                            .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                        Text("Discoverable and joinable by any agent on this server, even with a different API key. Unchecked, only your API key can find it.")
                            .gallopText(.caption, color: SemanticColor.textTertiary)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.leading, 20)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 10)
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
        // Height grew from 390 with item E's wrapping-caption checkbox rows
        // (two-line-plus captions replacing two single-line toggles) —
        // judgment call pending the controller's own visual pass (skipped
        // here per Task 14's carried context).
        .frame(width: 480, height: 480)
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

    private var canCreate: Bool {
        nameValidationMessage == nil
            && !isCreating
            && store.connectionStatus.isConnected
    }

    private var nameValidationMessage: String? {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmedName.isEmpty { return "Enter a room name." }
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
        return store.connectionStatus.isConnected ? "Create Room" : "Connecting…"
    }

    private func createRoom() {
        guard canCreate else { return }
        isCreating = true
        Task {
            _ = await store.createRoom(
                name: name,
                description: description,
                ephemeral: ephemeral,
                isPublic: isPublic
            )
            isCreating = false
        }
    }

    private func cancel() {
        store.createRoomParentID = nil
        dismiss()
    }
}
