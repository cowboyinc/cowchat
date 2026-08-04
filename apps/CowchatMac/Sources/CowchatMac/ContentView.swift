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
    @AppStorage("CowchatMac.appearance") private var appearance = AppAppearance.system.rawValue
    @State private var isSidebarVisible = true
    @State private var isSettingsPresented = false

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
                    ChatRoomView(room: room, isSidebarVisible: $isSidebarVisible)
                        .id(room.id)
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
        .sheet(isPresented: $store.isCreateRoomPresented) {
            CreateRoomView()
                .environmentObject(store)
        }
        .sheet(isPresented: $isSettingsPresented) {
            SettingsView(isPresented: $isSettingsPresented)
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
        .task { store.start() }
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
                        if baseRooms.isEmpty {
                            emptyRoomsState
                        } else {
                            pinnedRooms

                            ForEach(visibleGroups(at: timeline.date), id: \.title) { group in
                                roomGroup(group, now: timeline.date)
                            }

                            archiveSection(at: timeline.date)
                        }
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
            ? store.rooms
            : RoomSidebarPresentation.activeRooms(from: store.rooms)
        return RoomSidebarPresentation.filteredRooms(from: rooms, query: store.searchText)
    }

    private func visibleGroups(at now: Date) -> [RoomSidebarGroup] {
        RoomSidebarPresentation.groups(from: baseRooms, now: now)
            .filter { $0.title != "Earlier" }
    }

    private func archivedRooms(at now: Date) -> [Room] {
        RoomSidebarPresentation.groups(from: baseRooms, now: now)
            .first { $0.title == "Earlier" }?
            .rooms ?? []
    }

    private var activeCount: Int {
        RoomSidebarPresentation.activeRooms(from: store.rooms).count
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
                action: { store.isCreateRoomPresented = true }
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
                        Text("\(item == .all ? store.rooms.count : activeCount)")
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
                    label: "\(item.rawValue), \(item == .all ? store.rooms.count : activeCount)",
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
            TextField("Search rooms", text: $store.searchText)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: .textPrimary)
                .focused($isSearchFocused)
                .accessibilityLabel("Search rooms")
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
            from: store.rooms,
            among: baseRooms
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
                    RoomRow(room: room, isSelected: store.selectedRoomID == room.id, now: now)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
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
                            RoomRow(room: room, isSelected: store.selectedRoomID == room.id, now: now)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
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
            Image(systemName: store.searchText.isEmpty ? "person.2.slash" : "magnifyingglass")
                .font(.system(size: 20, weight: .medium))
                .foregroundStyle(GallopColor.iconTertiary.color)
            Text(emptyRoomsTitle)
                .gallopText(.bodyMStrong, color: .textSecondary)
            if !store.searchText.isEmpty {
                Text("Try another room name or description.")
                    .gallopText(.caption, color: .textTertiary)
                    .multilineTextAlignment(.center)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 18)
        .padding(.top, 44)
    }

    private var emptyRoomsTitle: String {
        if !store.searchText.isEmpty { return "No rooms found" }
        return scope == .active ? "No active rooms" : "No rooms available"
    }

    private var isArchiveVisible: Bool {
        isArchiveExpanded || !store.searchText.isEmpty
    }

    private var sidebarFooter: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(statusColor)
                .frame(width: 7, height: 7)
            Text(store.connectionStatus.label)
                .gallopText(.caption, color: .textSecondary)
            Spacer()
            if !store.connectionStatus.isConnected {
                Button("Reconnect") { store.start() }
                    .buttonStyle(.plain)
                    .gallopText(.caption, color: .surfaceGlassOnTextDefault)
                    .macAccessibleAction(label: "Reconnect") { store.start() }
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
        .frame(height: 52)
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
}

private struct RoomRow: View {
    let room: Room
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
        if let description = room.description, !description.isEmpty { return description }
        return room.ephemeral ? "Temporary room" : "Open conversation"
    }
}

