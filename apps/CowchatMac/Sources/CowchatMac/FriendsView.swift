import AppKit
import SwiftUI

/// What the Friends page makes of the account's friends and pending requests: Requests (sent to the person), Sent (sent
/// by them, which the server can't withdraw) and Friends, in that order, each drawn only when it holds a row.
enum FriendsList {
    enum Group: Equatable {
        case requests, sent, friends

        // copy:
        var title: String {
            switch self {
            case .requests: return "Requests"
            case .sent: return "Sent"
            case .friends: return "Friends"
            }
        }
    }

    /// The pending requests sent to the person.
    static func incoming(_ requests: [SocialFriendRequest], viewer: String?) -> [SocialFriendRequest] {
        requests.filter { $0.status == "pending" && $0.to.id == viewer && $0.from.id != viewer }
    }

    /// The pending requests the person sent.
    static func sent(_ requests: [SocialFriendRequest], viewer: String?) -> [SocialFriendRequest] {
        requests.filter { $0.status == "pending" && $0.from.id == viewer && $0.to.id != viewer }
    }

    /// The groups the page draws, in order, leaving out an empty one.
    static func groups(requests: [SocialFriendRequest], friends: [SocialFriend], viewer: String?) -> [Group] {
        [
            incoming(requests, viewer: viewer).isEmpty ? nil : Group.requests,
            sent(requests, viewer: viewer).isEmpty ? nil : .sent,
            friends.isEmpty ? nil : .friends,
        ].compactMap { $0 }
    }

    /// What the page shows under its header.
    enum Content: Equatable {
        /// "Sign in to add friends." with Sign in, and no accessory.
        case signedOut
        /// "No friends yet." with the person's handle.
        case empty
        case groups([Group])
    }

    static func content(signedIn: Bool, requests: [SocialFriendRequest], friends: [SocialFriend],
                        viewer: String?) -> Content {
        guard signedIn else { return .signedOut }
        let groups = groups(requests: requests, friends: friends, viewer: viewer)
        return groups.isEmpty ? .empty : .groups(groups)
    }

    // copy:
    /// The header's count, "12 friends", or nothing at zero.
    static func countLine(_ count: Int) -> String? {
        count == 0 ? nil : "\(count) \(count == 1 ? "friend" : "friends")"
    }

    /// Copy link: the person's friend link on the pasteboard.
    static func copy(_ link: SocialFriendLink, to pasteboard: NSPasteboard = .general) {
        pasteboard.clearContents()
        pasteboard.setString(link.url.absoluteString, forType: .string)
    }
}

/// The Friends page: the header with the handle field, Add and Copy link, then Requests, Sent and Friends in one column
/// capped at 760. A friend's row opens New room with that friend, and its ⋯ holds Remove friend. Signed out, it says to
/// sign in.
struct FriendsPage: View {
    @ObservedObject var social: SocialStore
    /// Opens New room with the friend picked.
    let newRoom: (SocialFriend) -> Void
    /// Opens the signed-out page.
    let signIn: () -> Void
    @State private var handle = ""
    @State private var addError: String?
    @State private var copied = false
    @FocusState private var isHandleFocused: Bool
    /// The rows' width with their insets, which the header takes.
    @State private var rowsWidth: CGFloat?
    /// The widest end column, which every row takes so the handles line up across the groups.
    @State private var endWidth: CGFloat = 0

    private var isSignedIn: Bool { social.phase == .signedIn && social.savedSession != nil }
    private var viewer: String? { social.account?.id }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // copy:
            PageHeader(title: "Friends", caption: isSignedIn ? FriendsList.countLine(social.friends.count) : nil,
                       rowsWidth: rowsWidth) {
                if isSignedIn { accessory }
            }
            if let addError {
                Text(addError)
                    .gallopText(.bodyS, color: SemanticColor.textError)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .padding(.leading, 20)
                    .padding(.top, 8)
            }
            switch FriendsList.content(signedIn: isSignedIn, requests: social.requests, friends: social.friends,
                                       viewer: viewer) {
            case .signedOut: signedOut
            case .empty: empty
            case .groups(let groups): list(groups)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onPreferenceChange(PageRowsWidthKey.self) { rowsWidth = $0 }
        .onPreferenceChange(FriendsEndWidthKey.self) { endWidth = $0 }
        .onChange(of: handle) { _ in addError = nil }
        .task(id: social.savedSession?.scopeId) {
            guard isSignedIn else { return }
            await social.refresh()
            await social.loadFriendLink()
        }
    }

