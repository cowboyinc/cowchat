import SwiftUI
import CowchatKit

/// A sheet with one section per signed-in account. With one account the section has no header.
private struct AccountSheet<Section: View>: View {
    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    let title: String
    let identifier: String
    /// The accounts to show, or nil for every signed-in connection's.
    let accounts: [AccountStore]?
    /// False when the screen is a tab, which has nothing to dismiss.
    var showsDone = true
    @ViewBuilder let section: (AccountStore) -> Section

    private var shown: [AccountStore] { accounts ?? store.accounts }

    var body: some View {
        NavigationStack {
            List {
                ForEach(shown, id: \.connectionID) { account in
                    if shown.count > 1 {
                        Text(store.connections.first { $0.id == account.connectionID }?.name ?? "")
                            .gallopText(.bodyS, color: SemanticColor.textTertiary)
                            .plainRow()
                    }
                    section(account)
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(SemanticColor.surface400.ignoresSafeArea())
            .accessibilityIdentifier(identifier)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if showsDone {
                    ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
                }
            }
            .refreshable { for account in shown { await account.refresh() } }
            .task { for account in shown { await account.refresh() } }
        }
    }
}

/// Shown in place of the empty line when a screen's last load failed.
private struct LoadFailedRow: View {
    var body: some View {
        Text("Could not load. Pull down to try again.")
            .gallopText(.bodyL, color: SemanticColor.textError)
            .plainRow()
            .accessibilityIdentifier("account.loadFailed")
    }
}

private extension View {
    func plainRow() -> some View {
        listRowBackground(Color.clear).listRowSeparator(.hidden)
    }

    func accountErrors(_ account: AccountStore) -> some View {
        alert(
            account.errorMessage ?? "",
            isPresented: Binding(get: { account.errorMessage != nil }, set: { if !$0 { account.errorMessage = nil } })
        ) {
            Button("OK", role: .cancel) {}
        }
    }
}

// MARK: Inbox

enum AccountSheetKind: String, Identifiable {
    case inbox, friends, team, profile
    var id: String { rawValue }
}

struct InboxView: View {
    var accounts: [AccountStore]?
    var showsDone = true
    let open: (RoomRef) -> Void

    var body: some View {
        AccountSheet(title: "Inbox", identifier: "inbox.list", accounts: accounts, showsDone: showsDone) { InboxSection(account: $0, open: open) }
    }
}

private struct InboxSection: View {
    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var account: AccountStore
    let open: (RoomRef) -> Void
    @State private var answering: InboxEntry?

    var body: some View {
        if account.failedParts.contains(.inbox) {
            LoadFailedRow()
        } else if account.inbox.isEmpty {
            Text("Nothing here yet.").gallopText(.bodyL, color: SemanticColor.textTertiary).plainRow()
        }
        if account.inbox.contains(where: { !$0.isOpenInvitation && account.isUnread($0) }) {
            Button("Mark all read") { account.markAllRead() }
                .buttonStyle(GallopLinkButtonStyle())
                .plainRow()
                .accessibilityIdentifier("inbox.markAllRead")
        }
        ForEach(account.inbox) { entry in
            Button { press(entry) } label: { InboxRow(entry: entry, isUnread: account.isUnread(entry)) }
                .buttonStyle(.plain)
                .plainRow()
                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                    if account.isUnread(entry) {
                        Button("Read") { account.markRead(entry) }
                    } else {
                        Button("Unread") { account.markUnread(entry) }
                    }
                }
                .accessibilityIdentifier("inbox.item")
        }
        .confirmationDialog(
            "Join \(answering?.roomName ?? "")?",
            isPresented: Binding(get: { answering != nil }, set: { if !$0 { answering = nil } }),
            titleVisibility: .visible,
            presenting: answering
        ) { entry in
            Button("Join") { Task { if await account.accept(entry) { await show(entry) } } }
            Button("Decline", role: .destructive) { Task { await account.decline(entry) } }
        }
        .accountErrors(account)
    }

    private func press(_ entry: InboxEntry) {
        if entry.isOpenInvitation {
            answering = entry
        } else {
            account.markRead(entry)
            Task { await show(entry) }
        }
    }

    private func show(_ entry: InboxEntry) async {
        let ref = RoomRef(connectionID: entry.connectionID ?? account.connectionID, roomID: entry.roomID)
        if !store.rooms.contains(where: { $0.ref == ref }) { await store.refreshRooms() }
        guard store.rooms.contains(where: { $0.ref == ref }) else { return }
        dismiss()
        open(ref)
    }
}

