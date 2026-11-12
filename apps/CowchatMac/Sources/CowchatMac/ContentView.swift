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
    /// The height under the toolbar, which a starting sheet keeps within.
    @State private var contentHeight: CGFloat?
    @Environment(\.controlActiveState) private var controlActiveState

    init(onShowOnboarding: @escaping () -> Void = {}) {
        self.onShowOnboarding = onShowOnboarding
    }

    var body: some View {
        HStack(spacing: 0) {
            if isSidebarVisible {
                SidebarView(isSidebarVisible: $isSidebarVisible)
                .frame(width: 280)
                .transition(.move(edge: .leading).combined(with: .opacity))
            }

            Group {
                switch MainArea.content(page: workspace.page, selectedRoom: store.selectedRoom) {
                case .page(.inbox):
                    InboxPage(inbox: workspace.inbox, sessions: workspace.liveSessions, social: workspace.social)
                case .page(.team):
                    TeamPage(team: workspace.team, account: workspace.accountGlobal ?? workspace.local)
                case .page(.friends):
                    FriendsPage(social: workspace.social, sessions: FriendsList.sessions(workspace.sidebarRooms),
                                addToSession: { await workspace.addFriend($0.id, to: $1) }, signIn: onShowOnboarding)
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
        .background {
            GeometryReader { content in
                Color.clear
                    .onAppear { contentHeight = content.size.height }
                    .onChange(of: content.size.height) { contentHeight = $0 }
            }
        }
        .environmentObject(workspace.liveSessions)
        // The unified toolbar otherwise paints its own system strip; tint it
        // with the same surface400 shell so the nav reads as one tan surface.
        .toolbarBackground(SemanticColor.surface400, for: .windowToolbar)
        .navigationTitle(windowTitle)
        .toolbar {
            // The three buttons are one set: a 20 glyph in Icon/Secondary in a 36 circle that is clear at rest and takes
            // a sidebar row's hover under the pointer (navigation round 2, section 2).
            ToolbarItem(placement: .primaryAction) {
                ToolbarCircleHover { isHovering in
                    Button { workspace.show(.friends) } label: {
                        // Gallop has no outline person glyph (@cowboyinc/gallop-foundations 0.7.0 has only the filled
                        // Posse), so this is SF Symbol person.2. At 14 semibold its stroke is about 1.4, close to Add's
                        // and Sidebar.Expanded's beside it.
                        ToolbarCircleLabel(isHovering: isHovering) {
                            Image(systemName: "person.2")
                                .font(.system(size: 14, weight: .semibold))
                        }
                    }
                    .buttonStyle(.plain)
                    // copy:
                    .help("Friends")
                    .macAccessibleAction(label: "Open Friends") { workspace.show(.friends) }
                }
            }
            ToolbarItem(placement: .navigation) {
                ToolbarCircleHover { isHovering in
                    Button {
                        withAnimation(.easeInOut(duration: 0.2)) { isSidebarVisible.toggle() }
                    } label: {
                        // Gallop's Sidebar.Expanded while the sidebar shows and Sidebar.Collapsed while it is hidden,
                        // as the prototype site's rail swaps them.
                        ToolbarCircleLabel(isHovering: isHovering) {
                            GallopIconView(icon: isSidebarVisible ? .sidebarExpanded : .sidebarCollapsed,
                                           fallbackSystemName: "sidebar.left", size: 20)
                        }
                    }
                    .buttonStyle(.plain)
                    .focusable(false)
                    .help(isSidebarVisible ? "Hide sidebar" : "Show sidebar")
                    .macAccessibleAction(label: "Toggle sidebar") {
                        withAnimation(.easeInOut(duration: 0.2)) { isSidebarVisible.toggle() }
                    }
                }
            }
            ToolbarItem(placement: .navigation) {
                // The New menu holds every way to start something (navigation note, section 7.1). It stays in reach
                // while the sidebar is hidden.
                ToolbarCircleHover { isHovering in
                    Menu {
                        NewMenuItems(workspace: workspace)
                    } label: {
                        ToolbarCircleLabel(isHovering: isHovering) {
                            GallopIconView(icon: .add, fallbackSystemName: "plus", size: 20)
                        }
                    }
                    .menuStyle(.button)
                    .buttonStyle(.plain)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    // copy:
                    .help("New")
                    .accessibilityLabel("New")
                }
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
            isWhatsNewPresented = false
            store.roomBeingRenamed = nil
            workspace.starting = nil
            workspace.roomPeople = nil
            workspace.show(destination.page)
        }
        .onReceive(NotificationCenter.default.publisher(for: .cowchatShowFriends)) { _ in
            workspace.show(.friends)
        }
        .sheet(item: $workspace.starting) { sheet in
            Group {
                switch sheet {
                case let .newRoom(request): NewRoomSheet(workspace: workspace, request: request)
                case .joinWithCode: JoinWithCodeSheet(workspace: workspace)
                case let .addPerson(session): AddPersonSheet(workspace: workspace, session: session)
                }
            }
            // 12 clear above and below the sheet.
            .environment(\.sheetHeightLimit, contentHeight.map { $0 - 24 })
        }
        .sheet(isPresented: Binding(get: { workspace.roomPeople != nil },
                                    set: { if !$0 { workspace.roomPeople = nil } })) {
            RoomParticipantsSheet(social: workspace.social, participants: $workspace.roomPeople)
        }
        .onReceive(NotificationCenter.default.publisher(for: .cowchatShowSettings)) { _ in
            // The app menu's command reaches the key window. A post while the app is in the background, which only
            // the prototype's director makes, reaches the window too.
            guard controlActiveState == .key || controlActiveState == .active || !NSApp.isActive else { return }
            isSettingsPresented = true
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
            }
        }
        .animation(.easeInOut(duration: 0.2), value: store.roomReadyNotice?.id)
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

/// A toolbar button's label (navigation round 2, section 2): its 20 glyph in Icon/Secondary, centred in a 36 circle
/// that is clear at rest and takes a sidebar row's hover under the pointer, whose capsule is a circle at 36. It has no
/// pressed state, since a SwiftUI menu can't show one and the three buttons must look alike.
private struct ToolbarCircleLabel<Glyph: View>: View {
    let isHovering: Bool
    @ViewBuilder let glyph: () -> Glyph

    var body: some View {
        glyph()
            .foregroundStyle(SemanticColor.iconSecondary)
            .frame(width: 20, height: 20)
            .frame(width: 36, height: 36)
            .background(SidebarRowBackground(state: .init(isSelected: false, isHovering: isHovering)))
            .contentShape(Circle())
    }
}

/// A toolbar button's hover, taken on the button or the menu itself, as the sidebar's filter button and
/// `SheetMenuField` take theirs, so the three toolbar buttons take it the same way.
private struct ToolbarCircleHover<Control: View>: View {
    @ViewBuilder let control: (_ isHovering: Bool) -> Control
    @State private var isHovering = false

    var body: some View {
        control(isHovering)
            .onHover { isHovering = $0 }
    }
}

private struct SidebarView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @Binding var isSidebarVisible: Bool
    /// What a legacy scroller takes from the list's right edge, which the pinned rows give up too so they end where the
    /// cells end.
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
        VStack(alignment: .leading, spacing: 0) {
            // Pinned, and never scrolled: the search field with its filter, 8 under the toolbar so its top is level
            // with the main card's, then 12 under it the Inbox, Team and the line under them, 8 in from both sides, so
            // they share one left edge with the cells (navigation round 2, section 3.1). Signed out only the search
            // field is left.
            VStack(alignment: .leading, spacing: 0) {
                SearchCapsule(placeholder: workspace.sidebarFilter.searchPlaceholder, text: $workspace.searchText,
                              isFocused: $isSearchFocused, clearLabel: "Clear search") {
                    SidebarFilterButton()
                }
                AccountNavRows(inbox: workspace.inbox)
            }
            .padding(.horizontal, 8)
            .padding(.trailing, listsScrollerWidth)

            SidebarList(
                local: workspace.local,
                globalOrLocal: workspace.global ?? workspace.local,
                accountOrLocal: workspace.accountGlobal ?? workspace.local,
                sessions: workspace.liveSessions,
                inbox: workspace.inbox,
                hasAccountGlobal: workspace.accountGlobal != nil,
                hasGlobal: workspace.global != nil
            )
            .onPreferenceChange(SidebarScrollerWidthKey.self) { listsScrollerWidth = $0 }
        }
        .padding(.top, 8)
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
}

/// The sidebar's filter as Gallop's IconButton at size XS inside the search field: a 28 circle with the 16
/// Settings.Slider glyph that opens the filter's menu (navigation round 2, section 3.4). At All it is the ghost button;
/// at any other value it takes the Primary button's fill, the on recipe of the Gallop Chip it replaces. Signed out the
/// menu offers All and Archived only. A SwiftUI menu doesn't tell its label when it is pressed, so the button draws
/// hover and not press, and pressing it leaves the search's text unfocused.
private struct SidebarFilterButton: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @State private var isHovering = false

    var body: some View {
        let filter = workspace.sidebarFilter
        Menu {
            ForEach(SidebarFilter.offered(signedIn: workspace.accountGlobal != nil)) { value in
                Toggle(value.title, isOn: Binding(get: { value == workspace.sidebarFilter },
                                                  set: { if $0 { workspace.sidebarFilter = value } }))
            }
        } label: {
            GallopIconView(icon: .settingsSlider, fallbackSystemName: "slider.horizontal.3", size: 16)
                .foregroundStyle(glyph(isOn: filter != .all))
                .frame(width: 28, height: 28)
                .background(fill(isOn: filter != .all), in: Circle())
                .contentShape(Circle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .onHover { isHovering = $0 }
        // copy:
        .help("Filter")
        // copy:
        .accessibilityLabel("Filter, \(filter.title)")
    }

    private func fill(isOn: Bool) -> Color {
        if isOn { return isHovering ? SemanticColor.buttonPrimaryHover : SemanticColor.buttonPrimaryDefault }
        return isHovering ? SemanticColor.buttonGhostHover : Color.clear
    }

    private func glyph(isOn: Bool) -> Color {
        if isOn { return SemanticColor.buttonPrimaryIconDefault }
        return isHovering ? SemanticColor.buttonGhostIconHover : SemanticColor.buttonGhostIconDefault
    }
}

/// A destination above the list: its glyph at 18, 10 before the title in Body S Strong, 10 at the sides, 36 tall, on
/// the sidebar's row fill. The Inbox and Team draw it. A count sits at the row's end in Caption, Text/Tertiary, and
/// nothing at zero.
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
            .frame(height: 36)
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

/// The Inbox and Team, 12 under the search field and 2 apart, and the line under them with 8 above it, once the person
/// is signed in. The Inbox's
/// count is every unread item, open invitations included, and it shows nothing at zero. The line is 1 point of
/// Border/Default at the rows' padding. The note's Surface/Glass/Border/Shadow is the dark sidebar's own colour, so the
/// line could not be seen in dark mode.
private struct AccountNavRows: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var inbox: InboxStore

    var body: some View {
        if workspace.accountGlobal != nil {
            VStack(spacing: 0) {
                // copy:
                SidebarNavRow(title: "Inbox", icon: .incoming, fallbackSystemName: "tray.and.arrow.down",
                              count: inbox.unreadCount, isSelected: workspace.page == .inbox) {
                    workspace.show(.inbox)
                }
                // copy:
                SidebarNavRow(title: "Team", icon: .posse, fallbackSystemName: "person.2",
                              isSelected: workspace.page == .team) {
                    workspace.show(.team)
                }
                .padding(.top, 2)
                Rectangle()
                    .fill(SemanticColor.borderDefault)
                    .frame(height: 1)
                    .padding(.horizontal, 10)
                    .padding(.top, 8)
                    .accessibilityHidden(true)
            }
            .padding(.top, 12)
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

/// The scrolling list under the sidebar's line: the rows the list model gives for the filter and the search
/// (`RoomSidebarPresentation`), with the cells of all three connections in one order. The order holds while the pointer
/// is over the list and re-sorts when it leaves, so a busy room never moves a cell under a press. The list never
/// scrolls by itself. Its scroll view takes its identity from the filter and from whether the search is empty, so it
/// starts from its top when either changes and at no other time (navigation note, section 3.6).
private struct SidebarList: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var local: ChatStore
    @ObservedObject var globalOrLocal: ChatStore
    @ObservedObject var accountOrLocal: ChatStore
    @ObservedObject var sessions: LiveSessions
    @ObservedObject var inbox: InboxStore
    let hasAccountGlobal: Bool
    let hasGlobal: Bool
    /// Each room's time as the pointer came over the list, which orders the list until the pointer leaves. A room
    /// that appears meanwhile goes to the end.
    @State private var frozenActivity: [String: Date]?
    /// The list's width and its content's, whose difference is what a legacy scroller takes.
    @State private var listsWidth: CGFloat = 0
    @State private var contentWidth: CGFloat = 0

    var body: some View {
        TimelineView(.periodic(from: .now, by: anyAgentsThinking ? 10 : 60)) { timeline in
            let rows = rows
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    if rows.isEmpty, let line = emptyLine {
                        Text(line)
                            .gallopText(.bodyS, color: SemanticColor.textTertiary)
                            .padding(.leading, 10)
                            .padding(.top, 4)
                    }
                    ForEach(rows) { row in
                        self.row(row, now: timeline.date)
                    }
                }
                .padding(.horizontal, 8)
                // The cells start 8 under the line, or 12 under the search field when there is no line.
                .padding(.top, hasAccountGlobal ? 8 : 12)
                .padding(.bottom, 12)
                .background {
                    GeometryReader { content in
                        Color.clear.preference(key: SidebarListsContentWidthKey.self, value: content.size.width)
                    }
                }
            }
            .id(scrollIdentity)
            .background {
                GeometryReader { viewport in
                    Color.clear
                        .onAppear { listsWidth = viewport.size.width }
                        .onChange(of: viewport.size.width) { listsWidth = $0 }
                }
            }
            .onPreferenceChange(SidebarListsContentWidthKey.self) { contentWidth = $0 }
            .preference(key: SidebarScrollerWidthKey.self,
                        value: contentWidth > 0 ? max(0, listsWidth - contentWidth) : 0)
            .scrollIndicators(.hidden)
            .onHover { frozenActivity = $0 ? liveActivity : nil }
        }
    }

    /// The filter, and whether a search is typed. A new value of either is a new list, shown from its top.
    private var scrollIdentity: String {
        "\(workspace.sidebarFilter.rawValue)/\(workspace.searchText.isEmpty)"
    }

    private var isSearching: Bool {
        !workspace.searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var rows: [SidebarRow] {
        let rooms = workspace.sidebarRooms
        let frozen = frozenActivity
        let activity: (SidebarRoom) -> Date = { room in
            guard let frozen else { return room.activity }
            return frozen[room.id] ?? .distantPast
        }
        if isSearching {
            return RoomSidebarPresentation.searchRows(query: workspace.searchText, filter: workspace.sidebarFilter,
                                                      rooms: rooms, joinable: workspace.sidebarJoinable,
                                                      activity: activity)
        }
        return RoomSidebarPresentation.rows(for: workspace.sidebarFilter, rooms: rooms, activity: activity)
    }

    private var emptyLine: String? {
        RoomSidebarPresentation.emptyLine(filter: workspace.sidebarFilter, isSearching: isSearching,
                                          isSearchingMessages: isSearchingMessages)
    }

    private var stores: [(WorkspaceStore.Server, ChatStore)] {
        [(.local, local)] + (hasGlobal ? [(.global, globalOrLocal)] : [])
            + (hasAccountGlobal ? [(.accountGlobal, accountOrLocal)] : [])
    }

    private func store(for server: WorkspaceStore.Server) -> ChatStore {
        switch server {
        case .local: return local
        case .global: return globalOrLocal
        case .accountGlobal: return accountOrLocal
        }
    }

    private var liveActivity: [String: Date] {
        Dictionary(workspace.sidebarRooms.map { ($0.id, $0.activity) }, uniquingKeysWith: { max($0, $1) })
    }

    private var anyAgentsThinking: Bool {
        stores.contains { !$0.1.lastThinkingAt.isEmpty }
    }

    private var isSearchingMessages: Bool {
        stores.contains { $0.1.isSearchingMessages }
    }

    /// What the cells of a connection show of it: down once it has been down for a while.
    private func connection(of server: WorkspaceStore.Server) -> SidebarConnection {
        let store = store(for: server)
        return SidebarConnection(status: store.connectionStatus, isDown: store.isConnectionDown)
    }

    @ViewBuilder
    private func row(_ row: SidebarRow, now: Date) -> some View {
        switch row {
        case let .cell(room):
            cell(room, now: now)
        case let .label(title):
            Text(title)
                .gallopText(.caption, color: SemanticColor.textTertiary)
                .padding(.leading, 10)
                // With the rows' 2 between, 8 under the group before and 4 over the group after.
                .padding(.top, 6)
                .padding(.bottom, 2)
        case let .joinable(item):
            let connection = connection(of: item.server)
            SidebarCell(name: item.name,
                        placeIcon: SidebarPlaceIcon.icon(place: .online, isLocalServer: false, connection: connection),
                        connection: connection, time: item.at?.cowchatShortTime(relativeTo: now) ?? "",
                        isSelected: false) {
                switch item.source {
                case let .invitation(itemID): workspace.openInvitation(itemID: itemID)
                case let .room(room): Task { await workspace.openJoinable(room) }
                }
            }
        }
    }

    private func cell(_ room: SidebarRoom, now: Date) -> some View {
        let store = store(for: room.server)
        let connection = connection(of: room.server)
        let placeIcon = SidebarPlaceIcon.icon(place: room.place, isLocalServer: room.server == .local,
                                              connection: connection)
        let mark: SidebarMark? = room.session.flatMap { session in
            switch session.state {
            case .working: return .working
            case .prompt: return .waiting(session.harness.appName)
            case .idle: return nil
            }
        }
        return SidebarCell(
            name: room.room.name, placeIcon: placeIcon, connection: connection, mark: mark,
            time: room.activity.cowchatShortTime(relativeTo: now), isUnread: room.isUnread,
            isSelected: workspace.isSelected(room.room, on: room.server),
            open: { Task { await workspace.select(room: room.room, on: room.server) } },
            addPerson: room.session.map { session in { workspace.presentAddPerson(to: session) } }
        ) {
            if room.isArchived {
                Button("Unarchive") { store.unarchive(room.room) }
            } else {
                if let session = room.session {
                    // copy:
                    Button("Add person…") { workspace.presentAddPerson(to: session) }
                }
                Button("Rename") { store.presentRename(room.room) }
                    .disabled(!store.canRename(room.room))
                if room.room.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame {
                    Button("Archive") {
                        Task { await store.archive(room.room) }
                    }
                }
            }
            if placeIcon.isDown {
                // copy:
                Button("Reconnect") { reconnect() }
            }
        }
    }

    /// Reconnects every connection that is down, as the sidebar footer's Reconnect button did.
    private func reconnect() {
        for (_, store) in stores where !store.connectionStatus.isConnected {
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

/// Cowboy's SidebarRowPill recipe, a capsule, which on the sidebar's 36 rows is Cowboy's pill. Every row in the sidebar
/// takes it: the Inbox, Team and every cell. The toolbar's buttons take its hover, as a circle at 36.
struct SidebarRowBackground: View {
    let state: SidebarRowState
    private var shape: Capsule { Capsule(style: .continuous) }

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

private struct ChatRoomView: View {
    @EnvironmentObject private var store: ChatStore
    @EnvironmentObject private var workspace: WorkspaceStore
    @EnvironmentObject private var sessions: LiveSessions
    let room: Room
    @Binding var isSidebarVisible: Bool
    @State private var isRoomKeyPresented = false
    @State private var roomKeyDraft = ""
    @State private var roomKeyError: String?
    @State private var isComposerExpanded = false
    @State private var isDestroyConfirmationPresented = false
    @State private var isDestroyingRoom = false
    @State private var isInvitesPresented = false
    @State private var isSlackPresented = false
    @State private var isLeasesPresented = false
    @State private var isArtifactPresented = false
    @State private var isMessageListNearBottom = true
    @State private var newMessageCount = 0
    @State private var hasCopiedEmptyRoomPrompt = false
    /// The list is revealed only after the initial bottom-anchor scroll, so a
    /// room switch never paints top-anchored content and animates it away.
    @State private var hasPositionedInitialScroll = false
    /// The runs of hidden agent messages opened in place, by their first message.
    @State private var openRuns: Set<String> = []

    private var title: String { room.name }

    private var isAccountRoom: Bool { workspace.activeServer == .accountGlobal }

    /// Whether the room offers People…, from the ⋯ menu and the header's caption.
    private var offersPeople: Bool {
        RoomPeople.isOffered(isLocal: store.isLocalConnection, apiKey: store.connectionProfile.apiKey)
    }

    /// Whether the viewer owns this account room, which is where a typed @name of one of their live sessions becomes a
    /// mention, since the app knows only the viewer's sessions.
    private var ownsRoom: Bool {
        let social = workspace.social
        let owner = social.rooms.first { $0.roomId == room.id }?.ownerId
        return isAccountRoom && owner != nil && owner == social.account?.id
    }

    /// In every account room the person's own sessions are named by their name alone, without the handle the server
    /// adds. Other senders keep the server's name.
    private func senderName(of message: ChatMessage) -> String? {
        guard isAccountRoom, let session = sessions.sessions.first(where: { $0.id == message.agentID }) else { return nil }
        return sessions.name(of: session)
    }

    /// The @names this room's messages and composer draw in the accent: the live sessions of the room's owner when
    /// that is the viewer, and the people the app knows.
    private var mentionMarks: MentionMarks {
        let social = workspace.social
        return MentionMarks(sessionNames: ownsRoom ? sessions.sessions.map(sessions.name(of:)) : [],
                            people: social.friends.map(\.account), viewer: social.account)
    }

    private var paneState: RoomPaneState {
        RoomPaneState.state(
            connectionStatus: store.connectionStatus,
            isLoadingMessages: store.isLoadingMessages,
            hasMessages: !store.messages.isEmpty
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
                switch paneState {
                case .empty: emptyRoom
                case .unreachable(let reason): unreachableRoom(reason)
                case .loading, .chat: EmptyView()
                }
                composer
            }
        }
        .background(SemanticColor.surface500)
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
                        if offersPeople {
                            // copy:
                            Button("People…") { Task { await workspace.showPeople(of: room.id) } }
                        }
                        if !isAccountRoom || ownsRoom {
                            Button("Invites…") { isInvitesPresented = true }
                        }
                        if !store.connectionProfile.apiKey.hasPrefix("csoc_") {
                            Button("Slack…") { isSlackPresented = true }
                        }
                    }
                    Divider()
                    if isAccountRoom, let session = sessions.session(inRoom: room.id) {
                        // copy:
                        Button("Add person…") { workspace.presentAddPerson(to: session) }
                    }
                    // Rename and destroy are creator-only, so for other
                    // agents' rooms they are hidden rather than left as
                    // permanently disabled clutter.
                    if store.canRename(room) {
                        Button("Rename room") { store.presentRename(room) }
                    }
                    Button("Archive room") { Task { await store.archive(room) } }
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
                    Text(title)
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    if room.encrypted {
                        GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 12)
                            .foregroundStyle(SemanticColor.iconTertiary)
                    }
                }
                TimelineView(.periodic(from: .now, by: 10)) { timeline in
                    let summary = presenceSummary(at: timeline.date)
                    let isActive = summary.contains("active")
                    HStack(spacing: 0) {
                        // The line opens People… where the room offers it and the connection is up (navigation
                        // note, section 8). Someone active keeps the warning colour under the pointer.
                        if offersPeople, store.connectionStatus.isConnected {
                            TextLink(title: summary, style: .caption,
                                     color: isActive ? SemanticColor.warning : SemanticColor.textTertiary,
                                     hoverColor: isActive ? nil : SemanticColor.textSecondary) {
                                Task { await workspace.showPeople(of: room.id) }
                            }
                            // copy:
                            .help("People")
                            // copy:
                            .macAccessibleAction(label: "People in \(room.name), \(summary)") {
                                Task { await workspace.showPeople(of: room.id) }
                            }
                        } else {
                            Text(summary)
                                .gallopText(.caption,
                                            color: isActive ? SemanticColor.warning : SemanticColor.textTertiary)
                        }
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
            do {
                let prompt = try await store.copyableConnectPrompt(for: room)
                let pasteboard = NSPasteboard.general
                pasteboard.clearContents()
                pasteboard.setString(prompt, forType: .string)
                hasCopiedEmptyRoomPrompt = true
            } catch is CancellationError {
                return
            } catch {
                store.errorMessage = error.localizedDescription
            }
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
            connectionStatus: store.connectionStatus
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

    /// A connected room with no messages: the message glyph and "No messages yet", centred in the feed above the
    /// composer, with Copy connect prompt below it.
    private var emptyRoom: some View {
        VStack(spacing: 0) {
            GallopIconView(icon: .message, fallbackSystemName: "bubble.left", size: 24)
                .foregroundStyle(SemanticColor.iconTertiary)
            // copy:
            Text("No messages yet")
                .gallopText(.h5, color: SemanticColor.textPrimary)
                .padding(.top, 10)
            emptyRoomPromptButton
                .padding(.top, 16)
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var emptyRoomPromptButton: some View {
        Button {
            copyConnectPrompt()
        } label: {
            // Reserve the wider label's width, so the pill does not resize when its label turns to Copied.
            ZStack {
                // copy:
                Text("Copy connect prompt").hidden()
                Text(hasCopiedEmptyRoomPrompt ? "Copied" : "Copy connect prompt")
            }
        }
        .buttonStyle(CowchatPillButtonStyle(prominent: true))
        .macAccessibleAction(label: "Copy connect prompt") {
            copyConnectPrompt()
        }
    }

    /// Offline or connecting with nothing loaded: the empty state's layout with the retry glyph, the connection's state
    /// as the title, and Reconnect while offline. `EmptyChatView`'s failed state without its caption or its
    /// troubleshooting text.
    private func unreachableRoom(_ reason: RoomPaneState.Unreachable) -> some View {
        VStack(spacing: 0) {
            GallopIconView(icon: .retry, fallbackSystemName: "arrow.clockwise", size: 24)
                .foregroundStyle(SemanticColor.iconTertiary)
            // copy:
            Text(reason == .connecting ? "Connecting…" : "Offline")
                .gallopText(.h5, color: SemanticColor.textPrimary)
                .padding(.top, 10)
            if reason == .offline {
                // copy:
                Button("Reconnect") { store.reconnect() }
                    .buttonStyle(CowchatPillButtonStyle(prominent: true))
                    .padding(.top, 16)
                    .macAccessibleAction(label: "Reconnect", action: store.reconnect)
            }
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
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

            ComposerInputRow(
                composer: store.composer,
                placeholder: "Message \(title)",
                isEnabled: isRoomWritable,
                isSendable: store.connectionStatus.isConnected && isRoomWritable,
                mentionMarks: mentionMarks,
                onSubmit: sendDraft,
                onCancel: {
                    withAnimation(.easeInOut(duration: 0.18)) { isComposerExpanded = false }
                }
            )
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity)
        .background(SemanticColor.surface500)
    }

    /// Sends the draft, with the live sessions it names after an @ as its mentions in an account room the person owns.
    private func sendDraft() {
        store.sendDraft(mentions: ownsRoom ? sessions.mentions(in: store.draft) : [])
    }

    private var isRoomWritable: Bool {
        !room.encrypted || store.unlockedRoomIDs.contains(room.id)
    }
}

/// The composer's field and send button. It alone observes the draft, so typing redraws this row and not the room
/// view, its message list or anything else that watches `ChatStore`.
private struct ComposerInputRow: View {
    @ObservedObject var composer: ComposerDraft
    let placeholder: String
    let isEnabled: Bool
    /// Whether the room can take a message now, before the draft's own text is considered.
    let isSendable: Bool
    let mentionMarks: MentionMarks
    let onSubmit: () -> Void
    let onCancel: () -> Void
    @State private var isFieldHovering = false

    private var canSend: Bool {
        isSendable && !composer.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        HStack(spacing: 8) {
            ComposerTextField(
                text: $composer.text,
                placeholder: placeholder,
                isEnabled: isEnabled,
                onSubmit: onSubmit,
                onCancel: onCancel,
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

            Button(action: onSubmit) {
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
                action: onSubmit
            )
        }
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
    /// The name over the message when the room names its sender itself, as account rooms do the person's own sessions.
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
    @EnvironmentObject private var workspace: WorkspaceStore

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
                        Button("New room") { workspace.presentNewRoom() }
                            .buttonStyle(CowchatPillButtonStyle(prominent: true))
                            .keyboardShortcut(.defaultAction)
                            .padding(.top, 6)
                            .macAccessibleAction(label: "Create room") { workspace.presentNewRoom() }
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
        case .agents: return "Connect Claude Code and Codex to Cowchat."
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
                    Text("Your sessions' rooms and rooms shared with friends. Only invited people and approved agents can join.")
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
