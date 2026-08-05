import AppKit
import SwiftUI

private typealias GallopColor = GallopTheme.ColorToken

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
                .frame(width: 300)
                .transition(.move(edge: .leading).combined(with: .opacity))

                Rectangle()
                    .fill(GallopColor.borderDefault.color)
                    .frame(width: 1)
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
                    EmptyChatView(isSidebarVisible: $isSidebarVisible)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(GallopColor.surface500.color)
        .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .stroke(GallopColor.borderDefault.color, lineWidth: 1)
                .allowsHitTesting(false)
        }
        .frame(minWidth: 900, minHeight: 600)
        .ignoresSafeArea(.container, edges: .top)
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

private enum SidebarScope: String, CaseIterable, Identifiable {
    case all = "All rooms"
    case active = "Active"

    var id: String { rawValue }
}

private struct SidebarView: View {
    @EnvironmentObject private var store: ChatStore
    @Binding var isSidebarVisible: Bool
    @Binding var isSettingsPresented: Bool
    @State private var scope = SidebarScope.all
    @State private var isSearchVisible = false
    @State private var isArchiveExpanded = false
    @FocusState private var isSearchFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            sidebarChrome
            scopePicker
                .padding(.horizontal, 12)
                .padding(.bottom, 12)

            if isSearchVisible {
                searchField
                    .padding(.horizontal, 12)
                    .padding(.bottom, 12)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }

