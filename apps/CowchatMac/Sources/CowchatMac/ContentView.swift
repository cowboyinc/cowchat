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
    @State private var isWhatsNewPresented = false
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
                switch MainArea.content(page: workspace.page, selectedRoom: store.selectedRoom) {
                case .page(.inbox):
                    InboxPage(inbox: workspace.inbox, home: workspace.homeSessions)
                case .page(.team):
                    TeamPage(team: workspace.team, account: workspace.accountGlobal ?? workspace.local)
                case .page(.friends):
                    FriendsPage(social: workspace.social, newRoom: { workspace.newRoom(with: $0) },
                                signIn: onShowOnboarding)
                case .chat(let room):
                    ChatRoomView(room: room, isSidebarVisible: $isSidebarVisible)
                        .id(room.id)
                case .empty:
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
        .environmentObject(workspace.homeSessions)
        // The unified toolbar otherwise paints its own system strip; tint it
        // with the same surface400 shell so the nav reads as one tan surface.
        .toolbarBackground(SemanticColor.surface400, for: .windowToolbar)
        .navigationTitle(windowTitle)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                // copy:
                Button { workspace.show(.friends) } label: {
                    Label("Friends", systemImage: "person.2")
                }
                .help("Friends")
                .macAccessibleAction(label: "Open Friends") { workspace.show(.friends) }
            }
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
        // A social notification's click: a friend request opens the Friends page, and a room invitation the Inbox.
        .onReceive(workspace.notifications.$destination.receive(on: RunLoop.main)) { destination in
            guard let destination, workspace.notifications.consumeDestination(destination) else { return }
            isSettingsPresented = false
            store.isCreateRoomPresented = false
            isWhatsNewPresented = false
            store.roomBeingRenamed = nil
            workspace.roomDraft = nil
            workspace.roomPeople = nil
            workspace.show(destination.page)
        }
        .onReceive(NotificationCenter.default.publisher(for: .cowchatShowFriends)) { _ in
            workspace.show(.friends)
        }
        .sheet(item: $workspace.roomDraft) { draft in
            SocialRoomCreationView(social: workspace.social, draft: draft) { roomID in
                Task { _ = await workspace.openCreatedSocialRoom(roomID: roomID) }
            }
        }
        .sheet(isPresented: Binding(get: { workspace.roomPeople != nil },
                                    set: { if !$0 { workspace.roomPeople = nil } })) {
            RoomParticipantsSheet(social: workspace.social, participants: $workspace.roomPeople)
        }
        .sheet(
            isPresented: $store.isCreateRoomPresented,
            onDismiss: { store.createRoomParentID = nil }
        ) {
            RoomCreationSheet(initialSource: workspace.activeServer)
                .environmentObject(store)
        }
        .onReceive(NotificationCenter.default.publisher(for: .cowchatShowWhatsNew)) { _ in
            guard controlActiveState == .key || controlActiveState == .active else { return }
            isWhatsNewPresented = true
        }
        .sheet(isPresented: $isWhatsNewPresented) { WhatsNewView() }
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

    // copy:
    /// The window's title: the page's, or the open room's.
    private var windowTitle: String {
        switch workspace.page {
        case .inbox: return "Inbox"
        case .team: return "Team"
        case .friends: return "Friends"
        case nil: return store.selectedRoom?.name ?? "Cowchat"
        }
    }
}

private struct SidebarView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @Binding var isSidebarVisible: Bool
    @Binding var isSettingsPresented: Bool
    @State private var isArchiveExpanded = false
    @State private var isRoomBrowserPresented = false
    /// What a legacy scroller takes from the room lists' right edge, which the pinned Inbox and Home rows give up too so
    /// they end where Team's row ends.
    @State private var listsScrollerWidth: CGFloat = 0
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
            // One gap of 10 under the search field puts the first row's icon 20 under it, the distance between the
            // icons of the rows stacked below.
            searchField
                .padding(.horizontal, 12)
                .padding(.bottom, 10)

            // Find a room, the Inbox and Home stack at their own 38, with their icons and titles in one column.
            // copy:
            SidebarNavRow(title: "Find a room", icon: .search, fallbackSystemName: "magnifyingglass",
                          accessibilityLabel: "Find a room", isSelected: false) {
                isRoomBrowserPresented = true
            }
            .padding(.horizontal, 12)
            .padding(.trailing, listsScrollerWidth)
            .sheet(isPresented: $isRoomBrowserPresented) {
                RoomBrowserView(workspace: workspace)
            }

            InboxNavRow(inbox: workspace.inbox)
                .padding(.trailing, listsScrollerWidth)

            HomeNavRow(account: workspace.accountGlobal ?? workspace.local, home: workspace.homeSessions)
                .padding(.trailing, listsScrollerWidth)

            SidebarRoomLists(
                local: workspace.local,
                globalOrLocal: workspace.global ?? workspace.local,
                accountOrLocal: workspace.accountGlobal ?? workspace.local,
                home: workspace.homeSessions,
                hasAccountGlobal: workspace.accountGlobal != nil,
                hasGlobal: workspace.global != nil
            )
            .onPreferenceChange(SidebarScrollerWidthKey.self) { listsScrollerWidth = $0 }

            // Archive stays pinned above the footer instead of trailing the
            // room list (which floats it mid-sidebar when the list is short).
            ArchiveSection(
                local: workspace.local,
                globalOrLocal: workspace.global ?? workspace.local,
                accountOrLocal: workspace.accountGlobal ?? workspace.local,
                hasAccountGlobal: workspace.accountGlobal != nil,
                hasGlobal: workspace.global != nil,
                isExpanded: $isArchiveExpanded
            )
            .padding(.horizontal, 8)

            SidebarFooter(
                local: workspace.local,
                globalOrLocal: workspace.global ?? workspace.local,
                accountOrLocal: workspace.accountGlobal ?? workspace.local,
                hasAccountGlobal: workspace.accountGlobal != nil,
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
        SearchCapsule(placeholder: "Search rooms or messages", text: $workspace.searchText,
                      isFocused: $isSearchFocused, clearLabel: "Clear room search")
    }

}

/// A destination above the room lists: Dash's sunrise at 18, or the
/// destination's own glyph, 10 before the title in Body S Strong, 10 at the
/// sides, 38 tall, on the sidebar's row background. Find a room, the Inbox,
/// Home and Team draw it. A count sits at the row's end in Caption, Text/Tertiary,
/// as the Archive row's does, and nothing at zero.
private struct SidebarNavRow: View {
    let title: String
    var icon = GallopIcon.sunrise
    var fallbackSystemName = "sunrise"
    var count = 0
    /// The row's accessible name, "Open <title>" unless it says otherwise.
    var accessibilityLabel: String?
    let isSelected: Bool
    let action: () -> Void
    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                GallopIconView(icon: icon, fallbackSystemName: fallbackSystemName, size: 18)
                    .foregroundStyle(
                        isSelected
                            ? SemanticColor.surfaceGlassOnIconDefault
                            : SemanticColor.iconSecondary
                    )
                Text(title)
                    .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
                Spacer()
                if count > 0 {
                    Text("\(count)")
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                }
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
            label: accessibilityLabel ?? "Open \(title)",
            value: accessibilityValue,
            action: action
        )
    }

    // copy:
    private var accessibilityValue: String? {
        let parts = [count > 0 ? "\(count) unread" : nil, isSelected ? "selected" : nil].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }
}

/// The Inbox above Home, pinned as Home's row is, once the person is signed in. Its count is every unread item, open
/// invitations included, and it shows nothing at zero.
private struct InboxNavRow: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var inbox: InboxStore

    var body: some View {
        if workspace.accountGlobal != nil {
            // copy:
            SidebarNavRow(title: "Inbox", icon: .incoming, fallbackSystemName: "tray.and.arrow.down",
                          count: inbox.unreadCount, isSelected: workspace.page == .inbox) {
                workspace.show(.inbox)
            }
            .padding(.horizontal, 12)
        }
    }
}

/// Team under Home's sessions, at the top of the room lists' scroll view, drawn as Home's row is.
private struct TeamNavRow: View {
    @EnvironmentObject private var workspace: WorkspaceStore

    var body: some View {
        if workspace.accountGlobal != nil {
            // copy:
            SidebarNavRow(title: "Team", icon: .posse, fallbackSystemName: "person.2",
                          isSelected: workspace.page == .team) {
                workspace.show(.team)
            }
            // The scroll view's lists sit 8 in, so 4 more puts the row 12 in, as Home's is.
            .padding(.horizontal, 4)
            .padding(.bottom, 10)
        }
    }
}

