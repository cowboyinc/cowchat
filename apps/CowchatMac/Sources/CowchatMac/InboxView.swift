import SwiftUI

/// The Inbox page: the header with Mark all read, then one list of rows capped at the message column's width, open
/// invitations first and then everything else, newest first. Before the first poll answers it draws only its header.
struct InboxPage: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var inbox: InboxStore
    @ObservedObject var home: HomeSessions

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // copy:
            PageHeader(title: "Inbox") {
                Button("Mark all read") { inbox.markAllRead() }
                    .buttonStyle(CowchatPillButtonStyle())
                    .disabled(!inbox.hasUnreadMessages)
                    .macAccessibleAction(label: "Mark all read", isEnabled: inbox.hasUnreadMessages) {
                        inbox.markAllRead()
                    }
            }
            if inbox.isLoaded {
                if inbox.items.isEmpty {
                    // copy:
                    Text("You're all caught up.")
                        .gallopText(.bodyM, color: SemanticColor.textTertiary)
                        .padding(.leading, PageLayout.emptyLeading)
                        .padding(.top, PageLayout.emptyTop)
                } else {
                    list
                }
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .sheet(item: $inbox.joining) { item in
            InboxJoinSheet(item: item, inbox: inbox, home: home)
                .environmentObject(workspace)
        }
    }

    private var list: some View {
        ScrollView {
            TimelineView(.periodic(from: .now, by: 60)) { timeline in
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(inbox.items.enumerated()), id: \.element.id) { index, item in
                        if index > 0 { PageRowSeam() }
                        InboxRow(item: item, title: title(of: item), room: roomTitle(of: item),
                                 isUnread: inbox.isUnread(item), now: timeline.date,
                                 press: { press(item) },
                                 toggleRead: { toggleRead(item) },
                                 decline: item.isOpenInvitation ? { decline(item) } : nil)
                    }
                }
                .frame(maxWidth: PageLayout.listWidth, alignment: .leading)
                .padding(.top, 4)
                .padding(.bottom, 16)
            }
            .padding(.horizontal, PageLayout.listInset)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.automatic)
    }

    private func title(of item: InboxItem) -> String {
        item.title { id in home.sessions.first { $0.id == id }.map(home.name(of:)) }
    }

    // copy:
    /// The room as the sidebar shows it: "Home" for the account's Home.
    private func roomTitle(of item: InboxItem) -> String {
        item.server == .accountGlobal && item.roomID == home.roomID ? "Home" : item.roomName
    }

    /// A message opens its room; an open invitation opens the Join sheet and stays unread until it is answered.
    private func press(_ item: InboxItem) {
        if item.isOpenInvitation {
            inbox.joining = item
        } else {
            Task { await workspace.open(item) }
        }
    }

    private func toggleRead(_ item: InboxItem) {
        if inbox.isUnread(item) { inbox.markRead(item) } else { inbox.markUnread(item) }
    }

    private func decline(_ item: InboxItem) {
        Task { await workspace.decline(item) }
    }
}

/// One Inbox row, laid on the notification history's columns: 74 tall, 16 in at both sides, the unread dot centred in
/// a 20 box on the title line, 8, the title over the message line, then the Room column (160) and the When column
/// (64), each right-aligned and centred on the row's height, 16 apart. It is a button with the page row's hover and
/// press, and a menu that marks it read or unread and declines an open invitation.
struct InboxRow: View {
    let item: InboxItem
    let title: String
    let room: String
    let isUnread: Bool
    let now: Date
    let press: () -> Void
    let toggleRead: () -> Void
    let decline: (() -> Void)?

    static let height: CGFloat = 74
    static let roomWidth: CGFloat = 160
    static let whenWidth: CGFloat = 64
    static let titleMinWidth: CGFloat = 240

    var body: some View {
        Button(action: press) {
            HStack(alignment: .center, spacing: 16) {
                HStack(alignment: .top, spacing: 8) {
                    InboxUnreadDot(isUnread: isUnread)
                    VStack(alignment: .leading, spacing: 2) {
                        InboxTitleLine(title: title, isUnread: isUnread)
                        InboxMessageLine(message: item.message)
                    }
                }
                .frame(minWidth: 20 + 8 + Self.titleMinWidth, maxWidth: .infinity, alignment: .leading)
                Text(room)
                    .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(width: Self.roomWidth, alignment: .trailing)
                Text(when)
                    .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    .lineLimit(1)
                    .frame(width: Self.whenWidth, alignment: .trailing)
            }
            .padding(.horizontal, 16)
            .frame(height: Self.height)
        }
        .buttonStyle(PageRowButtonStyle())
        .contextMenu { InboxItemMenu(isUnread: isUnread, toggleRead: toggleRead, decline: decline) }
        .macAccessibleAction(label: accessibilityLabel, value: isUnread ? "Unread" : nil, action: press)
    }

    private var when: String { InboxTime.relative(item.at, now: now) }

    // copy:
    /// "Logan's sign-in-field, cowboy-junction, 39m ago, could you review…"
    private var accessibilityLabel: String {
        [title, room, when, item.message].filter { !$0.isEmpty }.joined(separator: ", ")
    }
}

/// The Inbox row's right-click menu: Mark as read or Mark as unread, and Decline on an open invitation.
struct InboxItemMenu: View {
    let isUnread: Bool
    let toggleRead: () -> Void
    let decline: (() -> Void)?