private struct ChatRoomView: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room
    @Binding var isSidebarVisible: Bool
    @State private var isComposerExpanded = false

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

            Menu {
                Button("Create room") { store.isCreateRoomPresented = true }
                if !store.connectionStatus.isConnected {
                    Button("Reconnect") { store.start() }
                }
                Divider()
                Text(room.ephemeral ? "Temporary room" : "Persistent room")
                Text(room.visibility.capitalized)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(GallopColor.iconSecondary.color)
                    .frame(width: 32, height: 32)
                    .background(GallopColor.buttonSecondaryDefault.color, in: Circle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("Room actions")
        }
        .padding(.leading, isSidebarVisible ? 18 : 104)
        .padding(.trailing, 14)
        .frame(height: 58)
        .background(GallopColor.surface600.color)
    }

    private var displayedMemberCount: Int {
        if !store.roomMembers.isEmpty { return store.roomMembers.count }
        return max(room.memberCount ?? 0, store.connectionStatus.isConnected ? 1 : 0)
    }

    private var presenceSummary: String {
        let activeMembers = store.roomMembers.filter {
            ($0.status ?? "").localizedCaseInsensitiveCompare("idle") != .orderedSame
        }
        if !activeMembers.isEmpty {
            let names = activeMembers.prefix(2).map(\.name).joined(separator: " · ")
            return "\(names) active"
        }
        let count = displayedMemberCount
        return count == 1 ? "1 member" : "\(count) members"
    }

    private var messageList: some View {
        TimelineView(.periodic(from: .now, by: 60)) { timeline in
            ScrollViewReader { proxy in
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
                    }
                    .padding(.horizontal, 20)
                    .padding(.top, 18)
                    .padding(.bottom, isComposerExpanded ? 86 : 72)
                }
                .scrollIndicators(.hidden)
                .onChange(of: store.messages.count) { _ in
                    if let last = store.messages.last {
                        withAnimation(.easeOut(duration: 0.2)) {
                            proxy.scrollTo(last.id, anchor: .bottom)
                        }
                    }
                }
                .onAppear {
                    if let last = store.messages.last { proxy.scrollTo(last.id, anchor: .bottom) }
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
                    action: {}
                )
                .disabled(true)

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

            Text(displayedContent)
                .textSelection(.enabled)
                .gallopText(.bodyL, color: .textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var showsResponseControl: Bool {
        MessagePreview.needsDisclosure(for: content)
    }

    private var displayedContent: String {
        isExpanded ? content : MessagePreview.collapsedContent(for: content)
    }
}

private struct AgentAvatar: View {
    let name: String
    let size: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
            .fill(GallopColor.surfaceGlassOnDefault.color)
            .overlay {
                Text(initial)
                    .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
                    .foregroundStyle(GallopColor.surfaceGlassOnTextDefault.color)
            }
            .overlay {
                RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
                    .stroke(GallopColor.borderDefault.color, lineWidth: 0.5)
            }
            .frame(width: size, height: size)
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
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(help, systemImage: systemName)
                .labelStyle(.iconOnly)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(
                    isActive
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
        .help(help)
        .macAccessibleAction(label: help, action: action)
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
                    action: { store.isCreateRoomPresented = true }
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

private struct SettingsView: View {
    @EnvironmentObject private var store: ChatStore
    @Binding var isPresented: Bool
    @AppStorage("CowchatMac.appearance") private var appearance = AppAppearance.system.rawValue

    var body: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                Text("Preferences")
                    .gallopText(.caption, color: .textTertiary)
                    .padding(.horizontal, 16)
                    .padding(.top, 20)
                    .padding(.bottom, 8)
                settingsRow("Theme", systemName: "circle.lefthalf.filled", selected: true)

                Text("Rooms")
                    .gallopText(.caption, color: .textTertiary)
                    .padding(.horizontal, 16)
                    .padding(.top, 22)
                    .padding(.bottom, 8)
                ScrollView {
                    VStack(spacing: 2) {
                        ForEach(store.rooms.prefix(10)) { room in
                            settingsRow(room.name, systemName: "number", selected: false)
                        }
                    }
                }
                .scrollIndicators(.hidden)

                Spacer()
                Text("Cowchat")
                    .gallopText(.caption, color: .textTertiary)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 8)
                settingsRow("About", systemName: "info.circle", selected: false)
                    .padding(.bottom, 14)
            }
            .frame(width: 230)
            .background(GallopColor.surface400.color)

            Rectangle().fill(GallopColor.borderDefault.color).frame(width: 1)

            VStack(alignment: .leading, spacing: 24) {
                HStack {
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Theme")
                            .gallopText(.h4, color: .textPrimary)
                        Text("Choose how Cowchat appears on this Mac.")
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
    }

    private func settingsRow(_ title: String, systemName: String, selected: Bool) -> some View {
        HStack(spacing: 9) {
            Image(systemName: systemName)
                .font(.system(size: 13, weight: .medium))
                .frame(width: 16)
            Text(title)
                .gallopText(.bodyM)
                .lineLimit(1)
            Spacer()
        }
        .foregroundStyle(selected ? GallopColor.textPrimary.color : GallopColor.textSecondary.color)
        .padding(.horizontal, 12)
        .frame(height: 36)
        .background(
            selected ? GallopColor.surfaceGlassOnDefault.color : Color.clear,
            in: RoundedRectangle(cornerRadius: 10)
        )
        .padding(.horizontal, 8)
    }

    private func themePreview(title: String, dark: Bool) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            RoundedRectangle(cornerRadius: 9)
                .fill(dark ? Color(red: 0.11, green: 0.09, blue: 0.08) : Color(red: 0.98, green: 0.97, blue: 0.96))
                .overlay(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 7)
                        .fill(dark ? Color(red: 0.18, green: 0.15, blue: 0.13) : Color.white)
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
}

private struct CreateRoomView: View {
    @EnvironmentObject private var store: ChatStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var description = ""
    @State private var ephemeral = false
    @State private var isPublic = false
    @State private var isCreating = false

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("New room")
                        .gallopText(.h4, color: .textPrimary)
                    Text("Create a conversation on your local Cowchat server.")
                        .gallopText(.bodyM, color: .textTertiary)
                }
                Spacer()
                CircleIconButton(
                    systemName: "xmark",
                    help: "Close",
                    action: { dismiss() }
                )
            }

            VStack(spacing: 12) {
                styledField("Room name", text: $name)
                styledField("Description (optional)", text: $description)

                Toggle("Temporary room", isOn: $ephemeral)
                    .gallopText(.bodyM, color: .textSecondary)
                Toggle("Public room", isOn: $isPublic)
                    .gallopText(.bodyM, color: .textSecondary)
            }

            Spacer()

            HStack(spacing: 10) {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: .textSecondary)
                    .padding(.horizontal, 16)
                    .frame(height: 38)
                    .background(GallopColor.buttonSecondaryDefault.color, in: Capsule())
                    .macAccessibleAction(label: "Cancel") { dismiss() }

                Button("Create") {
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
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !isCreating
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
}