/// Home, pinned above the room lists' scroll view under the Inbox, so it
/// never scrolls away. Its live sessions scroll
/// under it at the top of the lists (`HomePlaces`).
private struct HomeNavRow: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var account: ChatStore
    @ObservedObject var home: HomeSessions

    var body: some View {
        if workspace.accountGlobal != nil,
           let room = account.rooms.first(where: { $0.id == home.roomID }) {
            // copy:
            SidebarNavRow(title: "Home", isSelected: workspace.isSelected(room, on: .accountGlobal)) {
                Task { await workspace.select(room: room, on: .accountGlobal) }
            }
            // The nav rows sit 12 in, and Home's sessions 2 under it.
            .padding(.horizontal, 12)
            .padding(.bottom, 2)
        }
    }
}

/// The person's live sessions under Home's pinned row: at most five
/// (`HomeSessions.picked`), then a row for the rest. They sit at the top of
/// the room lists' scroll view, so however many sessions are live the rooms
/// stay in reach.
private struct HomePlaces: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var account: ChatStore
    @ObservedObject var home: HomeSessions

    var body: some View {
        if workspace.accountGlobal != nil, account.rooms.contains(where: { $0.id == home.roomID }) {
            VStack(spacing: 2) {
                ForEach(home.shown) { session in
                    SessionRow(home: home, session: session)
                        .sidebarListItem("session/\(session.id)")
                }
                if home.hiddenCount > 0 {
                    // The session names start after the harness mark's 16 and the 8 beside it.
                    SessionsMoreRow(title: home.moreTitle, indent: SessionRow.indent + 16 + 8) {
                        home.showsAll.toggle()
                    }
                    .sidebarListItem("sessions-more")
                }
            }
            // The lists sit 8 in; the nav rows sit 12 in.
            .padding(.horizontal, 4)
            // Home's row and the first list keep the 10 between them they had.
            .padding(.bottom, home.shown.isEmpty ? 8 : 10)
        }
    }
}

/// One of the person's live sessions under Home: its harness mark lined up
/// with Home's title, its name in Body S and its second line in Caption, and
/// the turning mark at the row's end while it works. Pressing it opens the
/// room made for the session, or Home with the session mentioned in the
/// composer. It never draws selected; the room it opens shows the selection.
private struct SessionRow: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var home: HomeSessions
    let session: LiveSession
    @State private var isHovering = false

    /// Home's title starts 38 in from the row's edge: 10 of padding, the 18
    /// icon and 10 between them.
    static let indent: CGFloat = 38

    var body: some View {
        Button {
            Task { await workspace.openSession(session) }
        } label: {
            SessionRowLabel(home: home, session: session, line: home.line(for: session), showsWork: true)
                .padding(.leading, Self.indent)
                .padding(.trailing, 6)
                .frame(height: 40)
                .background(SidebarRowBackground(state: .init(isSelected: false, isHovering: isHovering)))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
        .macAccessibleAction(label: "Open \(home.name(of: session))", value: accessibilityValue) {
            Task { await workspace.openSession(session) }
        }
    }

    // copy:
    private var accessibilityValue: String? {
        switch session.state {
        case .working: return "Working"
        case .prompt: return home.line(for: session)
        case .idle: return nil
        }
    }
}

/// A session's mark, name and second line, as Home's rows and the @
/// completion draw them: the harness mark at 16, 8 before the name in Body S,
/// the line under it in Caption (Text/Tertiary, or the warning colour while
/// the session waits at a prompt), and with `showsWork`, a slot at the end as
/// wide as an XS button holding the turning mark while it works.
private struct SessionRowLabel: View {
    @ObservedObject var home: HomeSessions
    let session: LiveSession
    let line: String
    let showsWork: Bool

    var body: some View {
        HStack(spacing: 8) {
            HarnessMark(harness: session.harness)
                .help(session.harness.appName)
            VStack(alignment: .leading, spacing: 0) {
                Text(home.name(of: session))
                    .gallopText(.bodyS, color: SemanticColor.textPrimary)
                    .lineLimit(1)
                Text(line)
                    .gallopText(
                        .caption,
                        color: session.state == .prompt && showsWork
                            ? SemanticColor.warning : SemanticColor.textTertiary
                    )
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 0)
            if showsWork {
                Group {
                    if session.state == .working { SessionWorkingMark() }
                }
                .frame(width: 28, height: 16)
            }
        }
    }
}

/// The harness a session runs in, as a 16-point rounded square: Claude Code's
/// asterisk on its clay, or Codex's code glyph on near-black. Gallop has no
/// tokens for other products' marks, so their colours are written here. In
/// dark mode Codex's mark turns light so it keeps 3:1 on every dark surface.
struct HarnessMark: View {
    let harness: LiveSession.Harness
    var size: CGFloat = 16

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
            .fill(background)
            .overlay {
                Image(systemName: harness == .claudeCode ? "asterisk" : "chevron.left.forwardslash.chevron.right")
                    .font(.system(size: size * 0.56, weight: .bold))
                    .foregroundStyle(glyph)
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }

    private var background: Color {
        switch harness {
        case .claudeCode: return HexColor.adaptive(light: "#D97757", dark: "#D97757")
        case .codex: return HexColor.adaptive(light: "#1D1916", dark: "#AEA399")
        }
    }

    private var glyph: Color {
        switch harness {
        case .claudeCode: return .white
        case .codex: return HexColor.adaptive(light: "#FFFFFF", dark: "#1D1916")
        }
    }
}

/// A working session's mark: Dash Chat's live-task glyph, which a room row
/// draws while its agents work, turning once a second as Gallop's spinner
/// does. At 20 its arc has the 10-point spinner's radius and stroke, in
/// Icon/Tertiary so it stays quiet beside the names.
struct SessionWorkingMark: View {
    @State private var turning = false

    var body: some View {
        GallopIconView(icon: .thinking, fallbackSystemName: "circle.dotted", size: 20)
            .foregroundStyle(SemanticColor.iconTertiary)
            .frame(width: 10, height: 10)
            .rotationEffect(.degrees(turning ? 360 : 0))
            .animation(.linear(duration: 1).repeatForever(autoreverses: false), value: turning)
            .onAppear { turning = true }
            .accessibilityHidden(true)
    }
}

/// The row after the first five sessions when more are live: how many more in Body S, Text/Secondary at the names'
/// indent with no icon, 32 tall, with the row hover of the surface it sits on. Pressing it shows every row, and it then
/// says how many it hides again. Home's sessions in the sidebar and each person on the Team page draw it.
struct SessionsMoreRow: View {
    enum Surface { case sidebar, page }

    let title: String
    let indent: CGFloat
    var surface = Surface.sidebar
    let action: () -> Void
    @State private var isHovering = false

    var body: some View {
        Group {
            switch surface {
            case .sidebar:
                Button(action: action) {
                    label.background(SidebarRowBackground(state: .init(isSelected: false, isHovering: isHovering)))
                }
                .buttonStyle(.plain)
                .onHover { isHovering = $0 }
            case .page:
                Button(action: action) { label }
                    .buttonStyle(PageRowButtonStyle())
            }
        }
        .macAccessibleAction(label: title, action: action)
    }

    private var label: some View {
        Text(title)
            .gallopText(.bodyS, color: SemanticColor.textSecondary)
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, indent)
            .padding(.trailing, 6)
            .frame(height: 32)
            .contentShape(Rectangle())
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
                isSlackConnected: store.slackConnectedRoomIDs.contains(room.id),
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
            store.slackConnectedRoomIDs.contains(room.id) ? "Connected to Slack" : nil,
            workspace.isSelected(room, on: server) ? "selected" : nil,
        ].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }
}

/// Where each row of the sidebar's lists sat in the scroll view when it last laid out, and the scroll view's height.
/// A reference, so writing it while the lists scroll redraws nothing.
private final class SidebarListLayout {
    var frames: [String: CGRect] = [:]
    var height: CGFloat = 0
}

/// What a legacy scroller takes from the room lists' width: the lists' width less their content's, or none.
private struct SidebarScrollerWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

/// The room lists' content width, measured inside their scroll view.
private struct SidebarListsContentWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

private struct SidebarListFramesKey: PreferenceKey {
    static let defaultValue: [String: CGRect] = [:]

    static func reduce(value: inout [String: CGRect], nextValue: () -> [String: CGRect]) {
        value.merge(nextValue()) { $1 }
    }
}

private extension View {
    /// A row of the sidebar's lists: scrolled to by `id`, and kept whole at the lists' top edge when it is.
    func sidebarListItem(_ id: String) -> some View {
        self.id(id).background {
            GeometryReader { row in
                Color.clear.preference(key: SidebarListFramesKey.self,
                                       value: [id: row.frame(in: .named(SidebarRoomLists.coordinateSpace))])
            }
        }
    }
}

