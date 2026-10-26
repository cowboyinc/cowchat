import SwiftUI

struct SocialRoomDraft: Identifiable {
    let id = UUID()
    var name = ""
    var friendID: String?
    var agentIDs: Set<String> = []
    var proposedAgentIDs: Set<String> = []

    var requestBody: [String: Any] {
        var body: [String: Any] = [
            "name": name.trimmingCharacters(in: .whitespacesAndNewlines),
            "agent_ids": agentIDs.sorted(),
            "proposed_agent_ids": friendID == nil ? [] : proposedAgentIDs.sorted(),
        ]
        if let friendID { body["friend_id"] = friendID }
        return body
    }
}

struct SocialRoomCreationView: View {
    @ObservedObject var social: SocialStore
    @Environment(\.dismiss) private var dismiss
    @State private var draft: SocialRoomDraft
    private let onCreated: (String) -> Void

    init(social: SocialStore, draft: SocialRoomDraft = SocialRoomDraft(), onCreated: @escaping (String) -> Void = { _ in }) {
        self.social = social
        _draft = State(initialValue: draft)
        self.onCreated = onCreated
    }

    private var friend: SocialFriend? { social.friends.first { $0.id == draft.friendID } }
    private var friendSelectionLabel: String {
        guard let friend else { return "Only me" }
        return "\(friend.account.displayName) (@\(friend.account.handle))"
    }
    private var canCreate: Bool {
        !draft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
        (draft.friendID == nil || friend != nil) && !social.busy && social.phase == .signedIn
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
                        Text("Invite a friend (optional)").gallopText(.bodyMStrong)
                        Menu {
                            Button("Only me") { draft.friendID = nil }
                            ForEach(social.friends) { friend in
                                Button("\(friend.account.displayName) (@\(friend.account.handle))") {
                                    draft.friendID = friend.id
                                }
                            }
                        } label: {
                            HStack {
                                Text(friendSelectionLabel).gallopText(.bodyM)
                                Spacer()
                                GallopIconView(icon: .chevronDown, fallbackSystemName: "chevron.down")
                                    .frame(width: 18, height: 18)
                            }
                            .padding(.horizontal, 16)
                            .frame(maxWidth: .infinity, minHeight: 44)
                            .background(SemanticColor.textfieldDefault, in: Capsule())
                            .overlay { Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1) }
                        }
                        .menuStyle(.borderlessButton)
                        .frame(maxWidth: .infinity)
                        if draft.friendID != nil && friend == nil {
                            Text("Choose a current friend or select Only me.")
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
                    if let friend {
                        SocialAgentSelection(title: "Suggest their agents", agents: friend.agents, selection: $draft.proposedAgentIDs)
                        Text("Your friend must accept the invitation and approve their own agents. Accepted participants can read all prior room history.")
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
                Button(draft.friendID == nil ? "Create room" : "Create room & invite") {
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
        .onChange(of: draft.friendID) { _ in draft.proposedAgentIDs = [] }
        .onChange(of: social.phase) { if $0 != .signedIn { dismiss() } }
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