private struct InboxRow: View {
    let entry: InboxEntry
    let isUnread: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Circle()
                .fill(isUnread ? Palette.nugget500 : Color.clear)
                .frame(width: 8, height: 8)
                .padding(.top, 7)
                .accessibilityLabel(isUnread ? "Unread" : "")
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(entry.title)
                        .gallopText(isUnread ? .bodyLStrong : .bodyL, color: SemanticColor.textPrimary)
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    Text(GallopTime.short(entry.at)).gallopText(.caption, color: SemanticColor.textTertiary)
                }
                Text(entry.message).gallopText(.bodyM, color: SemanticColor.textSecondary).lineLimit(2)
                Text(entry.roomName).gallopText(.caption, color: SemanticColor.textTertiary).lineLimit(1)
            }
        }
        .padding(.vertical, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }
}

// MARK: Friends

struct FriendsView: View {
    var accounts: [AccountStore]?

    var body: some View {
        AccountSheet(title: "Friends", identifier: "friends.list", accounts: accounts) { FriendsSection(account: $0) }
    }
}

private struct FriendsSection: View {
    @ObservedObject var account: AccountStore
    @State private var handle = ""
    @State private var myLink: InviteToShare?
    @State private var isAdding = false

    // A pasted friend link works in the same field as a handle.
    private func add() async {
        let text = handle.trimmingCharacters(in: .whitespacesAndNewlines)
        let added: Bool
        if let url = URL(string: text), let route = FriendLinkRoute.parse(url) {
            added = await account.requestFriend(route)
        } else {
            added = await account.addFriend(handle: text)
        }
        if added { handle = "" }
    }

    var body: some View {
        HStack(spacing: 8) {
            TextField("Add by handle or link", text: $handle)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .gallopText(.bodyL, color: SemanticColor.textPrimary)
                .accessibilityIdentifier("friends.handle")
            Button("Add") {
                isAdding = true
                Task {
                    await add()
                    isAdding = false
                }
            }
            .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
            .disabled(isAdding || handle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .accessibilityIdentifier("friends.add")
        }
        .plainRow()
        // On a view that is always present: an alert on an empty view never shows.
        .accountErrors(account)
        Button {
            Task { if let url = await account.myFriendLink() { myLink = InviteToShare(token: url.absoluteString) } }
        } label: {
            Label("Share my friend link", systemImage: "square.and.arrow.up")
        }
        .buttonStyle(GallopButtonStyle(kind: .secondary))
        .plainRow()
        .accessibilityIdentifier("friends.shareLink")
        .sheet(item: $myLink) { ShareSheet(text: $0.token).presentationDetents([.medium, .large]) }
        if account.failedParts.contains(.friends) { LoadFailedRow() }
        if !account.incomingRequests.isEmpty {
            Text("Requests").gallopText(.bodyS, color: SemanticColor.textTertiary).plainRow()
            ForEach(account.incomingRequests) { request in
                HStack(spacing: 8) {
                    PersonLabel(account: request.from)
                    Spacer(minLength: 8)
                    Button("Decline") { Task { await account.answer(request, accept: false) } }
                        .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
                    Button("Accept") { Task { await account.answer(request, accept: true) } }
                        .buttonStyle(GallopButtonStyle(kind: .primary, fillsWidth: false))
                }
                .plainRow()
            }
        }
        if !account.sentRequests.isEmpty {
            Text("Sent").gallopText(.bodyS, color: SemanticColor.textTertiary).plainRow()
            ForEach(account.sentRequests) { PersonLabel(account: $0.to).plainRow() }
        }
        if !account.friends.isEmpty {
            Text(account.friends.count == 1 ? "1 friend" : "\(account.friends.count) friends")
                .gallopText(.bodyS, color: SemanticColor.textTertiary)
                .plainRow()
            ForEach(account.friends) { friend in
                PersonLabel(account: friend.account)
                    .plainRow()
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        Button("Remove") { Task { await account.removeFriend(friend) } }
                            .tint(SemanticColor.buttonPrimaryDestructiveDefault)
                    }
            }
        }
    }
}

/// A field with its name beside it, so two fields in a row can be told apart.
struct LabeledField<Field: View>: View {
    let label: String
    @ViewBuilder let field: Field