    var body: some View {
        // copy:
        Button(isUnread ? "Mark as read" : "Mark as unread", action: toggleRead)
        if let decline {
            // copy:
            Button("Decline", role: .destructive, action: decline)
        }
    }
}

/// The room row's unread dot, 7 across in Nugget/500, centred in a 20 box on the title line. A read row keeps the box
/// empty so every title starts on the same edge.
struct InboxUnreadDot: View {
    let isUnread: Bool

    var body: some View {
        ZStack {
            if isUnread {
                Circle()
                    .fill(Palette.nugget500)
                    .frame(width: 7, height: 7)
            }
        }
        .frame(width: 20, height: 20)
        .accessibilityHidden(true)
    }
}

/// Who sent an item, on one line in Text/Primary: Body M Strong while unread and Body M once read.
struct InboxTitleLine: View {
    let title: String
    let isUnread: Bool

    var body: some View {
        Text(title)
            .gallopText(isUnread ? .bodyMStrong : .bodyM, color: SemanticColor.textPrimary)
            .lineLimit(1)
            .truncationMode(.tail)
            .frame(height: 20)
    }
}

/// An item's one line of what it says, in Body S, Text/Tertiary.
struct InboxMessageLine: View {
    let message: String

    var body: some View {
        Text(message)
            .gallopText(.bodyS, color: SemanticColor.textTertiary)
            .lineLimit(1)
            .truncationMode(.tail)
            .frame(height: 20)
    }
}

/// The relative time the sidebar's rows show, "39m ago", for an item's date.
enum InboxTime {
    static func relative(_ date: Date, now: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: date).cowchatRelativeTime(relativeTo: now)
    }
}

/// Pressing an open invitation: the app's invitation acceptance as the Join sheet, with the person's live sessions or
/// none to join with, Decline at the left of its buttons, and Cancel and Join at the right.
struct InboxJoinSheet: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    let item: InboxItem
    @ObservedObject var inbox: InboxStore
    @ObservedObject var home: HomeSessions
    @State private var session: String?

    var body: some View {
        if let invitation = inbox.invitation(for: item) {
            SocialInvitationReview(
                invitation: invitation, sessions: home.sessions, sessionName: home.name(of:),
                selectedSession: $session, busy: workspace.social.busy, error: workspace.social.errorMessage,
                onCancel: { inbox.joining = nil },
                onDecline: {
                    Task {
                        if await workspace.decline(item) { inbox.joining = nil }
                    }
                },
                onAccept: {
                    let chosen = home.sessions.first { $0.id == session }
                    Task {
                        if await workspace.accept(item, with: chosen) { inbox.joining = nil }
                    }
                })
        }
    }
}

/// The menu bar's panel: the Inbox's row at the bell's measure, 12 above and below and 16 at the sides, with the time
/// riding the title's line at its end in Body S, Text/Tertiary, and no Room column. It shows the four newest unread
/// items, then a row that opens the Inbox.
struct InboxPanel: View {
    @ObservedObject var inbox: InboxStore
    @ObservedObject var home: HomeSessions
    let open: (InboxItem) -> Void
    let openInbox: () -> Void

    static let width: CGFloat = 400

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { timeline in
            VStack(alignment: .leading, spacing: 0) {
                let items = inbox.newestUnread
                if items.isEmpty {
                    // copy:
                    Text("You're all caught up.")
                        .gallopText(.bodyS, color: SemanticColor.textTertiary)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                } else {
                    ForEach(items) { item in
                        InboxPanelRow(item: item, title: title(of: item), now: timeline.date) { open(item) }
                    }
                }
                Button(action: openInbox) {
                    HStack(spacing: 2) {
                        // copy:
                        Text("See all")
                            .gallopText(.bodyM, color: SemanticColor.textSecondary)
                        Spacer(minLength: 0)
                        GallopIconView(icon: .chevronRightSmall, fallbackSystemName: "chevron.right", size: 16)
                            .foregroundStyle(SemanticColor.iconTertiary)
                    }
                    .padding(.horizontal, 16)
                    .frame(height: 44)
                }
                .buttonStyle(PageRowButtonStyle())
                .macAccessibleAction(label: "Open Inbox", action: openInbox)
            }
            .padding(8)
            .frame(width: Self.width)
        }
        .background(SemanticColor.surface500)
    }

    private func title(of item: InboxItem) -> String {
        item.title { id in home.sessions.first { $0.id == id }.map(home.name(of:)) }
    }
}

/// One item in the menu bar's panel: the dot box, 8, the title with the time at its end, and the message under it.
struct InboxPanelRow: View {
    let item: InboxItem
    let title: String
    let now: Date
    let press: () -> Void

    var body: some View {
        Button(action: press) {
            HStack(alignment: .top, spacing: 8) {
                InboxUnreadDot(isUnread: true)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 8) {
                        InboxTitleLine(title: title, isUnread: true)
                        Spacer(minLength: 0)
                        Text(when)
                            .gallopText(.bodyS, color: SemanticColor.textTertiary)
                            .lineLimit(1)
                            .fixedSize()
                    }
                    InboxMessageLine(message: item.message)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(PageRowButtonStyle())
        .macAccessibleAction(label: [title, when, item.message].joined(separator: ", "), value: "Unread", action: press)
    }

    private var when: String { InboxTime.relative(item.at, now: now) }
}