            TimelineView(.periodic(from: .now, by: 60)) { timeline in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if baseRooms.isEmpty && archivedRooms(at: timeline.date).isEmpty {
                            emptyRoomsState
                        } else {
                            pinnedRooms

                            ForEach(visibleGroups(at: timeline.date), id: \.title) { group in
                                roomGroup(group, now: timeline.date)
                            }
                        }

                        archiveSection(at: timeline.date)
                    }
                    .padding(.horizontal, 8)
                    .padding(.bottom, 12)
                }
                .scrollIndicators(.hidden)
            }

            sidebarFooter
        }
        .background(GallopColor.surfaceGlass500.color)
    }

    private var baseRooms: [Room] {
        let rooms = scope == .all
            ? store.unarchivedRooms
            : RoomSidebarPresentation.activeRooms(
                from: store.unarchivedRooms,
                excludingCurrentClientFrom: store.connectionStatus.isConnected
                    ? store.selectedRoomID
                    : nil
            )
        return RoomSidebarPresentation.filteredRooms(
            from: rooms,
            query: store.searchText,
            matchingMessageRoomIDs: store.messageSearchRoomIDs
        )
    }

    private func visibleGroups(at now: Date) -> [RoomSidebarGroup] {
        let rooms = RoomSidebarPresentation.roomsForRecencyGroups(
            from: baseRooms,
            allRooms: store.unarchivedRooms,
            pinnedRoomIDs: store.pinnedRoomIDs
        )
        return RoomSidebarPresentation.groups(from: rooms, now: now)
    }

    private func archivedRooms(at now: Date) -> [Room] {
        let rooms = scope == .all
            ? store.archivedRooms
            : RoomSidebarPresentation.activeRooms(
                from: store.archivedRooms,
                excludingCurrentClientFrom: store.connectionStatus.isConnected
                    ? store.selectedRoomID
                    : nil
            )
        return RoomSidebarPresentation.filteredRooms(
            from: rooms,
            query: store.searchText,
            matchingMessageRoomIDs: store.messageSearchRoomIDs
        )
    }

    private var activeCount: Int {
        RoomSidebarPresentation.activeRooms(
            from: store.unarchivedRooms,
            excludingCurrentClientFrom: store.connectionStatus.isConnected
                ? store.selectedRoomID
                : nil
        ).count
    }

    private var sidebarChrome: some View {
        HStack(spacing: 8) {
            Spacer(minLength: 70)
            CircleIconButton(
                systemName: "rectangle.split.1x2",
                help: "Hide sidebar",
                action: {
                    store.searchText = ""
                    isSidebarVisible = false
                }
            )
            CircleIconButton(
                systemName: "square.and.pencil",
                help: "Create room",
                action: { store.presentCreateRoom() }
            )
            .keyboardShortcut("n", modifiers: .command)
        }
        .padding(.horizontal, 12)
        .frame(height: 52)
    }

    private var scopePicker: some View {
        HStack(spacing: 0) {
            ForEach(SidebarScope.allCases) { item in
                Button {
                    scope = item
                } label: {
                    HStack(spacing: 6) {
                        Text(item.rawValue)
                            .gallopText(.bodySStrong)
                        Text("\(item == .all ? store.unarchivedRooms.count : activeCount)")
                            .gallopText(.caption)
                            .opacity(0.72)
                    }
                    .foregroundStyle(
                        item == scope
                            ? GallopColor.surfaceGlassOnTextDefault.color
                            : GallopColor.surfaceGlassOffTextDefault.color
                    )
                    .frame(maxWidth: .infinity)
                    .frame(height: 34)
                    .background(
                        item == scope
                            ? GallopColor.surfaceGlassOnDefault.color
                            : Color.clear,
                        in: Capsule()
                    )
                }
                .buttonStyle(.plain)
                .macAccessibleAction(
                    label: "\(item.rawValue), \(item == .all ? store.unarchivedRooms.count : activeCount)",
                    value: item == scope ? "selected" : nil
                ) {
                    scope = item
                }
            }
        }
        .padding(3)
        .background(GallopColor.surface400.color, in: Capsule())
        .overlay {
            Capsule().stroke(GallopColor.borderDefault.color.opacity(0.7), lineWidth: 0.5)
        }
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(GallopColor.iconTertiary.color)
            TextField("Search rooms or messages", text: $store.searchText)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: .textPrimary)
                .focused($isSearchFocused)
                .accessibilityLabel("Search rooms or messages")
            if !store.searchText.isEmpty {
                Button { store.searchText = "" } label: {
                    Label("Clear room search", systemImage: "xmark.circle.fill")
                        .labelStyle(.iconOnly)
                        .foregroundStyle(GallopColor.iconSubtle.color)
                }
                .buttonStyle(.plain)
                .macAccessibleAction(label: "Clear room search") { store.searchText = "" }
            }
        }
        .padding(.horizontal, 11)
        .frame(height: 36)
        .background(GallopColor.textfieldDefault.color, in: Capsule())
        .overlay {
            Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 1)
        }
    }

    @ViewBuilder
    private var pinnedRooms: some View {
        let rooms = RoomSidebarPresentation.visiblePinnedRooms(
            from: store.unarchivedRooms,
            among: baseRooms,
            pinnedRoomIDs: store.pinnedRoomIDs
        )
        if !rooms.isEmpty {
            Text("Pinned")
                .gallopText(.caption, color: .textTertiary)
                .padding(.horizontal, 8)
                .padding(.bottom, 8)

            HStack(alignment: .top, spacing: 8) {
                ForEach(rooms) { room in
                    Button {
                        Task { await store.select(room: room) }
                    } label: {
                        VStack(spacing: 6) {
                            RoomAvatar(
                                name: room.name,
                                size: 38,
                                accented: store.selectedRoomID == room.id
                            )
                            Text(room.name)
                                .gallopText(.dataLabel, color: .textSecondary)
                                .lineLimit(1)
                                .frame(maxWidth: .infinity)
                        }
                    }
                    .buttonStyle(.plain)
                    .contextMenu { roomContextMenu(for: room) }
                    .frame(maxWidth: .infinity)
                    .macAccessibleAction(
                        label: "Open \(room.name)",
                        value: store.selectedRoomID == room.id ? "selected" : nil
                    ) {
                        Task { await store.select(room: room) }
                    }
                }
            }
            .padding(.horizontal, 4)
            .padding(.bottom, 16)
        }
    }

    private func roomGroup(_ group: RoomSidebarGroup, now: Date) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(group.title)
                .gallopText(.caption, color: .textTertiary)
                .padding(.horizontal, 8)
                .padding(.top, 4)

            ForEach(group.rooms) { room in
                Button {
                    Task { await store.select(room: room) }
                } label: {
                    RoomRow(
                        room: room,
                        messagePreview: store.roomMessagePreviews[room.id],
                        isSelected: store.selectedRoomID == room.id,
                        now: now
                    )
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .contextMenu { roomContextMenu(for: room) }
                .macAccessibleAction(
                    label: "Open \(room.name)",
                    value: store.selectedRoomID == room.id ? "selected" : nil
                ) {
                    Task { await store.select(room: room) }
                }
            }
        }
        .padding(.bottom, 10)
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
                .foregroundStyle(GallopColor.textTertiary.color)
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
                        .gallopText(.caption, color: .textTertiary)
                        .padding(.horizontal, 10)
                        .padding(.bottom, 8)
                } else {
                    ForEach(rooms) { room in
                        Button {
                            Task { await store.select(room: room) }
                        } label: {
                            RoomRow(
                                room: room,
                                messagePreview: store.roomMessagePreviews[room.id],
                                isSelected: store.selectedRoomID == room.id,
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
                            value: store.selectedRoomID == room.id ? "selected" : nil
                        ) {
                            Task { await store.select(room: room) }
                        }
                    }
                }
            }
        }
    }

    private var emptyRoomsState: some View {
        VStack(spacing: 8) {
            if store.isSearchingMessages {
                ProgressView()
                    .controlSize(.small)
            } else {
                Image(systemName: store.searchText.isEmpty ? "person.2.slash" : "magnifyingglass")
                    .font(.system(size: 20, weight: .medium))
                    .foregroundStyle(GallopColor.iconTertiary.color)
            }
            Text(emptyRoomsTitle)
                .gallopText(.bodyMStrong, color: .textSecondary)
            if !store.searchText.isEmpty {
                Text("Try another room, message, or agent name.")
                    .gallopText(.caption, color: .textTertiary)
                    .multilineTextAlignment(.center)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 18)
        .padding(.top, 44)
    }

    private var emptyRoomsTitle: String {
        if store.isSearchingMessages { return "Searching messages…" }
        if !store.searchText.isEmpty { return "No rooms or messages found" }
        return scope == .active ? "No active rooms" : "No rooms available"
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
                            .gallopText(.caption, color: .textSecondary)
                        Text(store.connectionStatus.label)
                            .gallopText(.dataLabel, color: .textTertiary)
                            .help(store.connectionStatus.failureMessage ?? store.connectionStatus.label)
                    }
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(GallopColor.iconTertiary.color)
                }
                .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .help("Choose Local or Cowchat Cloud")
            Spacer()
            if !store.connectionStatus.isConnected {
                CircleIconButton(
                    systemName: "arrow.clockwise",
                    help: "Reconnect",
                    action: store.reconnect
                )
            }
            CircleIconButton(
                systemName: "magnifyingglass",
                help: "Search rooms",
                isActive: isSearchVisible,
                action: {
                    withAnimation(.easeInOut(duration: 0.18)) {
                        isSearchVisible.toggle()
                        if isSearchVisible {
                            DispatchQueue.main.async { isSearchFocused = true }
                        } else {
                            isSearchFocused = false
                            store.searchText = ""
                        }
                    }
                }
            )
            CircleIconButton(
                systemName: "gearshape",
                help: "Settings",
                action: { isSettingsPresented = true }
            )
        }
        .padding(.horizontal, 12)
        .frame(height: 58)
        .background(GallopColor.surface600.color.opacity(0.78))
        .overlay(alignment: .top) {
            Rectangle().fill(GallopColor.borderDefault.color).frame(height: 1)
        }
    }

    private var statusColor: Color {
        switch store.connectionStatus {
        case .connected: return GallopColor.success.color
        case .connecting: return GallopColor.warning.color
        case .disconnected, .failed: return GallopColor.textError.color
        }
    }

    @ViewBuilder
    private func roomContextMenu(for room: Room) -> some View {
        Button("Rename") { store.presentRename(room) }
            .disabled(!store.canRename(room))
        Button(store.isPinned(room) ? "Unpin room" : "Pin room") {
            store.togglePinned(room)
        }
        if room.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame {
            Button("Archive") {
                Task { await store.archive(room) }
            }
        }
    }
}