    /// The handle field with Add, then Copy link, three 36 controls 8 apart.
    private var accessory: some View {
        HStack(spacing: 8) {
            // copy:
            SearchCapsule(placeholder: "Add by handle", text: $handle, isFocused: $isHandleFocused,
                          clearLabel: "Clear handle", width: 200, leading: "@", onSubmit: add)
            // copy:
            Button("Add", action: add)
                .buttonStyle(CowchatPillButtonStyle())
                .disabled(!canAdd)
                .macAccessibleAction(label: "Add friend", isEnabled: canAdd, action: add)
            if social.friendLinksSupported {
                Button(action: copyLink) {
                    // The wider label holds the width, so the button does not move when it reads Copied.
                    ZStack {
                        // copy:
                        Text("Copy link").hidden()
                        Text(copied ? "Copied" : "Copy link")
                    }
                }
                .buttonStyle(CowchatPillButtonStyle())
                .disabled(social.friendLinkState == .loading)
                .macAccessibleAction(label: "Copy link", action: copyLink)
            }
        }
    }

    private var canAdd: Bool {
        !handle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !social.busy
    }

    private func add() {
        guard canAdd else { return }
        let typed = handle
        Task {
            if await social.requestFriend(handle: typed) {
                if handle == typed { handle = "" }
            } else {
                addError = social.errorMessage
            }
        }
    }

    private func copyLink() {
        Task {
            if social.friendLink == nil { await social.loadFriendLink() }
            guard let link = social.friendLink else { return }
            FriendsList.copy(link)
            copied = true
            try? await Task.sleep(for: .seconds(2))
            copied = false
        }
    }

    private var empty: some View {
        // copy:
        Text("No friends yet. Your handle is @\(social.account?.handle ?? "").")
            .gallopText(.bodyM, color: SemanticColor.textTertiary)
            .padding(.leading, PageLayout.emptyLeading)
            .padding(.top, PageLayout.emptyTop)
    }

    private var signedOut: some View {
        VStack(alignment: .leading, spacing: 12) {
            // copy:
            Text("Sign in to add friends.")
                .gallopText(.bodyM, color: SemanticColor.textTertiary)
            // copy:
            Button("Sign in", action: signIn)
                .buttonStyle(CowchatPillButtonStyle(prominent: true))
                .macAccessibleAction(label: "Sign in", action: signIn)
        }
        .padding(.leading, PageLayout.emptyLeading)
        .padding(.top, PageLayout.emptyTop)
    }

    private func list(_ groups: [FriendsList.Group]) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                ForEach(groups, id: \.title) { group in
                    VStack(alignment: .leading, spacing: 0) {
                        TeamGroupHead(title: group.title)
                        rows(of: group)
                    }
                }
            }
            .frame(maxWidth: PageLayout.listWidth, alignment: .leading)
            .padding(.top, 4)
            .padding(.bottom, 16)
            .padding(.horizontal, PageLayout.listInset)
            .reportsPageRowsWidth()
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.automatic)
    }

    @ViewBuilder private func rows(of group: FriendsList.Group) -> some View {
        switch group {
        case .requests:
            ForEach(Array(FriendsList.incoming(social.requests, viewer: viewer).enumerated()), id: \.element.id) {
                index, request in
                if index > 0 { PageRowSeam() }
                FriendRow(person: request.from, endWidth: endWidth) {
                    FriendRequestButtons(name: request.from.displayName, busy: social.busy) { accept in
                        Task { await social.answerFriendRequest(request.id, accept: accept) }
                    }
                }
            }
        case .sent:
            ForEach(Array(FriendsList.sent(social.requests, viewer: viewer).enumerated()), id: \.element.id) {
                index, request in
                if index > 0 { PageRowSeam() }
                FriendRow(person: request.to, endWidth: endWidth) { EmptyView() }
            }
        case .friends:
            ForEach(Array(social.friends.enumerated()), id: \.element.id) { index, friend in
                if index > 0 { PageRowSeam() }
                FriendButtonRow(friend: friend, endWidth: endWidth, newRoom: { newRoom(friend) }, remove: {
                    Task { await social.removeFriend(friend.id) }
                })
            }
        }
    }
}

extension Notification.Name {
    /// Shows the Friends page, for a sheet outside the window's content, such as a friend link's confirmation.
    static let cowchatShowFriends = Notification.Name("CowchatMac.showFriends")
}

/// The widest end column among the Friends page's rows.
struct FriendsEndWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

