import SwiftUI

struct SocialRoomDraft: Identifiable {
    let id = UUID()
    var name = ""
    /// The friends to invite, in the order they were picked. Room creation carries the first; the room's invitations
    /// route invites the rest once it is made (Chad's plan, task 4).
    var friendIDs: [String] = []
    var agentIDs: Set<String> = []
    /// The friends' agents suggested for the room, across every friend picked.
    var proposedAgentIDs: Set<String> = []
    /// The company room the room is made inside: the team chosen in the sidebar's team row, or none.
    var parentID: String?

    /// The first friend picked, whom room creation invites.
    var friendID: String? {
        get { friendIDs.first }
        set { friendIDs = newValue.map { [$0] } ?? [] }
    }

    var requestBody: [String: Any] {
        var body: [String: Any] = [
            "name": name.trimmingCharacters(in: .whitespacesAndNewlines),
            "agent_ids": agentIDs.sorted(),
            "proposed_agent_ids": friendID == nil ? [] : proposedAgentIDs.sorted(),
        ]
        if let friendID { body["friend_id"] = friendID }
        if let parentID { body["parent_id"] = parentID }
        return body
    }

    /// Room creation's body when more than one friend is picked, suggesting only the first friend's agents.
    func requestBody(inviting first: SocialFriend) -> [String: Any] {
        var body = requestBody
        body["proposed_agent_ids"] = proposals(for: first)
        return body
    }

    /// The body that invites one more friend to the made room, with the agents of theirs suggested.
    func invitationBody(for friend: SocialFriend) -> [String: Any] {
        ["friend_id": friend.id, "proposed_agent_ids": proposals(for: friend)]
    }

    private func proposals(for friend: SocialFriend) -> [String] {
        proposedAgentIDs.intersection(friend.agents.map(\.id)).sorted()
    }
}

/// New room's friend search: the account's friends whose name or handle holds what was typed, in the order the server
/// lists them. An empty search matches none, so the list shows only what was picked until something is typed.
enum FriendSearch {
    static func matches(_ friends: [SocialFriend], query: String) -> [SocialFriend] {
        let query = SocialAPI.normalizedHandle(query)
        guard !query.isEmpty else { return [] }
        return friends.filter {
            $0.account.displayName.localizedCaseInsensitiveContains(query)
                || $0.account.handle.localizedCaseInsensitiveContains(query)
        }
    }
}

struct SocialRoomCreationView: View {
    @ObservedObject var social: SocialStore
    @Environment(\.dismiss) private var dismiss
    @State private var draft: SocialRoomDraft
    @State private var friendQuery = ""
    private let onCreated: (String) -> Void

    init(social: SocialStore, draft: SocialRoomDraft = SocialRoomDraft(), onCreated: @escaping (String) -> Void = { _ in }) {
        self.social = social
        _draft = State(initialValue: draft)
        self.onCreated = onCreated
    }

    /// The friends picked, as the account's friends list has them now.
    private var pickedFriends: [SocialFriend] {
        draft.friendIDs.compactMap { id in social.friends.first { $0.id == id } }
    }
    /// The friend rows under the search field: the friends picked, then those the search matches.
    private var friendRows: [SocialFriend] {
        pickedFriends + FriendSearch.matches(social.friends, query: friendQuery).filter { !draft.friendIDs.contains($0.id) }
    }
    private var canCreate: Bool {
        !draft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
        pickedFriends.count == draft.friendIDs.count && !social.busy && social.phase == .signedIn
    }