    var body: some View {
        HStack(spacing: 12) {
            Text(label)
                .gallopText(.bodyL, color: SemanticColor.textTertiary)
                .frame(width: 96, alignment: .leading)
            field.gallopText(.bodyL, color: SemanticColor.textPrimary)
        }
    }
}

private struct PersonLabel: View {
    let account: SocialAccount

    var body: some View {
        HStack(spacing: 10) {
            AgentAvatar(name: account.displayName, size: 32)
            VStack(alignment: .leading, spacing: 0) {
                Text(account.displayName).gallopText(.bodyL, color: SemanticColor.textPrimary).lineLimit(1)
                Text("@\(account.handle)").gallopText(.caption, color: SemanticColor.textTertiary).lineLimit(1)
            }
        }
    }
}

/// First sign-in on a new account: choose the name and handle other people see.
struct SetupSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var account: AccountStore
    @State private var displayName = ""
    @State private var handle = ""
    @State private var check = SocialHandleCheck.idle
    @State private var isSaving = false

    private var canContinue: Bool {
        !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && check.permits(SocialAPI.normalizedHandle(handle)) && !isSaving
    }

    var body: some View {
        NavigationStack {
            Form {
                TextField("Name", text: $displayName)
                    .gallopText(.bodyL, color: SemanticColor.textPrimary)
                    .gallopFormRow()
                    .accessibilityIdentifier("setup.name")
                TextField("Username", text: $handle)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .gallopText(.bodyL, color: SemanticColor.textPrimary)
                    .gallopFormRow()
                    .accessibilityIdentifier("setup.handle")
                Text(check.message)
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
                    .gallopFormRow()
                    .accessibilityIdentifier("setup.check")
            }
            .gallopForm()
            .navigationTitle("What should we call you?")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Continue") {
                        isSaving = true
                        Task {
                            if await account.establishSetup(displayName: displayName, handle: handle) { dismiss() }
                            isSaving = false
                        }
                    }
                    .disabled(!canContinue)
                    .accessibilityIdentifier("setup.continue")
                }
            }
            .task(id: handle) {
                let wanted = SocialAPI.normalizedHandle(handle)
                guard !wanted.isEmpty else { return check = .idle }
                check = .checking(wanted)
                try? await Task.sleep(for: .milliseconds(300))
                guard !Task.isCancelled else { return }
                let result = await account.handleCheck(wanted)
                if !Task.isCancelled { check = result }
            }
            .onAppear {
                if displayName.isEmpty { displayName = account.account?.displayName ?? "" }
                if handle.isEmpty { handle = account.account?.handle ?? "" }
            }
            .accountErrors(account)
        }
        .interactiveDismissDisabled()
    }
}

/// Presents setup for an account that needs it. One per account, since each watches its own.
struct SetupGate: View {
    @ObservedObject var account: AccountStore

    var body: some View {
        Color.clear.sheet(isPresented: .constant(account.needsSetup)) { SetupSheet(account: account) }
    }
}

/// A friend link the app was opened with: who it is, and a button to ask.
struct FriendLinkSheet: View {
    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    let route: FriendLinkRoute
    var accounts: [AccountStore]?

    @State private var preview: SocialFriendLinkPreview?
    @State private var isLoaded = false
    @State private var isSent = false
    @State private var isSending = false

    private var account: AccountStore? { (accounts ?? store.accounts).first { $0.serves(route) } }

    private var status: String? {
        guard let preview else { return isLoaded ? "This link could not be opened." : nil }
        switch preview.relationship {
        case .`self`: return "This is your own link."
        case .friends: return "You are already friends."
        case .outgoingPending: return "Request sent."
        case .incomingPending: return "They have asked you. Answer in Friends."
        case .none: return isSent ? "Request sent." : nil
        }
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 16) {
                if account == nil {
                    Text("Sign in to \(route.endpoint.host ?? "this server") to add this friend.")
                        .gallopText(.bodyL, color: SemanticColor.textSecondary)
                        .multilineTextAlignment(.center)
                }
                if let preview { PersonLabel(account: preview.account) }
                if let status {
                    Text(status).gallopText(.bodyM, color: SemanticColor.textTertiary).accessibilityIdentifier("friendLink.status")
                }
                if let account, preview?.relationship == SocialFriendRelationship.none, !isSent {
                    Button("Send friend request") {
                        isSending = true
                        Task {
                            isSent = await account.requestFriend(route)
                            isSending = false
                        }
                    }
                        .buttonStyle(GallopButtonStyle(kind: .primary))
                        .disabled(isSending)
                        .accessibilityIdentifier("friendLink.send")
                }
                Spacer()
            }
            .padding(24)
            .frame(maxWidth: .infinity)
            .background(SemanticColor.surface400.ignoresSafeArea())
            .navigationTitle("Add friend")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .task {
                preview = await account?.previewFriend(route)
                isLoaded = true
            }
        }
        .presentationDetents([.medium])
    }
}

