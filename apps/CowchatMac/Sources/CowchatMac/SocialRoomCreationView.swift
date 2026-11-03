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
        return friends.filter { holds($0.account, query: query) }
    }

    /// Whether a person's name or handle holds what was typed, with or without its @.
    static func holds(_ account: SocialAccount, query: String) -> Bool {
        let query = SocialAPI.normalizedHandle(query)
        return !query.isEmpty && (account.displayName.localizedCaseInsensitiveContains(query)
            || account.handle.localizedCaseInsensitiveContains(query))
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