private struct SidebarRoomLists: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var local: ChatStore
    @ObservedObject var globalOrLocal: ChatStore
    @ObservedObject var accountOrLocal: ChatStore
    @ObservedObject var home: HomeSessions
    let hasAccountGlobal: Bool
    let hasGlobal: Bool
    /// Where the rows sat when the lists last laid out, kept apart from state so scrolling redraws nothing.
    @State private var layout = SidebarListLayout()
    /// Room at the lists' end, so a row can scroll to the top edge when the end is near.
    @State private var endRoom: CGFloat = 0
    /// The lists' width and their content's, whose difference is what a legacy scroller takes.
    @State private var listsWidth: CGFloat = 0
    @State private var contentWidth: CGFloat = 0

    var body: some View {
        TimelineView(.periodic(from: .now, by: anyAgentsThinking ? 10 : 60)) { timeline in
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        HomePlaces(account: accountOrLocal, home: home)
                        TeamNavRow()
                        if visibleRoomsEmpty && isSearchActive {
                            emptyRoomsState
                        } else {
                            section(for: .local, store: local, at: timeline.date)
                            if hasGlobal || hasAccountGlobal {
                                sectionHeader("Global rooms")
                            }
                            if hasGlobal {
                                section(for: .global, store: globalOrLocal, at: timeline.date)
                            }
                            if hasAccountGlobal {
                                section(for: .accountGlobal, store: accountOrLocal, at: timeline.date)
                            }
                        }
                        // The lists' end, with any room added so a row can reach the top edge.
                        Color.clear
                            .frame(height: endRoom)
                            .sidebarListItem(Self.endID)
                    }
                    .padding(.horizontal, 8)
                    .padding(.bottom, Self.endPadding)
                    .background {
                        GeometryReader { content in
                            Color.clear.preference(key: SidebarListsContentWidthKey.self, value: content.size.width)
                        }
                    }
                }
                .coordinateSpace(name: Self.coordinateSpace)
                .background {
                    GeometryReader { viewport in
                        Color.clear
                            .onAppear {
                                layout.height = viewport.size.height
                                listsWidth = viewport.size.width
                            }
                            .onChange(of: viewport.size.height) { layout.height = $0 }
                            .onChange(of: viewport.size.width) { listsWidth = $0 }
                    }
                }
                .onPreferenceChange(SidebarListFramesKey.self) { layout.frames = $0 }
                .onPreferenceChange(SidebarListsContentWidthKey.self) { contentWidth = $0 }
                .preference(key: SidebarScrollerWidthKey.self,
                            value: contentWidth > 0 ? max(0, listsWidth - contentWidth) : 0)
                .scrollIndicators(.hidden)
                // Home's sessions sit above the rooms, so the open room's row is scrolled into view when it changes,
                // at once, as a native list reveals its selection.
                .onChange(of: selectedRowID) { id in
                    if let id { reveal(id, with: proxy) }
                }
            }
        }
    }

    static let coordinateSpace = "sidebarLists"
    private static let endID = "lists-end"
    private static let endPadding: CGFloat = 12

    /// Scrolls a row into view as little as it can, as `scrollTo` does, unless it is in view already. A row left cut
    /// at the top edge then scrolls away to the next row's top, with room added at the lists' end when the end is too
    /// near to scroll that far.
    private func reveal(_ id: String, with proxy: ScrollViewProxy) {
        if let frame = layout.frames[id], frame.minY >= -0.5, frame.maxY <= layout.height + 0.5 { return }
        proxy.scrollTo(id)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            // A room with no row here, such as Home, leaves the lists as they are.
            guard layout.frames[id] != nil else { return }
            let rows = layout.frames.filter { $0.key != Self.endID }.sorted { $0.value.minY < $1.value.minY }
            guard let cut = rows.first(where: { $0.value.minY < -0.5 && $0.value.maxY > 0.5 }),
                  let next = rows.first(where: { $0.value.minY >= cut.value.maxY - 0.5 }) else { return }
            let distance = next.value.minY
            let remaining = layout.frames[Self.endID].map { $0.maxY + Self.endPadding - layout.height } ?? .infinity
            if distance > remaining + 0.5 {
                endRoom += distance - remaining
                DispatchQueue.main.async { proxy.scrollTo(next.key, anchor: .top) }
            } else {
                proxy.scrollTo(next.key, anchor: .top)
            }
        }
    }

    /// The open room's row, as `rowID` names it.
    private var selectedRowID: String? {
        let server = workspace.activeServer
        return workspace.store(for: server)?.selectedRoomID.map { Self.rowID($0, on: server) }
    }

    /// A room row's ID for scrolling to it. Room IDs are each server's own, so the server is part of it.
    static func rowID(_ roomID: String, on server: WorkspaceStore.Server) -> String { "\(server.rawValue)/\(roomID)" }

    private func baseRooms(in store: ChatStore, server: WorkspaceStore.Server) -> [Room] {
        let searching = !workspace.searchText
            .trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let source = RoomSidebarPresentation.listedRooms(store.unarchivedRooms, on: server, searching: searching,
                                                         homeRoomID: home.roomID)
        return RoomSidebarPresentation.filteredRooms(
            from: source,
            query: workspace.searchText,
            matchingMessageRoomIDs: store.messageSearchRoomIDs
        )
    }

    private var anyAgentsThinking: Bool {
        !local.lastThinkingAt.isEmpty
            || (hasGlobal && !globalOrLocal.lastThinkingAt.isEmpty)
            || (hasAccountGlobal && !accountOrLocal.lastThinkingAt.isEmpty)
    }

    private var visibleRoomsEmpty: Bool {
        baseRooms(in: local, server: .local).isEmpty
            && (!hasGlobal || baseRooms(in: globalOrLocal, server: .global).isEmpty)
            && (!hasAccountGlobal || baseRooms(in: accountOrLocal, server: .accountGlobal).isEmpty)
    }

    /// Item D (quiet empty state): the sidebar only shows an explicit empty
    /// message while a search is actually in flight or has text typed. A
    /// genuinely empty room list (no search) renders nothing above Archive.
    private var isSearchActive: Bool {
        isSearchingMessages || !workspace.searchText.isEmpty
    }

    private var isSearchingMessages: Bool {
        local.isSearchingMessages || (hasGlobal && globalOrLocal.isSearchingMessages)
            || (hasAccountGlobal && accountOrLocal.isSearchingMessages)
    }

    @ViewBuilder
    private func section(
        for server: WorkspaceStore.Server,
        store: ChatStore,
        at now: Date
    ) -> some View {
        let rooms = baseRooms(in: store, server: server)
        if server == .local && (hasGlobal || hasAccountGlobal) {
            sectionHeader("Local rooms")
        }
        if !rooms.isEmpty {
            VStack(spacing: 0) {
                // Ordered and timed by the newest message each room's feed shows.
                ForEach(RoomSidebarPresentation.sortedByRecency(rooms.map(store.roomAsShown))) { room in
                    SidebarRoomRowButton(store: store, room: room, server: server, now: now)
                        .sidebarListItem(Self.rowID(room.id, on: server))
                }
            }
            .padding(.bottom, 10)
        }
    }

    private func sectionHeader(_ title: String) -> some View {
        HStack(spacing: 6) {
            Text(title)
                .gallopText(.caption, color: SemanticColor.textTertiary)
            Spacer()
        }
        .padding(.leading, 16)
        .padding(.trailing, 10)
        .padding(.top, 6)
        .padding(.bottom, 4)
        .sidebarListItem("header/\(title)")
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
    @ObservedObject var accountOrLocal: ChatStore
    let hasAccountGlobal: Bool
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

    private struct Entry: Identifiable {
        let server: WorkspaceStore.Server
        let store: ChatStore
        let room: Room
        var id: String { server.rawValue + "." + room.id }
    }

    private var entries: [Entry] {
        var all = archivedRooms(in: local).map { Entry(server: .local, store: local, room: $0) }
        if hasGlobal {
            all += archivedRooms(in: globalOrLocal).map { Entry(server: .global, store: globalOrLocal, room: $0) }
        }
        if hasAccountGlobal {
            all += archivedRooms(in: accountOrLocal).map { Entry(server: .accountGlobal, store: accountOrLocal, room: $0) }
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
                            ForEach(entries) { entry in
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
    @ObservedObject var accountOrLocal: ChatStore
    let hasAccountGlobal: Bool
    let hasGlobal: Bool
    @Binding var isSettingsPresented: Bool
    @EnvironmentObject private var workspace: WorkspaceStore

    var body: some View {
        HStack(spacing: 8) {
            TeamFooterLead(teams: workspace.teams) {
                Button {
                    isSettingsPresented = true
                } label: {
                    HStack(spacing: 6) {
                        Circle()
                            .fill(connectionColor)
                            .frame(width: 7, height: 7)
                        Text(connectionLabel)
                            .gallopText(.caption, color: SemanticColor.textSecondary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(connectionDetails)
                .macAccessibleAction(label: "Connection settings") { isSettingsPresented = true }
            }
            .task(id: workspace.social.savedSession?.scopeId) {
                await workspace.teams.load(for: workspace.social.savedSession)
            }
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

    private var connectionLabel: String {
        if !anyServerDisconnected { return "Connected" }
        if statuses.contains(where: { if case .connecting = $0 { return true }; return false }) {
            return "Connecting…"
        }
        return "Connection issue"
    }

    private var connectionColor: Color {
        if !anyServerDisconnected { return SemanticColor.success }
        if statuses.contains(where: { if case .connecting = $0 { return true }; return false }) {
            return SemanticColor.warning
        }
        return SemanticColor.textError
    }

    private var connectionDetails: String {
        var details = ["Local: \(local.connectionStatus.label)"]
        if hasGlobal { details.append("Global rooms (key): \(globalOrLocal.connectionStatus.label)") }
        if hasAccountGlobal { details.append("Global rooms (account): \(accountOrLocal.connectionStatus.label)") }
        return details.joined(separator: " · ")
    }

    private var statuses: [ConnectionStatus] {
        [local.connectionStatus]
            + (hasGlobal ? [globalOrLocal.connectionStatus] : [])
            + (hasAccountGlobal ? [accountOrLocal.connectionStatus] : [])
    }

    private var anyServerDisconnected: Bool {
        !local.connectionStatus.isConnected
            || (hasGlobal && !globalOrLocal.connectionStatus.isConnected)
            || (hasAccountGlobal && !accountOrLocal.connectionStatus.isConnected)
    }

    private func reconnectDisconnectedServers() {
        var stores = [local]
        if hasGlobal { stores.append(globalOrLocal) }
        if hasAccountGlobal { stores.append(accountOrLocal) }
        for store in stores where !store.connectionStatus.isConnected {
            store.reconnect()
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
    let isSlackConnected: Bool
    let isSelected: Bool
    let isUnread: Bool
    let isWorking: Bool
    let now: Date
    @State private var isHovering = false

    var body: some View {
        HStack(spacing: 10) {
            RoomAvatar(name: room.name, size: 40, accented: isSelected)
                .overlay(alignment: .bottomTrailing) {
                    if isSlackConnected { SlackRoomBadge().offset(x: 3, y: 3) }
                }
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
            Text("Now connect your agent to this room. Copy the connect prompt from the ⋯ menu.")
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
    @EnvironmentObject private var workspace: WorkspaceStore
    @EnvironmentObject private var home: HomeSessions
    let room: Room
    @Binding var isSidebarVisible: Bool
    /// Bumped to focus the composer with the cursor after the draft.
    @State private var composerFocusRequest = 0
    @State private var mentionHighlight = 0
    /// The draft the @ completion was closed on with Escape. It stays closed until the draft changes.
    @State private var mentionClosedDraft: String?
    @State private var isRoomKeyPresented = false
    @State private var roomKeyDraft = ""
    @State private var roomKeyError: String?
    @State private var isComposerExpanded = false
    @State private var isFieldHovering = false
    @State private var isDestroyConfirmationPresented = false
    @State private var isDestroyingRoom = false
    @State private var isInvitesPresented = false
    @State private var isSlackPresented = false
    @State private var isLeasesPresented = false
    @State private var isArtifactPresented = false
    @State private var isMessageListNearBottom = true
    @State private var newMessageCount = 0
    @State private var hasCopiedQuietRoomPrompt = false
    /// The list is revealed only after the initial bottom-anchor scroll, so a
    /// room switch never paints top-anchored content and animates it away.
    @State private var hasPositionedInitialScroll = false
    /// The runs of hidden agent messages opened in place, by their first message.
    @State private var openRuns: Set<String> = []

    private var parentRoom: Room? {
        guard let parentID = room.parentID else { return nil }
        return store.rooms.first { $0.id == parentID }
    }

    /// Whether this is the account's Home, which the sidebar and the header call Home whatever its room name.
    private var isHome: Bool {
        workspace.activeServer == .accountGlobal && room.id == home.roomID
    }

    // copy:
    private var title: String { isHome ? "Home" : room.name }

    /// Every session in Home is its person's, so Home names a session's messages by its name alone, without the handle
    /// the server adds. Other rooms keep the server's name.
    private func senderName(of message: ChatMessage) -> String? {
        guard isHome, let session = home.sessions.first(where: { $0.id == message.agentID }) else { return nil }
        return home.name(of: session)
    }

    /// The @names this room's messages and composer draw in the accent: the live sessions of the room's owner when
    /// that is the viewer, since the app knows only the viewer's sessions, and the people the app knows.
    private var mentionMarks: MentionMarks {
        let social = workspace.social
        let owner = isHome ? social.account?.id : social.rooms.first { $0.roomId == room.id }?.ownerId
        let ownsRoom = workspace.activeServer == .accountGlobal && owner != nil && owner == social.account?.id
        return MentionMarks(sessionNames: ownsRoom ? home.sessions.map(home.name(of:)) : [],
                            people: social.friends.map(\.account), viewer: social.account)
    }

    /// The live sessions the @ being typed in Home's draft can name, or none while no @ is being typed.
    private var mentionOptions: [LiveSession] {
        guard isHome, isComposerExpanded, store.draft != mentionClosedDraft,
              let query = HomeSessions.mentionQuery(in: store.draft) else { return [] }
        return home.options(matching: query)
    }

    private func paneState(at now: Date) -> RoomPaneState {
        RoomPaneState.state(
            connectionStatus: store.connectionStatus,
            isLoadingMessages: store.isLoadingMessages,
            hasMessages: !store.messages.isEmpty,
            hasOtherMembers: hasCollaborators(at: now)
        )
    }

    private func hasCollaborators(at now: Date) -> Bool {
        ChatPresencePresentation.hasCollaboratorSignal(
            members: store.roomMembers,
            currentAgentID: store.agentID,
            fallbackMemberCount: room.memberCount,
            fallbackMemberCountIncludesCurrentAgent:
                store.fallbackMemberCountIncludesCurrentAgent(in: room.id),
            recentActivityByAgent: store.recentAgentActivityAt[room.id],
            now: now
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            chatHeader

            ZStack(alignment: .bottomTrailing) {
                messageList
                    .environment(\.mentionMarks, mentionMarks)
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
                        if RoomPaneState.showsInviteNudge(
                            state: state,
                            hasOtherMembers: hasCollaborators(at: timeline.date)
                        ) {
                            InviteCollaboratorNudge {
                                await store.copyableConnectPrompt(for: room)
                            }
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                            .padding(.top, 12)
                        }
                    }
                }
                composer
            }
        }
        .background(SemanticColor.surface500)
        // The request is current when this view appears with the room, so the subscription's first value covers
        // opening Home from another room.
        .onReceive(store.$composerRequestRoomID.receive(on: RunLoop.main)) { takeComposerRequest($0) }
        .sheet(isPresented: $isRoomKeyPresented) {
            VStack(alignment: .leading, spacing: 16) {
                Text("Room key for \(room.name)").font(.headline)
                Text("Enter the shared room key. It stays in this Mac’s Keychain.").font(.callout)
                CapsuleField(placeholder: "Room key", text: $roomKeyDraft, isSecure: true)
                if let error = roomKeyError { Text(error).foregroundStyle(.red).font(.caption) }
                HStack {
                    if store.unlockedRoomIDs.contains(room.id) {
                        Button("Forget key", role: .destructive) {
                            do { try store.forgetRoomSecret(for: room); isRoomKeyPresented = false }
                            catch { roomKeyError = error.localizedDescription }
                        }
                    }
                    Spacer()
                    Button("Cancel") { isRoomKeyPresented = false }
                    Button("Save key") {
                        do { try store.saveRoomSecret(roomKeyDraft, for: room); isRoomKeyPresented = false }
                        catch { roomKeyError = "Could not save the key. Check that it matches this room and Keychain is available." }
                    }.disabled(roomKeyDraft.isEmpty).keyboardShortcut(.defaultAction)
                }
            }
            .padding(24).frame(width: 420)
            .onDisappear { roomKeyDraft = ""; roomKeyError = nil }
        }
        .sheet(isPresented: $isLeasesPresented) {
            RoomLeasesView(room: room).environmentObject(store)
        }
        .sheet(isPresented: $isArtifactPresented) {
            RoomArtifactView(room: room).environmentObject(store)
        }
        .sheet(isPresented: $isSlackPresented) {
            SlackRoomView(room: room).environmentObject(store)
        }
        .sheet(isPresented: $isInvitesPresented) {
            RoomInvitesView(room: room)
                .environmentObject(store)
        }
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
                    // copy:
                    Toggle("Show all messages", isOn: Binding(
                        get: { store.showsAllMessages(in: room.id) },
                        set: { store.setShowsAllMessages($0, in: room.id) }
                    ))
                    Divider()
                    Button("Leases…") { isLeasesPresented = true }
                    Button("Copy connect prompt") { copyConnectPrompt() }
                    if !store.isLocalConnection {
                        Button("Artifact…") { isArtifactPresented = true }
                        if RoomPeople.isOffered(isLocal: store.isLocalConnection, apiKey: store.connectionProfile.apiKey) {
                            // copy:
                            Button("People…") { Task { await workspace.showPeople(of: room.id) } }
                        }
                        if !store.connectionProfile.apiKey.hasPrefix("csoc_") {
                            Button("Invites…") { isInvitesPresented = true }
                            Button("Slack…") { isSlackPresented = true }
                        }
                    }
                    Divider()
                    // Rename and destroy are creator-only, so for other
                    // agents' rooms they are hidden rather than left as
                    // permanently disabled clutter.
                    if store.canRename(room) {
                        Button("Rename room") { store.presentRename(room) }
                    }
                    Button("Archive room") { Task { await store.archive(room) } }
                    if !store.connectionProfile.apiKey.hasPrefix("csoc_") {
                        Button("Create nested room…") { store.presentCreateRoom(parentID: room.id) }
                    }
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
                    Text(title)
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    if room.encrypted {
                        GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 12)
                            .foregroundStyle(SemanticColor.iconTertiary)
                    }
                }
                TimelineView(.periodic(from: .now, by: 10)) { timeline in
                    let summary = presenceSummary(at: timeline.date)
                    HStack(spacing: 0) {
                        Text(summary)
                            .gallopText(
                                .caption,
                                color: summary.contains("active")
                                    ? SemanticColor.warning : SemanticColor.textTertiary
                            )
                    }
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

    private func openAttachment(_ attachment: FileAttachment) {
        Task {
            do {
                try await store.openAttachment(attachment)
            } catch {
                store.errorMessage = error.localizedDescription
            }
        }
    }

    private func toggleRun(_ run: HiddenMessageRun) {
        withAnimation(.easeInOut(duration: 0.16)) { openRuns.formSymmetricDifference([run.id]) }
    }

    private func saveAttachment(_ attachment: FileAttachment) {
        Task {
            do {
                let saved = try await store.downloadAttachment(attachment)
                NSWorkspace.shared.activateFileViewerSelecting([saved])
            } catch {
                store.errorMessage = error.localizedDescription
            }
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
            connectionStatus: store.connectionStatus,
            // Every session in Home is its person's, so Home names them without the handle the server adds.
            name: isHome ? home.headerName(of:) : \.name,
            nameSeparator: isHome ? HomeSessions.headerNameSeparator : " · "
        )
    }

    /// The feed's padding under its last message while the composer is open, which keeps the message above the field.
    private static let openComposerClearance: CGFloat = 86

    private var messageList: some View {
        TimelineView(.periodic(from: .now, by: 60)) { timeline in
            ScrollViewReader { proxy in
                GeometryReader { viewport in
                ZStack(alignment: .bottom) {
                    ScrollView {
                        // A plain VStack on purpose. LazyVStack's prefetch
                        // cache never converges when variable-height rows are
                        // appended while the view is scrolled up: each pass
                        // shifts the height estimates, signalPrefetch requests
                        // another update, and the main thread livelocks at
                        // 100% CPU re-laying-out the feed (ANRs of 2026-08-06
                        // and 2026-08-19, both sample-confirmed in
                        // LazyLayoutViewCache.signalPrefetch). History is
                        // bounded, so eager layout converges in one pass.
                        VStack(alignment: .leading, spacing: 22) {

                        let pulseAnchors = ThinkingPulsePresentation.anchors(
                            messages: store.messages,
                            pulses: store.thinkingPulses[room.id, default: [:]],
                            currentAgentID: store.agentID,
                            now: timeline.date
                        )

                        let feed = MessageFeed.items(
                            store.messages,
                            showsAll: store.showsAllMessages(in: room.id),
                            openRuns: openRuns
                        )

                        ForEach(feed) { item in
                            switch item {
                            case .message(let message, let quiet):
                                VStack(alignment: .leading, spacing: 10) {
                                    MessageFeedRow(
                                        message: store.displayMessage(message),
                                        isMine: message.agentID == store.agentID,
                                        now: timeline.date,
                                        quiet: quiet,
                                        senderName: senderName(of: message),
                                        onSaveAttachment: room.encrypted ? nil : { saveAttachment($0) },
                                        onOpenAttachment: room.encrypted ? nil : { openAttachment($0) }
                                    )
                                    ForEach(
                                        pulseAnchors.byMessageID[message.id] ?? [],
                                        id: \.agentID
                                    ) { pulse in
                                        ThinkingPulsePill(pulse: pulse)
                                    }
                                }
                                .id(message.id)
                            case .run(let run):
                                // A closed run keeps the pulses of the agents inside it under its row.
                                VStack(alignment: .leading, spacing: 10) {
                                    HiddenRunRow(run: run) { toggleRun(run) }
                                    if !run.isOpen {
                                        ForEach(
                                            run.messages.flatMap { pulseAnchors.byMessageID[$0.id] ?? [] },
                                            id: \.agentID
                                        ) { pulse in
                                            ThinkingPulsePill(pulse: pulse)
                                        }
                                    }
                                }
                                .id(item.id)
                            }
                        }

                        // Pulsing agents who haven't spoken yet pill at the
                        // end of the feed rather than vanishing.
                        ForEach(pulseAnchors.unanchored, id: \.agentID) { pulse in
                            ThinkingPulsePill(pulse: pulse)
                        }

                            // Non-lazy containers fire onAppear on insertion,
                            // not on scroll — near-bottom detection reads the
                            // marker's position in the scroll viewport
                            // instead. The states it drives only affect the
                            // pill OVERLAY, never the feed's own layout, so
                            // the preference loop settles in one pass.
                            Color.clear
                                .frame(height: 1)
                                .id("message-list-bottom")
                                .background(
                                    GeometryReader { marker in
                                        Color.clear.preference(
                                            key: FeedBottomOffsetKey.self,
                                            value: marker.frame(in: .named("cowchatFeed")).minY
                                        )
                                    }
                                )
                        }
                        .padding(.horizontal, 20)
                        .padding(.top, 18)
                        .padding(.bottom, isComposerExpanded ? Self.openComposerClearance : 72)
                    }
                    .coordinateSpace(name: "cowchatFeed")
                    .onPreferenceChange(FeedBottomOffsetKey.self) { markerY in
                        let nearBottom = markerY <= viewport.size.height + 60
                        if nearBottom {
                            if !isMessageListNearBottom { isMessageListNearBottom = true }
                            if newMessageCount != 0 { newMessageCount = 0 }
                        } else if isMessageListNearBottom {
                            isMessageListNearBottom = false
                        }
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
                    } else if let latest = store.messages.last, store.showsInFeed(latest) {
                        // A hidden arrival only joins a fold, so there is nothing new to read.
                        newMessageCount += 1
                    }
                }
                // The open composer covers the feed's last lines, so a feed at its end moves up to show the newest
                // message whole above the field, as far as the feed's padding under it. It moves once the composer
                // has opened and without animating, since an animated scroll here is lost to the field taking focus.
                .onChange(of: isComposerExpanded) { expanded in
                    guard expanded, isMessageListNearBottom, hasPositionedInitialScroll,
                          viewport.size.height > 0 else { return }
                    let clearance = UnitPoint(x: 0.5, y: max(0, 1 - Self.openComposerClearance / viewport.size.height))
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                        proxy.scrollTo("message-list-bottom", anchor: clearance)
                    }
                }
                // The switch adds or takes away messages all through the feed, so a feed at its end stays there.
                .onChange(of: store.showsAllMessages(in: room.id)) { _ in
                    guard isMessageListNearBottom, hasPositionedInitialScroll else { return }
                    DispatchQueue.main.async {
                        proxy.scrollTo("message-list-bottom", anchor: .bottom)
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
    }

    /// Whether the room's quiet state offers the connect prompt: not in Home while one of the person's sessions is live
    /// there, since an agent is already in.
    private var offersConnectPrompt: Bool {
        RoomPaneState.quietOffersConnectPrompt(isHome: isHome, liveSessions: home.sessions.count)
    }

    private var quietRoom: some View {
        VStack(spacing: 10) {
            GallopIconView(icon: .message, fallbackSystemName: "bubble.left", size: 24)
                .foregroundStyle(SemanticColor.iconTertiary)
            Text("This room is quiet")
                .gallopText(.h5, color: SemanticColor.textPrimary)
            // copy: with a session live in Home, the line drops the connect prompt along with its button.
            Text(offersConnectPrompt ? "Bring an agent in with the connect prompt, or open the composer and say hello."
                                     : "Open the composer and say hello.")
                .gallopText(.bodyM, color: SemanticColor.textTertiary)
                .multilineTextAlignment(.center)

            if offersConnectPrompt { quietRoomPromptButton }
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var quietRoomPromptButton: some View {
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
                Button {
                    roomKeyDraft = ""
                    roomKeyError = nil
                    isRoomKeyPresented = true
                } label: {
                    Label(store.unlockedRoomIDs.contains(room.id) ? "Room key saved" : "Add room key to read and send messages", systemImage: "lock.fill")
                        .font(.caption)
                }.buttonStyle(.plain).padding(.bottom, 8)
            } else if !store.connectionStatus.isConnected {
                Label("Offline — reconnect before sending.", systemImage: "wifi.slash")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    .padding(.bottom, 8)
            }

            HStack(spacing: 8) {
                ComposerTextField(
                    text: $store.draft,
                    placeholder: "Message \(title)",
                    isEnabled: !room.encrypted || store.unlockedRoomIDs.contains(room.id),
                    onSubmit: sendDraft,
                    onCancel: {
                        withAnimation(.easeInOut(duration: 0.18)) { isComposerExpanded = false }
                    },
                    onKey: handleComposerKey,
                    focusRequest: composerFocusRequest,
                    mentionMarks: mentionMarks
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

                Button { sendDraft() } label: {
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
                    action: sendDraft
                )
            }
            // The completion floats over the feed, 8 above the field and lined up with its start, as a menu would: a
            // frame with no height at the field's top holds it by its bottom edge.
            .overlay(alignment: .topLeading) {
                let options = mentionOptions
                if !options.isEmpty {
                    MentionCompletion(
                        options: options,
                        highlight: min(mentionHighlight, options.count - 1),
                        pick: pickMention
                    )
                    .frame(height: 0, alignment: .bottomLeading)
                    .offset(y: -8)
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity)
        .background(SemanticColor.surface500)
        .onChange(of: HomeSessions.mentionQuery(in: store.draft)) { _ in mentionHighlight = 0 }
    }

    /// Sends the draft, with the live sessions it names after an @ as its mentions when it is Home's.
    private func sendDraft() {
        store.sendDraft(mentions: isHome ? home.mentions(in: store.draft) : [])
    }

    /// The arrow keys move through the @ completion, Return or Tab puts the highlighted name in the draft, and Escape
    /// closes it. With the completion closed the field takes every key itself.
    private func handleComposerKey(_ key: ComposerTextField.Key) -> Bool {
        let options = mentionOptions
        guard !options.isEmpty else { return false }
        let current = min(mentionHighlight, options.count - 1)
        switch key {
        case .up: mentionHighlight = max(0, current - 1)
        case .down: mentionHighlight = min(options.count - 1, current + 1)
        case .tab, .enter: pickMention(options[current])
        case .escape: mentionClosedDraft = store.draft
        }
        return true
    }

    private func pickMention(_ session: LiveSession) {
        store.draft = HomeSessions.completing(store.draft, with: home.name(of: session))
        composerFocusRequest += 1
    }

    /// Opening Home from a session's row starts its draft with the session's @name; the composer opens with the
    /// cursor after it.
    private func takeComposerRequest(_ roomID: String?) {
        guard roomID == room.id else { return }
        store.composerRequestRoomID = nil
        isComposerExpanded = true
        composerFocusRequest += 1
    }

    private var canSend: Bool {
        store.connectionStatus.isConnected
            && (!room.encrypted || store.unlockedRoomIDs.contains(room.id))
            && !store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

/// The @ completion over Home's composer: the live sessions the typed name matches, each with its harness mark, its
/// one name and where it runs, as Home's rows name them. The app has no menu that floats over a text field without
/// taking its keys, so this draws the demo's picker (a Gallop Submenu of MenuItem rows, 320 wide) on the app's
/// floating glass: Body L names, Body S lines, 16 at the sides and 12 above and below each row. The highlighted row
/// takes the glass hover, since Return or Tab picks it. Past five rows it stops at five and a half, so the cut row
/// says it scrolls, and the highlighted row scrolls into view as the arrows move it.
private struct MentionCompletion: View {
    @EnvironmentObject private var home: HomeSessions
    let options: [LiveSession]
    let highlight: Int
    let pick: (LiveSession) -> Void

    /// A row: 12 above and below, a 24 name line, 2 between and a 20 line under it.
    private static let rowHeight: CGFloat = 12 + 24 + 2 + 20 + 12
    private static let rowSpacing: CGFloat = 2
    private static let padding: CGFloat = 8
    private static let rowsShown = 5
    private static var cappedHeight: CGFloat {
        padding + CGFloat(rowsShown) * (rowHeight + rowSpacing) + rowHeight / 2
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 32, style: .continuous)
        Group {
            if options.count > Self.rowsShown {
                ScrollViewReader { proxy in
                    ScrollView { rows }
                        .frame(height: Self.cappedHeight)
                        .onAppear { proxy.scrollTo(highlight) }
                        .onChange(of: highlight) { proxy.scrollTo($0) }
                }
            } else {
                rows
            }
        }
        .frame(width: 320)
        .background {
            shape.fill(.ultraThinMaterial)
                .overlay { shape.fill(SemanticColor.surfaceGlass500) }
        }
        .clipShape(shape)
        .overlay { shape.stroke(SemanticColor.surfaceGlassBorderHighlight, lineWidth: 1) }
        .shadow(color: .black.opacity(0.08), radius: 2, y: 1)
        .shadow(color: .black.opacity(0.04), radius: 0, y: 0.5)
    }

    private var rows: some View {
        VStack(alignment: .leading, spacing: Self.rowSpacing) {
            ForEach(Array(options.enumerated()), id: \.element.id) { index, session in
                MentionCompletionRow(
                    name: home.name(of: session),
                    line: session.place,
                    harness: session.harness,
                    isHighlighted: index == highlight
                ) { pick(session) }
                .id(index)
            }
        }
        .padding(Self.padding)
    }
}

private struct MentionCompletionRow: View {
    let name: String
    let line: String
    let harness: LiveSession.Harness
    let isHighlighted: Bool
    let action: () -> Void
    @State private var isHovering = false

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 24, style: .continuous)
        let lit = isHighlighted || isHovering
        Button(action: action) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 12) {
                    HarnessMark(harness: harness, size: 20)
                        .frame(minHeight: 24)
                    Text(name)
                        .gallopText(
                            .bodyL,
                            color: lit ? SemanticColor.surfaceGlassOffTextHover : SemanticColor.surfaceGlassOffTextDefault
                        )
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                Text(line)
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.leading, 32)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(lit ? SemanticColor.surfaceGlassOffHover : .clear, in: shape)
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
        .help(harness.appName)
        .macAccessibleAction(label: "Mention \(name)", value: isHighlighted ? "selected" : nil, action: action)
    }
}

/// Distance of the feed's bottom marker from the top of the scroll viewport;
/// smaller than the viewport height means the user is at (or near) the bottom.
private struct FeedBottomOffsetKey: PreferenceKey {
    static var defaultValue: CGFloat = .infinity
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = min(value, nextValue())
    }
}

private struct MessageFeedRow: View {
    let message: ChatMessage
    let isMine: Bool
    let now: Date
    /// A message the room hides by default, drawn because its run is open or the room's switch is on. Its text takes
    /// Text/Tertiary.
    var quiet = false
    /// The name over the message when the room names its sender itself, as Home does its own sessions.
    var senderName: String?
    var onSaveAttachment: ((FileAttachment) -> Void)?
    var onOpenAttachment: ((FileAttachment) -> Void)?
    @State private var isHovering = false

    var body: some View {
        if isMine {
            HStack(alignment: .bottom) {
                Spacer(minLength: 120)
                VStack(alignment: .leading, spacing: 7) {
                    ExpandableMessageText(content: message.content, textColor: SemanticColor.textPrimary)
                    attachmentChip
                    carriedWork
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
                        Text(senderName ?? message.agentName)
                            .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                        Text(relativeTimestamp)
                            .gallopText(.caption, color: SemanticColor.textTertiary)
                        if let app = AgentAppResolver.resolvedApp(forAgentNamed: message.agentName),
                           AgentAppResolver.applicationURL(for: app) != nil {
                            OpenInAgentAppChip(app: app, isVisible: isHovering)
                        }
                    }
                    .modifier(OpenInAppAccessibility(label: openInLabel, value: relativeTimestamp, action: openInApp))
                    ExpandableMessageText(
                        content: message.content,
                        textColor: quiet ? SemanticColor.textTertiary : SemanticColor.textSecondary
                    )
                    attachmentChip
                    carriedWork
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

    /// What the message carries under its text: its pull request's row, its pictures as thumbnails, and its other
    /// files as attachment chips.
    @ViewBuilder
    private var carriedWork: some View {
        if let pullRequest = message.metadata.pullRequest {
            PullRequestRow(pullRequest: pullRequest)
        }
        let pictures = message.attachments.filter(\.isImage)
        if !pictures.isEmpty, let onOpenAttachment {
            MessageFlowLayout(spacing: 8, lineSpacing: 8) {
                ForEach(pictures, id: \.blobID) { picture in
                    AttachmentThumbnail(attachment: picture, onOpen: onOpenAttachment)
                }
            }
        }
        ForEach(message.attachments.filter { !$0.isImage }, id: \.blobID) { file in
            attachmentChip(file)
        }
    }

    @ViewBuilder
    private var attachmentChip: some View {
        if let attachment = message.fileAttachment { attachmentChip(attachment) }
    }

    @ViewBuilder
    private func attachmentChip(_ attachment: FileAttachment) -> some View {
        if onSaveAttachment != nil {
            Button {
                onSaveAttachment?(attachment)
            } label: {
                HStack(spacing: 9) {
                    Image(systemName: "doc.fill")
                        .font(.system(size: 15, weight: .medium))
                        .foregroundStyle(SemanticColor.iconSecondary)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(attachment.name)
                            .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        if let size = attachment.size {
                            Text(ByteCountFormatter.string(
                                fromByteCount: Int64(size), countStyle: .file
                            ))
                                .gallopText(.caption, color: SemanticColor.textTertiary)
                        }
                    }
                    Image(systemName: "arrow.down.circle")
                        .font(.system(size: 14, weight: .medium))
                        .foregroundStyle(SemanticColor.iconTertiary)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    SemanticColor.surface600,
                    in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                )
                .overlay {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .stroke(SemanticColor.borderDefault, lineWidth: 1)
                }
                .contentShape(RoundedRectangle(cornerRadius: 10))
            }
            .buttonStyle(.plain)
            .frame(maxWidth: 340, alignment: .leading)
            .macAccessibleAction(label: "Save \(attachment.name)") {
                onSaveAttachment?(attachment)
            }
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
        resolvedApp.map { "\(senderName ?? message.agentName), open in \($0.displayName)" }
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
    @Environment(\.mentionMarks) private var mentionMarks
    @State private var isExpanded = false
    @State private var expandedPlan: MessageRenderPlan?
    private let chart: ChartMessage?
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

        let chart = ChartMessage(content: content)
        self.chart = chart
        let preview = MessagePreview.collapsedPreview(for: chart?.title ?? content)
        collapsedPreview = preview
        collapsedPlan = makeRenderPlan(preview.source)
    }

    var body: some View {
        Group {
            if let chart { ChartMessageView(chart: chart) }
            else if content.hasPrefix(ChartMessage.prefix) {
                Label("This chart could not be displayed.", systemImage: "photo")
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
            } else { textBody }
        }
    }

    private var textBody: some View {
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
            Text(mentionMarks.marking(AttributedString(source)))
                .textSelection(.enabled)
                .gallopText(.bodyL, color: textColor)
                .fixedSize(horizontal: false, vertical: true)
        case .structured(let segments):
            VStack(alignment: .leading, spacing: 8) {
                ForEach(segments) { segment in
                    switch segment.kind {
                    case .prose:
                        Text(mentionMarks.marking(MessageContentParser.attributedInline(segment.text)))
                            .textSelection(.enabled)
                            .gallopText(.bodyL, color: textColor)
                            .fixedSize(horizontal: false, vertical: true)
                    case .heading(let level):
                        Text(mentionMarks.marking(MessageContentParser.attributedInline(segment.text)))
                            .textSelection(.enabled)
                            .gallopText(headingStyle(for: level), color: textColor)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityAddTraits(.isHeader)
                    case .unorderedList, .orderedList:
                        Text(mentionMarks.marking(MessageContentParser.attributedInline(segment.text)))
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
                            Text(mentionMarks.marking(MessageContentParser.attributedInline(segment.text)))
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

struct RoomAvatar: View {
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

/// What the window's main area draws: a page chosen from the sidebar, the selected room's chat, or the empty state.
/// The lobby is an ordinary room and opens as a chat (Patrick removed the Lobby page, 2 November).
enum MainArea: Equatable {
    case page(WorkspaceStore.Page)
    case chat(Room)
    case empty

    static func content(page: WorkspaceStore.Page?, selectedRoom: Room?) -> MainArea {
        if let page { return .page(page) }
        if let selectedRoom { return .chat(selectedRoom) }
        return .empty
    }
}

private struct EmptyChatView: View {
    @EnvironmentObject private var store: ChatStore

    var body: some View {
        Group {
            if store.rooms.isEmpty, case .failed = store.connectionStatus {
                connectionFailedState
            } else {
                // The quiet room's icon and title with no line under them (Patrick removed the Lobby page and its
                // welcome, 2 November). With no rooms, New room sits under the title.
                VStack(spacing: 10) {
                    GallopIconView(icon: .message, fallbackSystemName: "bubble.left", size: 24)
                        .foregroundStyle(SemanticColor.iconTertiary)
                    // copy:
                    Text(store.rooms.isEmpty ? "No rooms yet" : "No room open")
                        .gallopText(.h5, color: SemanticColor.textPrimary)

                    if store.rooms.isEmpty {
                        // copy:
                        Button("New room") { store.presentCreateRoom() }
                            .buttonStyle(CowchatPillButtonStyle(prominent: true))
                            .keyboardShortcut(.defaultAction)
                            .padding(.top, 6)
                            .macAccessibleAction(label: "Create room") { store.presentCreateRoom() }
                    }
                }
                .padding(.horizontal, 24)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(SemanticColor.surface500)
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
    case account
    case connection
    case agents
    case theme
    case roomIcons
    case updates
    case notifications
    case whatsNew

    var title: String {
        switch self {
        // copy:
        case .account: return "Account"
        case .connection: return "Connection"
        case .agents: return "Agents" // copy:
        case .theme: return "Theme"
        case .roomIcons: return "Room icons"
        case .updates: return "Updates"
        case .notifications: return "Notifications"
        case .whatsNew: return "What’s new"
        }
    }

    /// The line under the title. Account has none.
    var subtitle: String? {
        switch self {
        case .account: return nil
        case .connection: return "Choose where Cowchat stores and syncs your rooms."
        // copy:
        case .agents: return "Connect Claude Code and Codex to your Home."
        case .theme: return "Choose how Cowchat appears on this Mac."
        case .roomIcons: return "Every room gets the same icon on every Mac — it is drawn from the room name."
        case .updates: return "Keep Cowchat up to date."
        case .notifications: return "Choose what gets your attention on this Mac."
        case .whatsNew: return "The latest improvements to Cowchat."
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
    @EnvironmentObject private var softwareUpdater: SoftwareUpdater
    @EnvironmentObject private var workspace: WorkspaceStore
    @EnvironmentObject private var store: ChatStore
    @Binding var isPresented: Bool
    let onShowOnboarding: () -> Void
    @AppStorage("CowchatMac.appearance") private var appearance = AppAppearance.system.rawValue
    @AppStorage(RoomIconStyle.storageKey) private var roomIconStyle = RoomIconStyle.fallback.rawValue
    /// The page chosen in the list, or nil for the first: Account while an account is signed in, else Connection.
    @State private var chosenPage: SettingsPage?
    @State private var cloudURL = ""
    @State private var cloudAPIKey = ""
    @State private var isConnectingGlobal = false
    @State private var inviteToken = ""
    @State private var isRedeemingInvite = false
    @State private var inviteRedeemError: String?

    var body: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                Text("Preferences")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    .padding(.horizontal, 16)
                    .padding(.top, 20)
                    .padding(.bottom, 8)
                if isSignedIn {
                    // copy:
                    settingsNavigationRow(
                        "Account",
                        systemName: "person.crop.circle",
                        page: .account
                    )
                }
                settingsNavigationRow(
                    "Connection",
                    systemName: "network",
                    page: .connection
                )
                settingsNavigationRow(
                    "Agents",
                    systemName: "terminal",
                    page: .agents
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
                settingsNavigationRow(
                    "Notifications",
                    systemName: "bell",
                    page: .notifications
                )
                settingsNavigationRow(
                    "Updates",
                    systemName: "arrow.down.circle",
                    page: .updates
                )
                settingsNavigationRow(
                    "What’s new",
                    systemName: "sparkles",
                    page: .whatsNew
                )
                Spacer()
            }
            .frame(width: 230)
            .background(SemanticColor.surface400)

            Rectangle().fill(SemanticColor.borderDefault).frame(width: 1)

            VStack(alignment: .leading, spacing: 24) {
                settingsHeader

                switch selectedPage {
                case .account:
                    ScrollView {
                        AccountSettings(social: workspace.social) {
                            isPresented = false
                            Task { await workspace.signOut() }
                        }
                    }
                case .connection:
                    ScrollView {
                        connectionSettings
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                case .agents:
                    agentSettings
                case .theme:
                    themeSettings
                case .roomIcons:
                    roomIconSettings
                case .updates:
                    updateSettings
                case .notifications:
                    SocialNotificationSettings(notifications: workspace.notifications)
                case .whatsNew:
                    ScrollView { WhatsNewContent() }
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

    private var updateSettings: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("Cowchat \(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")")
                .gallopText(.bodyMStrong)
            Toggle("Automatically check for updates", isOn: Binding(
                get: { softwareUpdater.automaticallyChecksForUpdates },
                set: { softwareUpdater.setAutomaticallyChecksForUpdates($0) }
            ))
            .disabled(!softwareUpdater.isAvailable)
            Toggle("Download and install updates automatically", isOn: Binding(
                get: { softwareUpdater.automaticallyDownloadsUpdates },
                set: { softwareUpdater.setAutomaticallyDownloadsUpdates($0) }
            ))
            .disabled(!softwareUpdater.isAvailable || !softwareUpdater.automaticallyChecksForUpdates)
            Text("Updates include the app and its bundled server. Cowchat waits until you quit or choose to relaunch before installing.")
                .gallopText(.bodyS, color: SemanticColor.textSecondary)
            Button("Check for Updates…") { softwareUpdater.checkForUpdates() }
                .buttonStyle(CowchatPillButtonStyle(prominent: true))
                .disabled(!softwareUpdater.canCheckForUpdates)
            if !softwareUpdater.isAvailable {
                Text("Automatic updates are available in installed release builds.")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
            }
            Text("Standalone command-line tools and separately running servers update through Homebrew or your package manager.")
                .gallopText(.caption, color: SemanticColor.textTertiary)
        }
    }

    private var isSignedIn: Bool { workspace.social.phase == .signedIn }

    private var selectedPage: SettingsPage {
        let page = chosenPage ?? (isSignedIn ? .account : .connection)
        return page == .account && !isSignedIn ? .connection : page
    }

    private var settingsHeader: some View {
        HStack {
            VStack(alignment: .leading, spacing: 3) {
                Text(selectedPage.title)
                    .gallopText(.h4, color: SemanticColor.textPrimary)
                if let subtitle = selectedPage.subtitle {
                    Text(subtitle)
                        .gallopText(.bodyM, color: SemanticColor.textTertiary)
                }
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

            if let accountGlobal = workspace.accountGlobal {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text("Global rooms · account").gallopText(.bodyMStrong)
                        Spacer()
                        ServerStatusBadge(store: accountGlobal)
                    }
                    Text("Your Home and rooms shared with friends. Only invited people and approved agents can join.")
                        .gallopText(.bodyM, color: SemanticColor.textTertiary)
                }.gallopCard(cornerRadius: 14)
            }

            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text("Global rooms · API key")
                        .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                    Spacer()
                    if let global = workspace.global {
                        ServerStatusBadge(store: global)
                    }
                }
                Text("Find global rooms you can access, such as cowboy-junction, from the sidebar. Manage this connection's server and API key here.")
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
                if workspace.isGlobalEnabled {
                    HStack(spacing: 10) {
                        TextField("Have an invite? Paste its cinv_… token", text: $inviteToken)
                            .textFieldStyle(.plain)
                            .gallopText(.bodyM, color: SemanticColor.textPrimary)
                            .padding(.horizontal, 13)
                            .frame(height: 36)
                            .background(SemanticColor.textfieldDefault, in: Capsule())
                            .overlay {
                                Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
                            }
                        Button(isRedeemingInvite ? "Redeeming…" : "Redeem") { redeemInvite() }
                            .buttonStyle(.plain)
                            .gallopText(.bodySStrong, color: SemanticColor.buttonSecondaryTextDefault)
                            .padding(.horizontal, 14)
                            .frame(height: 36)
                            .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
                            .overlay {
                                Capsule().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                            }
                            .disabled(!canRedeemInvite)
                            .opacity(canRedeemInvite ? 1 : 0.45)
                            .fixedSize()
                            .macAccessibleAction(
                                label: "Redeem invite",
                                isEnabled: canRedeemInvite,
                                action: redeemInvite
                            )
                    }
                    if let inviteRedeemError {
                        Text(inviteRedeemError)
                            .gallopText(.caption, color: SemanticColor.textError)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                HStack(spacing: 10) {
                    Spacer()
                    if workspace.isGlobalEnabled {
                        Button("Disconnect community rooms") { workspace.disableGlobalRooms() }
                            .buttonStyle(.plain)
                            .gallopText(.bodyMStrong, color: SemanticColor.buttonSecondaryTextDefault)
                            .padding(.horizontal, 20)
                            .frame(height: 42)
                            .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
                            .overlay {
                                Capsule().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                            }
                            .fixedSize()
                            .macAccessibleAction(label: "Disconnect community rooms") {
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


    /// Claude Code and Codex with where each stands, and Connect on one that isn't connected, for a person who
    /// skipped them in first run. A signed-in account's bots follow in a list of their own, then the switch that shows
    /// the person's live sessions on Team.
    private var agentSettings: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                VStack(alignment: .leading, spacing: 12) {
                    AgentHookList(connections: workspace.agentConnections).gallopCard(cornerRadius: 14)
                    AgentHookError(connections: workspace.agentConnections)
                }
                if isSignedIn {
                    BotSettings(social: workspace.social)
                }
                if workspace.accountGlobal != nil {
                    TeamSharingSettings(team: workspace.team, social: workspace.social)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .task { await workspace.agentConnections.refresh() }
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
        let live = (workspace.local.rooms + (workspace.global?.rooms ?? []) + (workspace.accountGlobal?.rooms ?? []))
            .map(\.name).prefix(3)
        return live.isEmpty ? ["Lobby", "harness-signing", "canyon-deploy"] : Array(live)
    }

    private func settingsNavigationRow(
        _ title: String,
        systemName: String,
        page: SettingsPage
    ) -> some View {
        Button { chosenPage = page } label: {
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
        .macAccessibleAction(label: title, value: selectedPage == page ? "selected" : nil) { chosenPage = page }
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

    private var canRedeemInvite: Bool {
        !isRedeemingInvite
            && !inviteToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && workspace.global?.connectionStatus.isConnected == true
    }

    private func redeemInvite() {
        guard canRedeemInvite, let global = workspace.global else { return }
        isRedeemingInvite = true
        inviteRedeemError = nil
        Task {
            do {
                let room = try await global.redeemInvite(token: inviteToken)
                inviteToken = ""
                if let room {
                    await workspace.select(room: room, on: .global)
                    isPresented = false
                }
            } catch {
                inviteRedeemError = error.localizedDescription
            }
            isRedeemingInvite = false
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

            CapsuleField(placeholder: "Room name", text: $name, onSubmit: renameRoom)

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

private struct RoomCreationSheet: View {
    private enum Location: String, CaseIterable { case local = "Local", global = "Online" }
    @EnvironmentObject private var workspace: WorkspaceStore
    @EnvironmentObject private var store: ChatStore
    @State private var location: Location
    @State private var preferPrivate: Bool

    init(initialSource: WorkspaceStore.Server) {
        _location = State(initialValue: initialSource == .local ? .local : .global)
        _preferPrivate = State(initialValue: initialSource != .global)
    }

    private var hasGlobal: Bool { workspace.global != nil || workspace.accountGlobal != nil }
    private var usePrivateGlobal: Bool {
        location == .global && workspace.accountGlobal != nil && (workspace.global == nil || preferPrivate)
    }

    var body: some View {
        VStack(spacing: 0) {
            if store.createRoomParentID == nil {
                VStack(alignment: .leading, spacing: 12) {
                    if hasGlobal {
                        Picker("Room location", selection: $location) {
                            ForEach(Location.allCases, id: \.self) { location in
                                Text(location.rawValue).tag(location)
                            }
                        }.pickerStyle(.segmented)
                    } else {
                        Text("Local · This Mac").gallopText(.bodySStrong)
                    }
                    if location == .global, workspace.global != nil, workspace.accountGlobal != nil {
                        Picker("Room audience", selection: $preferPrivate) {
                            Text("You and friends").tag(true)
                            Text("With API key").tag(false)
                        }.pickerStyle(.segmented)
                    }
                }
                .padding(.horizontal, 28).padding(.top, 20).padding(.bottom, 8)
            }
            if usePrivateGlobal, store.createRoomParentID == nil {
                SocialRoomCreationView(social: workspace.social, draft: workspace.newRoomDraft())
            } else if location == .global && !hasGlobal {
                Text("The online connection was disconnected. Choose Local to create a room on this Mac.")
                    .gallopText(.bodyM).padding(28)
                Button("Create a local room") { location = .local }.padding(.bottom, 28)
            } else {
                CreateRoomView(
                    local: workspace.local,
                    globalOrLocal: workspace.global ?? workspace.local,
                    hasGlobal: workspace.global != nil,
                    source: location == .local ? .local : .global
                )
            }
        }.background(SemanticColor.surface600)
    }
}

private struct CreateRoomView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @EnvironmentObject private var store: ChatStore
    @ObservedObject var local: ChatStore
    @ObservedObject var globalOrLocal: ChatStore
    let hasGlobal: Bool
    var source: WorkspaceStore.Server? = nil
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
        return source ?? chosenServer ?? workspace.activeServer
    }

    private var targetStore: ChatStore {
        switch targetServer {
        case .local: return local
        case .global: return hasGlobal ? globalOrLocal : local
        case .accountGlobal: return workspace.accountGlobal ?? local
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
                                : "Create a conversation on the community Cowchat server.")
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
                if hasGlobal, parentRoom == nil, source == nil {
                    Picker("Server", selection: serverSelection) {
                        ForEach([WorkspaceStore.Server.local, .global]) { server in
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
                isPublic: false
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