/// The You tab: the person's profile, their connections, and notifications. The connections sit on the tab itself,
/// so it opens on something even with no account.
struct YouView: View {
    @EnvironmentObject private var store: ConnectionsStore

    var body: some View {
        NavigationStack {
            List {
                if !store.accounts.isEmpty {
                    Text("Profile").gallopText(.bodyS, color: SemanticColor.textTertiary).plainRow()
                }
                ForEach(store.accounts, id: \.connectionID) { ProfileSection(account: $0) }
                Text("Connections").gallopText(.bodyS, color: SemanticColor.textTertiary).plainRow()
                ConnectionRows()
                if !store.accounts.isEmpty {
                    Toggle("Show notifications", isOn: Binding(
                        get: { store.notificationsEnabled },
                        set: { wanted in Task { await store.setNotificationsEnabled(wanted) } }
                    ))
                    .gallopText(.bodyL, color: SemanticColor.textPrimary)
                    .plainRow()
                    .accessibilityIdentifier("notifications.toggle")
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(SemanticColor.surface400.ignoresSafeArea())
            .accessibilityIdentifier("you.list")
            .navigationTitle("You")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

/// The people and agents of an account room, with taking an agent out and adding the person's own.
struct AccountRoomPeople: View {
    @ObservedObject var account: AccountStore
    let roomID: String
    @State private var room: SocialParticipants?

    var body: some View {
        if let room {
            Text("People").gallopText(.bodyS, color: SemanticColor.textTertiary).plainRow()
            ForEach(room.members) { PersonLabel(account: $0).plainRow() }
            if !room.agents.isEmpty {
                Text("Agents").gallopText(.bodyS, color: SemanticColor.textTertiary).plainRow()
            }
            ForEach(room.agents) { agent in
                HStack(spacing: 10) {
                    AgentAvatar(name: agent.name, size: 32)
                    Text(agent.name).gallopText(.bodyL, color: SemanticColor.textPrimary).lineLimit(1)
                    Spacer(minLength: 8)
                    if account.canRemove(agent, from: room) {
                        Button("Remove") { Task { if await account.removeAgent(agent.id, roomID: roomID) { await load() } } }
                            .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
                            .accessibilityIdentifier("people.removeAgent")
                    }
                }
                .plainRow()
            }
            ForEach(account.agentsToAdd(to: room)) { agent in
                HStack(spacing: 10) {
                    AgentAvatar(name: agent.name, size: 32)
                    Text(agent.name).gallopText(.bodyL, color: SemanticColor.textTertiary).lineLimit(1)
                    Spacer(minLength: 8)
                    Button("Add") { Task { if await account.addAgents([agent.id], roomID: roomID) { await load() } } }
                        .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
                        .accessibilityIdentifier("people.addAgent")
                }
                .plainRow()
            }
        }
        Color.clear.frame(height: 0).plainRow()
            .task { await load() }
            .accountErrors(account)
    }

    private func load() async {
        if let loaded = await account.participants(roomID: roomID) { room = loaded }
    }
}

/// Invites friends to a room the person owns.
struct AddPersonSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var account: AccountStore
    let roomID: String
    @State private var invited: Set<String> = []
    @State private var inviting: Set<String> = []

    var body: some View {
        NavigationStack {
            List {
                if account.friends.isEmpty {
                    Text("No friends yet.").gallopText(.bodyL, color: SemanticColor.textTertiary).plainRow()
                }
                ForEach(account.friends) { friend in
                    HStack(spacing: 8) {
                        PersonLabel(account: friend.account)
                        Spacer(minLength: 8)
                        Button(invited.contains(friend.id) ? "Invited" : "Invite") {
                            inviting.insert(friend.id)
                            Task {
                                if await account.invite(friendID: friend.id, roomID: roomID) { invited.insert(friend.id) }
                                inviting.remove(friend.id)
                            }
                        }
                        .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
                        .disabled(invited.contains(friend.id) || inviting.contains(friend.id))
                    }
                    .plainRow()
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(SemanticColor.surface400.ignoresSafeArea())
            .navigationTitle("Add person")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .task { await account.refresh() }
            .accountErrors(account)
        }
        .presentationDetents([.medium, .large])
    }
}

// MARK: People

/// The People tab: friends, then the team's work.
struct PeopleView: View {
    let open: (RoomRef) -> Void

    var body: some View {
        AccountSheet(title: "People", identifier: "people.tab", accounts: nil, showsDone: false) { account in
            Text("Friends").gallopText(.bodyS, color: SemanticColor.textTertiary).plainRow()
            FriendsSection(account: account)
            Text("Team").gallopText(.bodyS, color: SemanticColor.textTertiary).plainRow()
            TeamSection(account: account, open: open)
        }
    }
}

// MARK: Team

struct TeamView: View {
    var accounts: [AccountStore]?
    let open: (RoomRef) -> Void

    var body: some View {
        AccountSheet(title: "Team", identifier: "team.list", accounts: accounts) { TeamSection(account: $0, open: open) }
    }
}

private struct TeamSection: View {
    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @ObservedObject var account: AccountStore
    let open: (RoomRef) -> Void

    var body: some View {
        if account.failedParts.contains(.team) {
            LoadFailedRow()
        } else if account.team.isEmpty {
            Text("No one is sharing work.").gallopText(.bodyL, color: SemanticColor.textTertiary).plainRow()
        }
        ForEach(account.team) { person in
            Text(person.account.displayName).gallopText(.bodyLStrong, color: SemanticColor.textPrimary).plainRow()
            ForEach(person.sessions) { session in
                Button { press(session) } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text(session.name).gallopText(.bodyL, color: SemanticColor.textPrimary).lineLimit(1)
                            Spacer(minLength: 8)
                            Text(session.stateLabel).gallopText(.caption, color: SemanticColor.textTertiary)
                        }
                        if session.name != session.folder {
                            Text(session.folder).gallopText(.caption, color: SemanticColor.textTertiary).lineLimit(1)
                        }
                        if let pullRequest = session.pullRequest {
                            Text([pullRequest.name, pullRequest.checksLabel].compactMap { $0 }.joined(separator: " · "))
                                .gallopText(.caption, color: SemanticColor.textSecondary)
                                .lineLimit(1)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .plainRow()
                .accessibilityIdentifier("team.session")
            }
        }
    }

    // A session's room opens when you are in it; otherwise its pull request does.
    private func press(_ session: TeamSession) {
        if let room = session.room {
            let ref = RoomRef(connectionID: account.connectionID, roomID: room.roomId)
            if store.rooms.contains(where: { $0.ref == ref }) {
                dismiss()
                open(ref)
                return
            }
        }
        if let url = session.pullRequest?.url { openURL(url) }
    }
}

// MARK: Profile

struct ProfileView: View {
    var accounts: [AccountStore]?

    var body: some View {
        AccountSheet(title: "Profile", identifier: "profile.list", accounts: accounts) { ProfileSection(account: $0) }
    }
}

private struct ProfileSection: View {
    @ObservedObject var account: AccountStore
    @State private var displayName = ""
    @State private var handle = ""
    @State private var loadedFor: SocialAccount?

    private var isChanged: Bool {
        displayName.trimmingCharacters(in: .whitespacesAndNewlines) != account.account?.displayName
            || SocialAPI.normalizedHandle(handle) != account.account?.handle
    }

    var body: some View {
        Group {
            LabeledField(label: "Name") {
                TextField("Name", text: $displayName)
                    .accessibilityIdentifier("profile.name")
            }
            LabeledField(label: "Username") {
                TextField("Username", text: $handle)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("profile.handle")
            }
            Button("Save") { Task { await account.saveProfile(displayName: displayName, handle: handle) } }
                .buttonStyle(GallopButtonStyle(kind: .primary))
                .disabled(!isChanged || displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityIdentifier("profile.save")
        }
        .plainRow()
        .onChange(of: account.account, initial: true) {
            guard account.account != loadedFor, let loaded = account.account else { return }
            loadedFor = loaded
            displayName = loaded.displayName
            handle = loaded.handle
        }
        .accountErrors(account)
    }
}
