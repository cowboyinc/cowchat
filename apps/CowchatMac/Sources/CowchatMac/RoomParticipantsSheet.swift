import SwiftUI

/// Who may use People… in a room's menu: rooms on the account's server, which signs in with a `csoc_` key, where the
/// API-key server's rooms offer Invites… instead.
enum RoomPeople {
    static func isOffered(isLocal: Bool, apiKey: String) -> Bool {
        !isLocal && apiKey.hasPrefix("csoc_")
    }
}

/// People… in an account room's menu: the room's people and agents, approving more of the person's agents, taking one
/// out, and leaving the room. It keeps Friends' participants sheet as it was until it has a design pass of its own.
struct RoomParticipantsSheet: View {
    @ObservedObject var social: SocialStore
    @Binding var participants: SocialParticipants?
    @State private var consentingAgents: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let room = participants {
                HStack { Text(room.name).gallopText(.h3); Spacer(); Button("Done") { participants = nil } }
                SocialConsentDetails(members: room.members, agents: room.agents, slack: room.slackConnected)
                ForEach(room.agents) { agent in
                    HStack {
                        Text(agent.name)
                        Spacer()
                        if agent.ownerId == social.account?.id || room.ownerId == social.account?.id {
                            Button("Remove agent") { Task { await social.perform { api in
                                let _: SocialAcknowledgement = try await api.social(["rooms", room.roomId, "agents", agent.id], method: "DELETE")
                                participants = try await api.social(["rooms", room.roomId, "participants"])
                            } } }
                        }
                    }
                }
                SocialAgentSelection(title: "Approve additional agents", agents: social.agents.filter { own in !room.agents.contains(where: { $0.id == own.id }) }, selection: $consentingAgents)
                Button("Add selected agents") { Task { await social.perform { api in
                    let _: SocialAcknowledgement = try await api.social(["rooms", room.roomId, "agents"], method: "POST", body: ["agent_ids": Array(consentingAgents)])
                    consentingAgents = []
                    participants = try await api.social(["rooms", room.roomId, "participants"])
                } } }.disabled(consentingAgents.isEmpty || social.busy)
                if room.ownerId != social.account?.id {
                    Button("Leave room and remove my agents", role: .destructive) { Task { await social.perform { api in
                        let _: SocialAcknowledgement = try await api.social(["rooms", room.roomId, "membership"], method: "DELETE")
                        participants = nil
                    } } }
                }
            }
        }.padding(24).frame(width: 570).background(SemanticColor.surface500)
        .buttonStyle(CowchatPillButtonStyle())
    }
}

struct SocialConsentDetails: View {
    let members: [SocialAccount]
    let agents: [SocialAgent]
    let slack: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("People: \(members.map { "@\($0.handle)" }.joined(separator: ", "))", systemImage: "person.2")
            Label("Agents: \(agents.map(\.name).joined(separator: ", "))", systemImage: "cpu")
            Text("Messages are unencrypted. Joining gives access to all prior room history. Approved agents can read and send messages in this room.")
            Text(slack ? "Slack is connected. Externally visible messages may be relayed to Slack." : "Slack sharing is not available for collaboration rooms yet.")
        }.gallopText(.caption, color: SemanticColor.textSecondary).gallopCard(cornerRadius: 14)
    }
}