private struct RoomRow: View {
    let room: Room
    let messagePreview: String?
    let isSelected: Bool
    let now: Date

    var body: some View {
        HStack(spacing: 10) {
            RoomAvatar(name: room.name, size: 40, accented: isSelected)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 5) {
                    Text(room.name)
                        .gallopText(.bodySStrong)
                        .lineLimit(1)
                    if room.encrypted {
                        Image(systemName: "lock.fill")
                            .font(.system(size: 9, weight: .semibold))
                    }
                    Spacer(minLength: 6)
                    Text(
                        (room.lastActivity ?? room.createdAt)
                            .cowchatRelativeTime(relativeTo: now)
                    )
                        .gallopText(.dataLabel)
                        .opacity(0.72)
                }

                Text(roomSummary)
                    .gallopText(.caption)
                    .lineLimit(1)
                    .opacity(0.78)
            }
        }
        .foregroundStyle(
            isSelected
                ? GallopColor.buttonPrimaryTextDefault.color
                : GallopColor.textSecondary.color
        )
        .padding(.horizontal, 8)
        .frame(height: 54)
        .background(
            isSelected ? GallopColor.buttonPrimaryDefault.color : Color.clear,
            in: RoundedRectangle(cornerRadius: 12, style: .continuous)
        )
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
                if !isSidebarVisible {
                    CircleIconButton(
                        systemName: "rectangle.split.1x2",
                        help: "Show sidebar",
                        action: { isSidebarVisible = true }
                    )
                }

                VStack(alignment: .leading, spacing: 1) {
                    Text("Lobby")
                        .gallopText(.bodyMStrong, color: .textPrimary)
                    Text("\(availableAgentCount) available agents · \(store.pinnedRoomIDs.count) pinned rooms")
                        .gallopText(.caption, color: .textTertiary)
                }

                Spacer()
                CircleIconButton(
                    systemName: "plus",
                    help: "Create room",
                    action: { store.presentCreateRoom() }
                )
            }
            .padding(.leading, isSidebarVisible ? 18 : 104)
            .padding(.trailing, 14)
            .frame(height: 58)
            .background(GallopColor.surface600.color)

            Rectangle().fill(GallopColor.borderDefault.color).frame(height: 1)

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
                                .foregroundStyle(GallopColor.buttonSecondaryIconDefault.color)
                                .frame(width: 34, height: 34)
                                .background(GallopColor.buttonSecondaryDefault.color, in: Circle())
                            Spacer(minLength: 12)
                            Text("New Room")
                                .gallopText(.bodyMStrong, color: .textPrimary)
                        }
                        .frame(maxWidth: .infinity, minHeight: 132, alignment: .topLeading)
                        .padding(16)
                        .background(
                            GallopColor.surface600.color,
                            in: RoundedRectangle(cornerRadius: 14, style: .continuous)
                        )
                        .overlay {
                            RoundedRectangle(cornerRadius: 14, style: .continuous)
                                .stroke(GallopColor.borderDefault.color, lineWidth: 1)
                        }
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
        .background(GallopColor.surface500.color)
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
                            Image(systemName: "lock.fill")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundStyle(GallopColor.iconTertiary.color)
                        }
                        Spacer()
                    }

                    Spacer(minLength: 8)
                    if let parentRoom {
                        Text("in \(parentRoom.name)")
                            .gallopText(.dataLabel, color: .textTertiary)
                            .lineLimit(1)
                    }
                    Text(room.name)
                        .gallopText(.bodyMStrong, color: .textPrimary)
                        .lineLimit(1)
                    Text(
                        store.roomMessagePreviews[room.id]
                            ?? room.description
                            ?? (room.ephemeral ? "Temporary room" : "Open conversation")
                    )
                        .gallopText(.caption, color: .textTertiary)
                        .lineLimit(2)
                }
                .frame(maxWidth: .infinity, minHeight: 132, alignment: .topLeading)
                .padding(16)
                .background(
                    GallopColor.surface600.color,
                    in: RoundedRectangle(cornerRadius: 14, style: .continuous)
                )
                .overlay {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .stroke(GallopColor.borderDefault.color, lineWidth: 1)
                }
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Open \(room.name)") {
                Task { await store.select(room: room) }
            }

            Menu {
                Button("Rename") { store.presentRename(room) }
                    .disabled(!store.canRename(room))
                Button(store.isPinned(room) ? "Unpin room" : "Pin room") {
                    store.togglePinned(room)
                }
                Button("Archive") {
                    Task { await store.archive(room) }
                }
            } label: {
                Label("Room actions", systemImage: "ellipsis")
                    .labelStyle(.iconOnly)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(GallopColor.iconTertiary.color)
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
                if !isSidebarVisible {
                    CircleIconButton(
                        systemName: "rectangle.split.1x2",
                        help: "Show sidebar",
                        action: { isSidebarVisible = true }
                    )
                }
                VStack(alignment: .leading, spacing: 1) {
                    Text(room.name)
                        .gallopText(.bodyMStrong, color: .textPrimary)
                    Text("Waiting for your first collaborator")
                        .gallopText(.caption, color: .textTertiary)
                }
                Spacer()
            }
            .padding(.leading, isSidebarVisible ? 18 : 104)
            .padding(.trailing, 14)
            .frame(height: 58)
            .background(GallopColor.surface600.color)

            Rectangle().fill(GallopColor.borderDefault.color).frame(height: 1)

            VStack(spacing: 22) {
                HStack(spacing: 14) {
                    Image(systemName: "list.bullet.rectangle")
                    Image(systemName: "arrow.right")
                    Image(systemName: "sparkles")
                }
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(GallopColor.iconPrimary.color)

                Text("Paste this prompt into one AI chatbot")
                    .gallopText(.h5, color: .textPrimary)

                HStack(alignment: .bottom, spacing: 14) {
                    Text(roomPrompt)
                        .textSelection(.enabled)
                        .gallopText(.bodyMStrong, color: .textSecondary)
                        .fixedSize(horizontal: false, vertical: true)

                    Button(hasCopiedPrompt ? "Copied" : "Copy") { copyPrompt() }
                        .buttonStyle(.plain)
                        .gallopText(.bodyMStrong, color: .buttonPrimaryTextDefault)
                        .padding(.horizontal, 18)
                        .frame(height: 38)
                        .background(GallopColor.buttonPrimaryDefault.color, in: Capsule())
                        .macAccessibleAction(label: "Copy setup prompt", action: copyPrompt)
                }
                .padding(18)
                .frame(maxWidth: 620)
                .background(
                    GallopColor.surface600.color,
                    in: RoundedRectangle(cornerRadius: 16, style: .continuous)
                )
                .overlay {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .stroke(GallopColor.borderDefault.color, lineWidth: 1)
                }

                Button("Continue") {
                    Task { await store.completeRoomSetup(room) }
                }
                .buttonStyle(.plain)
                .gallopText(.bodyMStrong, color: .buttonSecondaryTextDefault)
                .padding(.horizontal, 18)
                .frame(height: 38)
                .background(GallopColor.buttonSecondaryDefault.color, in: Capsule())
                .overlay {
                    Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 0.5)
                }
                .macAccessibleAction(label: "Finish room setup") {
                    Task { await store.completeRoomSetup(room) }
                }
            }
            .padding(28)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(GallopColor.surface500.color)
    }

    private var roomPrompt: String {
        """
        You're going to collaborate with another AI chatbot in real time over Cowchat. Read the Cowchat skill, \(store.agentConnectionInstruction), join the exact room “\(room.name)”, and start listening right away. https://cowchat.cowboy.inc/skills.txt
        """
    }

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
                    .gallopText(.bodyMStrong, color: .textPrimary)
                Text("You can now begin chatting with your collaborator.")
                    .gallopText(.caption, color: .textTertiary)
            }
            Button("Open Room") {
                Task { await store.openRoomReadyNotice() }
            }
            .buttonStyle(.plain)
            .gallopText(.bodySStrong, color: .buttonPrimaryTextDefault)
            .padding(.horizontal, 14)
            .frame(height: 34)
            .background(GallopColor.buttonPrimaryDefault.color, in: Capsule())
            .macAccessibleAction(label: "Open \(room.name)") {
                Task { await store.openRoomReadyNotice() }
            }
            Button {
                store.roomReadyNotice = nil
            } label: {
                Label("Dismiss", systemImage: "xmark")
                    .labelStyle(.iconOnly)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(GallopColor.iconTertiary.color)
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Dismiss room notice") {
                store.roomReadyNotice = nil
            }
        }
        .padding(14)
        .background(
            GallopColor.surface600.color,
            in: RoundedRectangle(cornerRadius: 14, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(GallopColor.borderDefault.color, lineWidth: 1)
        }
        .shadow(color: GallopColor.surfaceGlassBorderShadow.color, radius: 18, y: 8)
    }
}