    var body: some View {
        VStack(spacing: 0) {
            CowchatSheetHeader(title: "Create room", subtitle: "Private global room on \(social.endpoint.host ?? "your Cowchat server").") { dismiss() }
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Room name").gallopText(.bodyMStrong)
                        TextField("e.g. Release planning", text: $draft.name)
                            .textFieldStyle(.roundedBorder).accessibilityLabel("Room name")
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        // copy:
                        Text("Invite friends (optional)").gallopText(.bodyMStrong)
                        // Friends' add-by-handle field. Return picks the first match.
                        TextField("Search friends", text: $friendQuery)
                            .textFieldStyle(.roundedBorder)
                            .onSubmit(pickFirstMatch)
                            .accessibilityLabel("Search friends")
                        ForEach(friendRows) { friend in
                            SocialFriendChoice(friend: friend, isPicked: draft.friendIDs.contains(friend.id)) {
                                toggle(friend)
                            }
                        }
                        if !friendQuery.trimmingCharacters(in: .whitespaces).isEmpty,
                           FriendSearch.matches(social.friends, query: friendQuery).isEmpty {
                            // copy:
                            Text("No friends found").gallopText(.bodyS, color: SemanticColor.textSecondary)
                        }
                        if pickedFriends.count != draft.friendIDs.count {
                            // copy:
                            Text("Choose current friends, or take out the others.")
                                .gallopText(.bodyS, color: SemanticColor.notificationErrorTextDefault)
                        } else if social.friends.isEmpty {
                            Text("You can create a room without adding a friend.")
                                .gallopText(.bodyS, color: SemanticColor.textSecondary)
                        }
                    }
                    SocialAgentSelection(title: "Approve your agents", agents: social.agents, selection: $draft.agentIDs)
                    if social.agents.isEmpty {
                        Text("No agents yet. You can approve your agents later from Participants.")
                            .gallopText(.caption, color: SemanticColor.textSecondary)
                    }
                    if !pickedFriends.isEmpty {
                        ForEach(pickedFriends.filter { !$0.agents.isEmpty }) { friend in
                            // copy: with more than one friend picked, each list names whose agents it holds.
                            SocialAgentSelection(title: pickedFriends.count == 1 ? "Suggest their agents"
                                                 : "Suggest \(friend.account.displayName)’s agents",
                                                 agents: friend.agents, selection: $draft.proposedAgentIDs)
                        }
                        // copy: the plural for more than one friend.
                        Text(pickedFriends.count == 1 ? "Your friend must accept the invitation and approve their own agents. Accepted participants can read all prior room history." : "Your friends must accept the invitation and approve their own agents. Accepted participants can read all prior room history.")
                            .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    } else {
                        Text("Only you and the agents you approve can access this room.")
                            .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    }
                    Text("Messages are unencrypted. Creating a room does not wake agents or authorize tool use.")
                        .gallopText(.caption, color: SemanticColor.textSecondary)
                    if let error = social.errorMessage {
                        Text(error).gallopText(.bodyS, color: SemanticColor.notificationErrorTextDefault)
                            .textSelection(.enabled)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 28).padding(.bottom, 20)
            }
            HStack {
                Button("Cancel") { dismiss() }.buttonStyle(CowchatPillButtonStyle())
                Spacer()
                Button(draft.friendIDs.isEmpty ? "Create room" : "Create room & invite") {
                    Task {
                        if let roomID = await social.createRoom(draft) { onCreated(roomID); dismiss() }
                    }
                }
                .buttonStyle(CowchatPillButtonStyle(prominent: true))
                .disabled(!canCreate)
                .keyboardShortcut(.defaultAction)
            }.padding(28)
        }
        .frame(width: 600, height: 620)
        .background(SemanticColor.surface600)
        .task { await social.refresh() }
        .onChange(of: draft.friendIDs) { _ in
            // Suggestions stay only for friends still picked.
            draft.proposedAgentIDs.formIntersection(pickedFriends.flatMap { $0.agents.map(\.id) })
        }
        .onChange(of: social.phase) { if $0 != .signedIn { dismiss() } }
    }
}

extension SocialRoomCreationView {
    private func toggle(_ friend: SocialFriend) {
        if let index = draft.friendIDs.firstIndex(of: friend.id) { draft.friendIDs.remove(at: index) }
        else { draft.friendIDs.append(friend.id) }
    }

    private func pickFirstMatch() {
        guard let first = FriendSearch.matches(social.friends, query: friendQuery)
            .first(where: { !draft.friendIDs.contains($0.id) }) else { return }
        draft.friendIDs.append(first.id)
        friendQuery = ""
    }
}

/// One friend under New room's search, drawn as the agent choices below it are: a check box, the friend's name and
/// handle, and the row filled while picked.
struct SocialFriendChoice: View {
    let friend: SocialFriend
    let isPicked: Bool
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 10) {
                Image(systemName: isPicked ? "checkmark.square.fill" : "square")
                    .font(.system(size: 20, weight: .medium))
                    .foregroundStyle(isPicked ? SemanticColor.buttonPrimaryIconDefault : SemanticColor.iconSecondary)
                Text(friend.account.displayName).gallopText(.bodyM).lineLimit(1)
                Text("@\(friend.account.handle)").gallopText(.bodyS, color: SemanticColor.textTertiary).lineLimit(1)
                Spacer()
            }
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .background(isPicked ? SemanticColor.surfaceGlassOnDefault : SemanticColor.textfieldDefault, in: Capsule())
            .overlay { Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1) }
        }
        .buttonStyle(.plain)
        .macAccessibleAction(label: friend.account.displayName, value: isPicked ? "selected" : nil, action: toggle)
    }
}

struct SocialAgentSelection: View {
    let title: String
    let agents: [SocialAgent]
    @Binding var selection: Set<String>

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).gallopText(.bodySStrong)
            ForEach(agents) { agent in
                Button {
                    if selection.contains(agent.id) { selection.remove(agent.id) } else { selection.insert(agent.id) }
                } label: {
                    HStack(spacing: 10) {
                        Image(systemName: selection.contains(agent.id) ? "checkmark.square.fill" : "square")
                            .font(.system(size: 20, weight: .medium))
                            .foregroundStyle(selection.contains(agent.id) ? SemanticColor.buttonPrimaryIconDefault : SemanticColor.iconSecondary)
                        Text(agent.name).gallopText(.bodyM)
                        Spacer()
                    }
                    .padding(.horizontal, 14)
                    .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                    .background(selection.contains(agent.id) ? SemanticColor.surfaceGlassOnDefault : SemanticColor.textfieldDefault, in: Capsule())
                    .overlay { Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1) }
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selection.contains(agent.id) ? [.isSelected] : [])
            }
        }
    }
}