/// One row on the Friends page, as production's record list draws one: 52 tall, 16 at the sides, the 24 avatar, 11,
/// the name in Body M (at least 160), 16, the handle in Body S Text/Tertiary (160, right-aligned), 16, and the end
/// column, as wide as the widest row's, so the handles line up across the groups.
struct FriendRow<End: View>: View {
    let person: SocialAccount
    let endWidth: CGFloat
    @ViewBuilder let end: () -> End

    static var height: CGFloat { 52 }
    static var nameMinWidth: CGFloat { 160 }
    static var handleWidth: CGFloat { 160 }

    var body: some View {
        HStack(spacing: 0) {
            SocialPersonAvatar(name: person.displayName, size: 24, letters: 1)
            Spacer().frame(width: 11)
            Text(person.displayName)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(minWidth: Self.nameMinWidth, maxWidth: .infinity, alignment: .leading)
            Spacer().frame(width: 16)
            Text(verbatim: "@\(person.handle)")
                .gallopText(.bodyS, color: SemanticColor.textTertiary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(width: Self.handleWidth, alignment: .trailing)
            Spacer().frame(width: 16)
            // The column keeps its width when the end is empty, as a Sent row's is.
            ZStack(alignment: .trailing) {
                Color.clear.frame(width: endWidth, height: 0)
                end()
                    .fixedSize()
                    .background(GeometryReader {
                        Color.clear.preference(key: FriendsEndWidthKey.self, value: $0.size.width)
                    })
            }
        }
        .padding(.horizontal, 16)
        .frame(height: Self.height)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A request's end: Decline (secondary), 8, Accept (prominent). The row itself is not a button.
private struct FriendRequestButtons: View {
    let name: String
    let busy: Bool
    let answer: (Bool) -> Void

    var body: some View {
        HStack(spacing: 8) {
            // copy:
            Button("Decline") { answer(false) }
                .buttonStyle(CowchatPillButtonStyle())
                .macAccessibleAction(label: "Decline \(name)", isEnabled: !busy) { answer(false) }
            // copy:
            Button("Accept") { answer(true) }
                .buttonStyle(CowchatPillButtonStyle(prominent: true))
                .macAccessibleAction(label: "Accept \(name)", isEnabled: !busy) { answer(true) }
        }
        .disabled(busy)
    }
}

/// A friend's row: a button that opens New room with them, the page rows' hover and press, and a 24 ⋯ that shows on
/// hover. The ⋯ and right-click give New room and Remove friend.
private struct FriendButtonRow: View {
    let friend: SocialFriend
    let endWidth: CGFloat
    let newRoom: () -> Void
    let remove: () -> Void
    @State private var isHovering = false

    var body: some View {
        ZStack(alignment: .trailing) {
            Button(action: newRoom) {
                FriendRow(person: friend.account, endWidth: endWidth) {
                    Color.clear.frame(width: 24, height: 24)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(PageRowButtonStyle())
            // copy:
            .help("New room with \(friend.account.displayName)")
            .macAccessibleAction(label: "\(friend.account.displayName), @\(friend.account.handle)", action: newRoom)
            // copy:
            PageRowMoreMenu(isVisible: isHovering, accessibilityLabel: "Actions for \(friend.account.displayName)") {
                items
            }
            .padding(.trailing, 16)
        }
        .contextMenu { items }
        .onHover { isHovering = $0 }
    }

    @ViewBuilder private var items: some View {
        // copy:
        Button("New room", action: newRoom)
        // copy:
        Button("Remove friend", role: .destructive, action: remove)
    }
}

/// A person's round mark: their initials on Surface/Glass/On/Default with a 0.5 Border/Default edge. At 40 it holds two
/// initials in Body M Strong; smaller, such as a Team page's 24, it holds `letters` initials at 0.42 of its size, bold
/// and rounded, as the message rows' 24 avatar sizes its letter.
struct SocialPersonAvatar: View {
    let name: String
    var size: CGFloat = 40
    var letters = 2

    var body: some View {
        let initials = Text(String(name.split(whereSeparator: { $0.isWhitespace }).prefix(letters).compactMap(\.first)))
            .textCase(.uppercase)
        Group {
            if size >= 40 {
                initials.gallopText(.bodyMStrong, color: SemanticColor.surfaceGlassOnTextDefault)
            } else {
                initials
                    .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
                    .foregroundStyle(SemanticColor.surfaceGlassOnTextDefault)
            }
        }
        .frame(width: size, height: size)
        .background(SemanticColor.surfaceGlassOnDefault, in: Circle())
        .overlay(Circle().stroke(SemanticColor.borderDefault, lineWidth: 0.5))
        .accessibilityHidden(true)
    }
}