private struct ChatRoomView: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room
    @Binding var isSidebarVisible: Bool
    @State private var isComposerExpanded = false
    @State private var isDestroyConfirmationPresented = false
    @State private var isDestroyingRoom = false
    @State private var isMessageListNearBottom = true
    @State private var newMessageCount = 0

    private var parentRoom: Room? {
        guard let parentID = room.parentID else { return nil }
        return store.rooms.first { $0.id == parentID }
    }

    var body: some View {
        VStack(spacing: 0) {
            chatHeader
            Rectangle()
                .fill(GallopColor.borderDefault.color)
                .frame(height: 1)

            ZStack(alignment: .bottomTrailing) {
                messageList
                composer
            }
        }
        .background(GallopColor.surface500.color)
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
    }

    private var chatHeader: some View {
        HStack(spacing: 10) {
            if !isSidebarVisible {
                CircleIconButton(
                    systemName: "rectangle.split.1x2",
                    help: "Show sidebar",
                    action: { isSidebarVisible = true }
                )
            }

            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    if let parentRoom {
                        Button(parentRoom.name) {
                            Task { await store.select(room: parentRoom) }
                        }
                        .buttonStyle(.plain)
                        .gallopText(.bodySStrong, color: .textTertiary)
                        .lineLimit(1)
                        .macAccessibleAction(label: "Open parent room \(parentRoom.name)") {
                            Task { await store.select(room: parentRoom) }
                        }
                        Image(systemName: "chevron.right")
                            .font(.system(size: 8, weight: .bold))
                            .foregroundStyle(GallopColor.iconSubtle.color)
                    }
                    Text(room.name)
                        .gallopText(.bodyMStrong, color: .textPrimary)
                    if room.encrypted {
                        Image(systemName: "lock.fill")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(GallopColor.iconTertiary.color)
                    }
                }
                Text(presenceSummary)
                    .gallopText(.caption, color: .textTertiary)
                    .lineLimit(1)
            }

            Spacer()

            HStack(spacing: 0) {
                Menu {
                    Button("Rename room") { store.presentRename(room) }
                        .disabled(!store.canRename(room))
                    Button(store.isPinned(room) ? "Unpin room" : "Pin room") {
                        store.togglePinned(room)
                    }
                    Button("Archive room") {
                        Task { await store.archive(room) }
                    }
                    Divider()
                    Button("Create nested room…") {
                        store.presentCreateRoom(parentID: room.id)
                    }
                    if !store.connectionStatus.isConnected {
                        Button("Reconnect") { store.start() }
                    }
                    Divider()
                    Text(room.ephemeral ? "Temporary room" : "Persistent room")
                    Text(room.visibility.capitalized)
                } label: {
                    Label("Room actions", systemImage: "ellipsis")
                        .labelStyle(.iconOnly)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(GallopColor.iconSecondary.color)
                        .frame(width: 34, height: 32)
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .accessibilityLabel("Room actions")

                Rectangle()
                    .fill(GallopColor.borderDefault.color)
                    .frame(width: 1, height: 18)

                Button {
                    isDestroyConfirmationPresented = true
                } label: {
                    Label("Destroy room", systemImage: "trash")
                        .labelStyle(.iconOnly)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(
                            store.canDestroy(room)
                                ? GallopColor.textError.color
                                : GallopColor.iconSubtle.color
                        )
                        .frame(width: 34, height: 32)
                }
                .buttonStyle(.plain)
                .disabled(!store.canDestroy(room) || isDestroyingRoom)
                .help(
                    store.canDestroy(room)
                        ? "Irreversibly remove this room from Cowchat"
                        : "Only the room creator can destroy it"
                )
                .macAccessibleAction(
                    label: "Destroy \(room.name)",
                    isEnabled: store.canDestroy(room) && !isDestroyingRoom
                ) {
                    isDestroyConfirmationPresented = true
                }
            }
            .padding(.horizontal, 2)
            .background(GallopColor.buttonSecondaryDefault.color, in: Capsule())
            .overlay {
                Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 0.5)
            }
        }
        .padding(.leading, isSidebarVisible ? 18 : 104)
        .padding(.trailing, 14)
        .frame(height: 58)
        .background(GallopColor.surface600.color)
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
        TimelineView(.periodic(from: .now, by: 60)) { timeline in
            ScrollViewReader { proxy in
                ZStack(alignment: .bottom) {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 22) {
                        if store.isLoadingMessages {
                            ProgressView()
                                .controlSize(.small)
                                .frame(maxWidth: .infinity)
                                .padding(.top, 32)
                        } else if store.messages.isEmpty {
                            quietRoom
                        }

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
                                    ProgressView().controlSize(.mini)
                                    Text(thinkingText)
                                        .gallopText(.caption, color: .textTertiary)
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

                    if newMessageCount > 0 {
                        Button(newMessageCount == 1 ? "1 new message" : "\(newMessageCount) new messages") {
                            withAnimation(.easeOut(duration: 0.2)) {
                                proxy.scrollTo("message-list-bottom", anchor: .bottom)
                            }
                            newMessageCount = 0
                        }
                        .buttonStyle(.plain)
                        .gallopText(.bodySStrong, color: .buttonPrimaryTextDefault)
                        .padding(.horizontal, 14)
                        .frame(height: 34)
                        .background(GallopColor.buttonPrimaryDefault.color, in: Capsule())
                        .padding(.bottom, isComposerExpanded ? 92 : 78)
                        .macAccessibleAction(label: "Show new messages") {
                            proxy.scrollTo("message-list-bottom", anchor: .bottom)
                            newMessageCount = 0
                        }
                    }
                }
                .onChange(of: MessageArrivalIdentity.latest(in: store.messages)) { _ in
                    if isMessageListNearBottom {
                        withAnimation(.easeOut(duration: 0.2)) {
                            proxy.scrollTo("message-list-bottom", anchor: .bottom)
                        }
                    } else {
                        newMessageCount += 1
                    }
                }
                .onAppear {
                    proxy.scrollTo("message-list-bottom", anchor: .bottom)
                }
            }
        }
    }

    private var quietRoom: some View {
        VStack(spacing: 10) {
            Image(systemName: "bubble.left")
                .font(.system(size: 24, weight: .medium))
                .foregroundStyle(GallopColor.iconTertiary.color)
            Text("This room is quiet")
                .gallopText(.h5, color: .textPrimary)
            Text("Open the composer and say hello.")
                .gallopText(.bodyM, color: .textTertiary)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 60)
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
                Label("Write a message", systemImage: "pencil")
                    .labelStyle(.iconOnly)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(GallopColor.buttonSecondaryIconDefault.color)
                    .frame(width: 42, height: 42)
                    .background(GallopColor.buttonSecondaryDefault.color, in: Circle())
                    .overlay {
                        Circle().stroke(GallopColor.borderDefault.color, lineWidth: 1)
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
                Label("Encrypted rooms are read-only in the macOS app.", systemImage: "lock.fill")
                    .gallopText(.caption, color: .textError)
                    .padding(.bottom, 8)
            } else if !store.connectionStatus.isConnected {
                Label("Offline — reconnect before sending.", systemImage: "wifi.slash")
                    .gallopText(.caption, color: .textTertiary)
                    .padding(.bottom, 8)
            }

            HStack(spacing: 8) {
                CircleIconButton(
                    systemName: "plus",
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
                .padding(.horizontal, 13)
                .frame(height: 42)
                .background(GallopColor.textfieldDefault.color, in: Capsule())
                .overlay {
                    Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 1)
                }

                Button { store.sendDraft() } label: {
                    Label("Send message", systemImage: "paperplane.fill")
                        .labelStyle(.iconOnly)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(GallopColor.buttonPrimaryIconDefault.color)
                        .frame(width: 38, height: 38)
                        .background(GallopColor.buttonPrimaryDefault.color, in: Circle())
                }
                .buttonStyle(.plain)
                .disabled(!canSend)
                .opacity(canSend ? 1 : 0.42)
                .macAccessibleAction(
                    label: "Send message",
                    isEnabled: canSend,
                    action: store.sendDraft
                )

                Button {
                    withAnimation(.easeInOut(duration: 0.18)) { isComposerExpanded = false }
                } label: {
                    Label("Close composer", systemImage: "xmark")
                        .labelStyle(.iconOnly)
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(GallopColor.iconTertiary.color)
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
        .background(GallopColor.surface600.color)
        .overlay(alignment: .top) {
            Rectangle().fill(GallopColor.borderDefault.color).frame(height: 1)
        }
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

    var body: some View {
        if isMine {
            HStack(alignment: .bottom) {
                Spacer(minLength: 120)
                VStack(alignment: .leading, spacing: 7) {
                    ExpandableMessageText(content: message.content)
                    Text(relativeTimestamp)
                        .gallopText(.caption, color: .textTertiary)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
                    .padding(.horizontal, 18)
                    .padding(.vertical, 13)
                    .background(
                        LinearGradient(
                            colors: [GallopColor.surface300.color, GallopColor.surface400.color],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        ),
                        in: RoundedRectangle(cornerRadius: 20, style: .continuous)
                    )
                    .overlay {
                        RoundedRectangle(cornerRadius: 20, style: .continuous)
                            .stroke(GallopColor.borderDefault.color, lineWidth: 0.5)
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
                            .gallopText(.bodyMStrong, color: .textPrimary)
                        Text(relativeTimestamp)
                            .gallopText(.caption, color: .textTertiary)
                    }
                    ExpandableMessageText(content: message.content)
                }
                .frame(maxWidth: 760, alignment: .leading)
                Spacer(minLength: 24)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var relativeTimestamp: String {
        let value = message.timestamp.cowchatRelativeTime(relativeTo: now)
        return value.isEmpty ? message.timestamp.cowchatTime : value
    }
}

private struct ExpandableMessageText: View {
    let content: String
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
                        Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                    }
                    .foregroundStyle(GallopColor.textTertiary.color)
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
                        .gallopText(.bodyL, color: .textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                case .code:
                    ScrollView(.horizontal) {
                        Text(segment.text)
                            .textSelection(.enabled)
                            .gallopText(.code, color: .textSecondary)
                            .fixedSize(horizontal: true, vertical: true)
                            .padding(12)
                    }
                    .scrollIndicators(.hidden)
                    .background(
                        GallopColor.surface400.color,
                        in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                    )
                    .overlay {
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .stroke(GallopColor.borderDefault.color, lineWidth: 1)
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
            .fill(GallopColor.surfaceGlassOnDefault.color)
            .overlay {
                if let appIcon {
                    Image(nsImage: appIcon)
                        .resizable()
                        .scaledToFit()
                        .padding(size * 0.06)
                } else {
                    Text(initial)
                        .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
                        .foregroundStyle(GallopColor.surfaceGlassOnTextDefault.color)
                }
            }
            .overlay {
                RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
                    .stroke(GallopColor.borderDefault.color, lineWidth: 0.5)
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }

    private var appIcon: NSImage? {
        let normalizedName = name.lowercased()
        let bundleID: String?
        if normalizedName.contains("claude") {
            bundleID = "com.anthropic.claudefordesktop"
        } else if normalizedName.contains("codex") {
            bundleID = "com.openai.codex"
        } else if normalizedName.contains("chatgpt") || normalizedName.contains("openai") {
            bundleID = "com.openai.chat"
        } else {
            bundleID = nil
        }
        guard let bundleID,
              let appURL = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) else {
            return nil
        }
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
            .fill(accented ? GallopColor.buttonPrimaryDefault.color : avatarFill)
            .overlay {
                Text(initials)
                    .font(.system(size: size * 0.31, weight: .bold, design: .rounded))
                    .foregroundStyle(
                        accented
                            ? GallopColor.buttonPrimaryTextDefault.color
                            : GallopColor.textSecondary.color
                    )
            }
            .overlay {
                Circle().stroke(GallopColor.borderDefault.color.opacity(0.8), lineWidth: 0.5)
            }
            .frame(width: size, height: size)
    }

    private var avatarFill: Color {
        let values = [
            GallopColor.buttonSecondaryDefault.color,
            GallopColor.surface400.color,
            GallopColor.surface600.color,
            GallopColor.surfaceGlassOnDefault.color,
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
    let systemName: String
    let help: String
    var isActive = false
    var isEnabled = true
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(help, systemImage: systemName)
                .labelStyle(.iconOnly)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(
                    !isEnabled
                        ? GallopColor.iconSubtle.color
                        : isActive
                        ? GallopColor.surfaceGlassOnTextDefault.color
                        : GallopColor.buttonSecondaryIconDefault.color
                )
                .frame(width: 32, height: 32)
                .background(
                    isActive
                        ? GallopColor.surfaceGlassOnDefault.color
                        : GallopColor.buttonSecondaryDefault.color,
                    in: Circle()
                )
                .overlay {
                    Circle().stroke(GallopColor.borderDefault.color, lineWidth: 0.5)
                }
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .help(help)
        .macAccessibleAction(label: help, isEnabled: isEnabled, action: action)
    }
}

private struct EmptyChatView: View {
    @EnvironmentObject private var store: ChatStore
    @Binding var isSidebarVisible: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                if !isSidebarVisible {
                    CircleIconButton(
                        systemName: "rectangle.split.1x2",
                        help: "Show sidebar",
                        action: { isSidebarVisible = true }
                    )
                }
                Text("Cowchat")
                    .gallopText(.bodyMStrong, color: .textPrimary)
                Spacer()
                CircleIconButton(
                    systemName: "square.and.pencil",
                    help: "Create room",
                    action: { store.presentCreateRoom() }
                )
            }
            .padding(.leading, isSidebarVisible ? 14 : 104)
            .padding(.trailing, 14)
            .frame(height: 58)
            .background(GallopColor.surface600.color)

            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Howdy, welcome to Cowchat")
                        .gallopText(.h4, color: .textPrimary)
                    Text("Choose a local room or start a new conversation.")
                        .gallopText(.bodyM, color: .textTertiary)
                }

                if store.rooms.isEmpty {
                    VStack(spacing: 10) {
                        Image(systemName: "bubble.left.and.bubble.right")
                            .font(.system(size: 28, weight: .medium))
                            .foregroundStyle(GallopColor.iconTertiary.color)
                        Text("No rooms available")
                            .gallopText(.h5, color: .textPrimary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), spacing: 12)], spacing: 12) {
                        ForEach(store.rooms.prefix(6)) { room in
                            Button {
                                Task { await store.select(room: room) }
                            } label: {
                                VStack(alignment: .leading, spacing: 12) {
                                    RoomAvatar(name: room.name, size: 38, accented: false)
                                    Text(room.name)
                                        .gallopText(.bodyMStrong, color: .textPrimary)
                                    Text(room.description ?? "Open conversation")
                                        .gallopText(.caption, color: .textTertiary)
                                        .lineLimit(2)
                                }
                                .frame(maxWidth: .infinity, minHeight: 116, alignment: .topLeading)
                                .padding(16)
                                .background(GallopColor.surface600.color, in: RoundedRectangle(cornerRadius: 12))
                                .overlay {
                                    RoundedRectangle(cornerRadius: 12)
                                        .stroke(GallopColor.borderDefault.color, lineWidth: 1)
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
            }
            .padding(24)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .background(GallopColor.surface500.color)
    }
}

private enum SettingsPage {
    case connection
    case theme
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
                    .gallopText(.caption, color: .textTertiary)
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
            .background(GallopColor.surface400.color)

            Rectangle().fill(GallopColor.borderDefault.color).frame(width: 1)

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
            .background(GallopColor.surface600.color)
        }
        .frame(width: 780, height: 580)
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(GallopColor.borderDefault.color, lineWidth: 1)
        }
        .onAppear(perform: loadCloudConfiguration)
    }

    private var settingsHeader: some View {
        HStack {
            VStack(alignment: .leading, spacing: 3) {
                Text(selectedPage == .connection ? "Connection" : "Theme")
                    .gallopText(.h4, color: .textPrimary)
                Text(
                    selectedPage == .connection
                        ? "Choose where Cowchat stores and syncs your rooms."
                        : "Choose how Cowchat appears on this Mac."
                )
                    .gallopText(.bodyM, color: .textTertiary)
            }
            Spacer()
            Button("Close") { isPresented = false }
                .buttonStyle(.plain)
                .gallopText(.bodySStrong, color: .buttonSecondaryTextDefault)
                .padding(.horizontal, 14)
                .frame(height: 32)
                .background(GallopColor.buttonSecondaryDefault.color, in: Capsule())
                .overlay {
                    Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 0.5)
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
                        .foregroundStyle(GallopColor.warning.color)
                    Text(failureMessage)
                        .gallopText(.bodyS, color: .textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                .padding(12)
                .background(GallopColor.surfaceGlassOnDefault.color, in: RoundedRectangle(cornerRadius: 12))
                .overlay {
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(GallopColor.borderDefault.color, lineWidth: 1)
                }
            }

            VStack(alignment: .leading, spacing: 12) {
                Text("Local server")
                    .gallopText(.bodyMStrong, color: .textPrimary)
                Text("Local is the default. Cowchat starts its bundled server when needed, and your room database stays on this Mac.")
                    .gallopText(.bodyM, color: .textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(16)
            .background(GallopColor.surface500.color, in: RoundedRectangle(cornerRadius: 14))
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(GallopColor.borderDefault.color, lineWidth: 1)
            }

            VStack(alignment: .leading, spacing: 12) {
                Text("Cowchat Cloud")
                    .gallopText(.bodyMStrong, color: .textPrimary)
                TextField("wss://your-cowchat.example/ws", text: $cloudURL)
                    .textFieldStyle(.plain)
                    .gallopText(.bodyM, color: .textPrimary)
                    .padding(.horizontal, 13)
                    .frame(height: 40)
                    .background(GallopColor.textfieldDefault.color, in: Capsule())
                    .overlay {
                        Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 1)
                    }
                SecureField("API key", text: $cloudAPIKey)
                    .textFieldStyle(.plain)
                    .gallopText(.bodyM, color: .textPrimary)
                    .padding(.horizontal, 13)
                    .frame(height: 40)
                    .background(GallopColor.textfieldDefault.color, in: Capsule())
                    .overlay {
                        Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 1)
                    }
                HStack {
                    Label("Stored only in this Mac's Keychain", systemImage: "lock.fill")
                        .gallopText(.caption, color: .textTertiary)
                    Spacer()
                    Button("Save and connect", action: saveCloudConfiguration)
                        .buttonStyle(.plain)
                        .gallopText(.bodyMStrong, color: .buttonPrimaryTextDefault)
                        .padding(.horizontal, 16)
                        .frame(height: 36)
                        .background(GallopColor.buttonPrimaryDefault.color, in: Capsule())
                        .disabled(!canSaveCloudConfiguration)
                        .opacity(canSaveCloudConfiguration ? 1 : 0.45)
                }
            }
            .padding(16)
            .background(GallopColor.surface500.color, in: RoundedRectangle(cornerRadius: 14))
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(GallopColor.borderDefault.color, lineWidth: 1)
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
            .gallopText(.bodyMStrong, color: .buttonSecondaryTextDefault)
            .padding(.horizontal, 16)
            .frame(height: 38)
            .background(GallopColor.buttonSecondaryDefault.color, in: Capsule())
            .overlay {
                Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 0.5)
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
                    ? GallopColor.textPrimary.color
                    : GallopColor.textSecondary.color
            )
            .padding(.horizontal, 12)
            .frame(height: 36)
            .background(
                selectedPage == page ? GallopColor.surfaceGlassOnDefault.color : Color.clear,
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
                            ? GallopColor.buttonPrimaryDefault.color
                            : GallopColor.iconSubtle.color
                    )
            }
            .foregroundStyle(GallopColor.textSecondary.color)
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity)
            .frame(height: 64)
            .background(
                selected ? GallopColor.surfaceGlassOnDefault.color : GallopColor.surface500.color,
                in: RoundedRectangle(cornerRadius: 14)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(
                        selected
                            ? GallopColor.buttonPrimaryDefault.color
                            : GallopColor.borderDefault.color,
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
                .fill(themePreviewColor(.surface500, dark: dark))
                .overlay(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 7)
                        .fill(themePreviewColor(.surface700, dark: dark))
                        .frame(width: 42)
                        .padding(6)
                }
                .frame(height: 90)
                .overlay {
                    RoundedRectangle(cornerRadius: 9)
                        .stroke(GallopColor.borderDefault.color, lineWidth: 1)
                }
            Text(title)
                .gallopText(.bodySStrong, color: .textSecondary)
        }
        .frame(maxWidth: 190)
    }

    private func themePreviewColor(_ token: GallopColor, dark: Bool) -> Color {
        Color(nsColor: token.rgba(for: dark ? .dark : .light).nsColor)
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
                        .gallopText(.h4, color: .textPrimary)
                    Text("The new name is shared with everyone who can see this room.")
                        .gallopText(.bodyM, color: .textTertiary)
                }
                Spacer()
                CircleIconButton(systemName: "xmark", help: "Close", action: cancel)
            }

            TextField("Room name", text: $name)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: .textPrimary)
                .padding(.horizontal, 14)
                .frame(height: 42)
                .background(GallopColor.textfieldDefault.color, in: Capsule())
                .overlay {
                    Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 1)
                }
                .onSubmit(renameRoom)

            if let validationMessage {
                Text(validationMessage)
                    .gallopText(.caption, color: .textError)
            }

            Spacer()

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel", action: cancel)
                    .keyboardShortcut(.cancelAction)
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: .textSecondary)
                    .padding(.horizontal, 16)
                    .frame(height: 38)
                    .background(GallopColor.buttonSecondaryDefault.color, in: Capsule())
                    .macAccessibleAction(label: "Cancel", action: cancel)

                Button(isRenaming ? "Renaming…" : "Rename", action: renameRoom)
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: .buttonPrimaryTextDefault)
                    .padding(.horizontal, 18)
                    .frame(height: 38)
                    .background(GallopColor.buttonPrimaryDefault.color, in: Capsule())
                    .disabled(!canRename)
                    .opacity(canRename ? 1 : 0.45)
                    .macAccessibleAction(
                        label: "Rename room",
                        isEnabled: canRename,
                        action: renameRoom
                    )
            }
        }
        .padding(26)
        .frame(width: 480, height: 260)
        .background(GallopColor.surface600.color)
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
                        .gallopText(.h4, color: .textPrimary)
                    Text(
                        parentRoom.map {
                            "Create a separate conversation inside \($0.name). Membership and history stay independent."
                        }
                            ?? "Create a conversation on your local Cowchat server."
                    )
                        .gallopText(.bodyM, color: .textTertiary)
                }
                Spacer()
                CircleIconButton(
                    systemName: "xmark",
                    help: "Close",
                    action: cancel
                )
            }

            VStack(spacing: 12) {
                styledField("Room name", text: $name)
                if !name.isEmpty, let nameValidationMessage {
                    Text(nameValidationMessage)
                        .gallopText(.caption, color: .textError)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 4)
                }
                styledField("Description (optional)", text: $description)

                Toggle("Temporary room", isOn: $ephemeral)
                    .gallopText(.bodyM, color: .textSecondary)
                Toggle("Public room", isOn: $isPublic)
                    .gallopText(.bodyM, color: .textSecondary)
            }

            Spacer()

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel", action: cancel)
                    .keyboardShortcut(.cancelAction)
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: .textSecondary)
                    .padding(.horizontal, 16)
                    .frame(height: 38)
                    .background(GallopColor.buttonSecondaryDefault.color, in: Capsule())
                    .macAccessibleAction(label: "Cancel", action: cancel)

                Button(createButtonTitle) {
                    createRoom()
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.plain)
                .gallopText(.bodyMStrong, color: .buttonPrimaryTextDefault)
                .padding(.horizontal, 18)
                .frame(height: 38)
                .background(GallopColor.buttonPrimaryDefault.color, in: Capsule())
                .disabled(!canCreate)
                .opacity(canCreate ? 1 : 0.45)
                .macAccessibleAction(
                    label: "Create room",
                    isEnabled: canCreate,
                    action: createRoom
                )
            }
        }
        .padding(26)
        .frame(width: 480, height: 390)
        .background(GallopColor.surface600.color)
    }

    private func styledField(_ placeholder: String, text: Binding<String>) -> some View {
        TextField(placeholder, text: text)
            .textFieldStyle(.plain)
            .gallopText(.bodyM, color: .textPrimary)
            .padding(.horizontal, 14)
            .frame(height: 42)
            .background(GallopColor.textfieldDefault.color, in: Capsule())
            .overlay {
                Capsule().stroke(GallopColor.borderDefault.color, lineWidth: 1)
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
        return store.connectionStatus.isConnected ? "Create" : "Connecting…"
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
