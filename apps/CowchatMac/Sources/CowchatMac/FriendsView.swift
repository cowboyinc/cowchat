import AppKit
import SwiftUI

struct FriendsView: View {
    @ObservedObject var social: SocialStore
    @Environment(\.dismiss) private var dismiss
    @State private var page = SocialSection.friends
    @State private var isEditingProfile = false
    @State private var profileHandle = ""
    @State private var profileName = ""
    @State private var handle = ""
    @State private var agentName = ""
    @State private var roomName = ""
    @State private var friendID = ""
    @State private var selectedAgents: Set<String> = []
    @State private var proposedAgents: Set<String> = []
    @State private var participants: SocialParticipants?
    @State private var invitation: SocialRoomInvitation?
    @State private var consentingAgents: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(spacing: 14) {
                SocialIconBadge(symbol: "person.2.fill", prominent: true)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Friends & agents").gallopText(.h3)
                    Text("Bring your people. Choose your crew.").gallopText(.bodyS, color: SemanticColor.textSecondary)
                }
                Spacer()
                Button { dismiss() } label: {
                    GallopIconView(icon: .dismiss, fallbackSystemName: "xmark")
                        .frame(width: 32, height: 32)
                        .background(SemanticColor.buttonSecondaryDefault, in: Circle())
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.cancelAction)
                .accessibilityLabel("Close Friends & agents")
                .help("Close")
            }
            if let error = social.errorMessage {
                Text(error).gallopText(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            if social.phase == .signedIn && !social.serverNeedsUpdate {
                HStack(spacing: 12) {
                    SocialPersonAvatar(name: social.account?.displayName ?? "Account")
                    VStack(alignment: .leading, spacing: 2) {
                        Text(social.account?.displayName ?? "Account").gallopText(.bodyMStrong)
                        Text("@" + (social.account?.handle ?? "") + " · " + (social.endpoint.host ?? "Cowchat Global"))
                            .gallopText(.caption, color: SemanticColor.textSecondary)
                            .lineLimit(1).truncationMode(.middle)
                            .help(social.endpoint.host ?? "Cowchat Global")
                    }
                    Spacer()
                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString("@" + (social.account?.handle ?? ""), forType: .string)
                    } label: {
                        GallopIconView(icon: .copy, fallbackSystemName: "doc.on.doc")
                            .foregroundStyle(SemanticColor.iconSecondary)
                            .frame(width: 32, height: 32).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain).help("Copy handle").accessibilityLabel("Copy handle")
                    Button {
                        profileHandle = social.account?.handle ?? ""
                        profileName = social.account?.displayName ?? ""
                        isEditingProfile = true
                    } label: {
                        Label("Edit profile", systemImage: "pencil")
                    }.buttonStyle(CowchatPillButtonStyle())
                    Menu {
                        Button("Refresh") { Task { await social.refresh() } }
                        Button("Sign out") { Task { await social.signOut() } }
                    } label: {
                        Image(systemName: "ellipsis")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(SemanticColor.iconSecondary)
                            .frame(width: 28, height: 28)
                    }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden).buttonStyle(.plain).fixedSize()
                    .help("Account actions").accessibilityLabel("Account actions")
                }
                .padding(14)
                .background(SemanticColor.surface600, in: RoundedRectangle(cornerRadius: 14))
                .overlay(RoundedRectangle(cornerRadius: 14).stroke(SemanticColor.borderDefault, lineWidth: 0.5))
                HStack(spacing: 4) {
                    ForEach(SocialSection.allCases, id: \.rawValue) { section in
                        Button { page = section } label: {
                            HStack(spacing: 6) {
                                Image(systemName: section.symbol).font(.system(size: 12, weight: .medium))
                                Text(section.rawValue).gallopText(.bodySStrong)
                                if section == .invitations && !social.invitations.isEmpty {
                                    Text("\(social.invitations.count)").gallopText(.dataLabel)
                                        .padding(.horizontal, 5)
                                        .background(SemanticColor.surface600, in: Capsule())
                                }
                            }
                            .frame(maxWidth: .infinity).padding(.vertical, 10)
                            .foregroundStyle(page == section ? SemanticColor.surfaceGlassOnTextDefault : SemanticColor.textSecondary)
                            .background(page == section ? SemanticColor.surfaceGlassOnDefault : Color.clear, in: Capsule())
                            .contentShape(Capsule())
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(page == section ? [.isSelected] : [])
                    }
                }
                .padding(4)
                .background(SemanticColor.surface400, in: Capsule())
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        switch page {
                        case .agents: agentsPage
                        case .invitations: invitationsPage
                        case .rooms: roomsPage
                        case .friends: friendsPage
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(2)
                }
            } else { signInPage; Spacer() }
        }
        .padding(24)
        .frame(width: 700, height: 650)
        .background(SemanticColor.surface500)
        .tint(SemanticColor.buttonPrimaryDefault)
        .buttonStyle(CowchatPillButtonStyle())
        .task { await social.refresh() }
        .onDisappear { social.cancelSignIn(); social.dismissAgentCredential() }
        .onChange(of: social.phase) { phase in
            guard phase != .signedIn else { return }
            invitation = nil
            participants = nil
            consentingAgents = []
            friendID = ""
        }
        .sheet(isPresented: $isEditingProfile) {
            SocialProfileEditor(
                handle: $profileHandle, displayName: $profileName, busy: social.busy, error: social.errorMessage,
                identities: social.identities, linkProviders: social.availableLinkProviders, linkingProvider: social.linkingProvider,
                onLink: { social.linkProvider($0) }, onCancelLink: { social.cancelSignIn() },
                onCancel: { isEditingProfile = false }, onSave: {
                Task { await social.perform { api in
                    let _: SocialMe = try await api.social(["me"], method: "PATCH", body: ["handle": profileHandle, "display_name": profileName])
                    isEditingProfile = false
                } }
            }).onDisappear { if social.linkingProvider != nil { social.cancelSignIn() } }
        }
        .sheet(isPresented: Binding(get: { invitation != nil }, set: { if !$0 { invitation = nil } })) {
            invitationSheet
        }
        .sheet(isPresented: Binding(get: { participants != nil }, set: { if !$0 { participants = nil } })) {
            participantsSheet
        }
        .sheet(isPresented: Binding(get: { social.issuedAgent != nil }, set: { if !$0 { social.dismissAgentCredential() } })) {
            agentCredentialSheet
        }
    }

    private var signInPage: some View {
        VStack(alignment: .leading, spacing: 16) {
            SocialIconBadge(symbol: "bubble.left.and.bubble.right", prominent: true)
            Text(social.serverNeedsUpdate ? "Friends needs a server update" : "Good company. Great agents.").gallopText(.h3)
            Text(social.serverNeedsUpdate
                 ? "This Mac has the new Friends feature. The connected Cowchat server needs to be updated before you can sign in and add friends."
                 : "Sign in to add friends and approve rooms where your agents can collaborate. Local rooms stay on this Mac. Existing API-key rooms stay separate.")
                .gallopText(.bodyM, color: SemanticColor.textSecondary)
            Label(social.endpoint.host ?? "Cowchat Global", systemImage: "network")
                .gallopText(.caption, color: SemanticColor.textTertiary)
            if social.serverNeedsUpdate {
                Text("Your existing rooms and connections are still available.")
                    .gallopText(.caption, color: SemanticColor.textSecondary)
                Button("Check again") { Task { await social.refresh() } }
                    .buttonStyle(CowchatPillButtonStyle())
            } else if social.phase == .awaitingBrowser {
                HStack { ProgressView().controlSize(.small); Text("Finish signing in in your browser."); Spacer(); Button("Cancel") { social.cancelSignIn() } }
            } else if social.providers.isEmpty {
                Text("Account sign-in is not configured on this server. Existing global connections still work in Settings.").gallopText(.caption, color: SemanticColor.textSecondary)
            } else {
                HStack {
                    ForEach(social.providers, id: \.self) { provider in
                        Button("Continue with \(provider == "github" ? "GitHub" : "Google")") { social.signIn(provider: provider) }.buttonStyle(CowchatPillButtonStyle(prominent: true))
                    }
                }
            }
        }.gallopCard(cornerRadius: 14)
    }

    private var friendsPage: some View {
        Group {
            VStack(alignment: .leading, spacing: 10) {
                Label("Add a friend", systemImage: "person.badge.plus").gallopText(.bodyMStrong)
                HStack {
                    TextField("@handle", text: $handle).textFieldStyle(.roundedBorder)
                    Button("Send request") {
                        Task { await social.perform { api in
                            struct Reply: Decodable { let request: SocialFriendRequest }
                            let _: Reply = try await api.social(["friend-requests"], method: "POST", body: ["handle": SocialAPI.normalizedHandle(handle)])
                            handle = ""
                        } }
                    }.buttonStyle(CowchatPillButtonStyle(prominent: true)).disabled(handle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || social.busy)
                }
                Text("Friends can invite you to a room. They cannot read your rooms or contact your agents until you approve that room.").gallopText(.caption, color: SemanticColor.textSecondary)
            }.gallopCard(cornerRadius: 14)
            ForEach(social.requests) { request in
                HStack {
                    SocialIconBadge(symbol: "envelope")
                    Text(request.from.id == social.account?.id ? "Request sent to @\(request.to.handle)" : "@\(request.from.handle) wants to be friends")
                        .gallopText(.bodyS)
                    Spacer()
                    if request.to.id == social.account?.id {
                        Button("Accept") { requestAction(request.id, "accept") }.buttonStyle(CowchatPillButtonStyle(prominent: true))
                        Button("Decline") { requestAction(request.id, "decline") }
                    }
                }.gallopCard(cornerRadius: 14)
            }
            ForEach(social.friends) { friend in
                HStack(spacing: 12) {
                    SocialPersonAvatar(name: friend.account.displayName)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(friend.account.displayName).gallopText(.bodyMStrong)
                        Text("@\(friend.account.handle) · \(friend.agents.count) discoverable \(friend.agents.count == 1 ? "agent" : "agents")").gallopText(.caption, color: SemanticColor.textSecondary)
                    }
                    Spacer()
                    Button { friendID = friend.id; selectedAgents = []; proposedAgents = []; roomName = "" } label: {
                        Label { Text("Collaborate") } icon: { CowchatCollaborateIcon() }
                    }.buttonStyle(CowchatPillButtonStyle(prominent: true))
                    Menu {
                        Button("Remove friend", role: .destructive) { Task { await social.perform { api in let _: SocialAcknowledgement = try await api.social(["friends", friend.id], method: "DELETE") } } }
                    } label: {
                        Image(systemName: "ellipsis")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(SemanticColor.iconSecondary)
                            .frame(width: 28, height: 28)
                    }.menuStyle(.borderlessButton).menuIndicator(.hidden).buttonStyle(.plain).fixedSize()
                        .accessibilityLabel("Actions for @\(friend.account.handle)")
                }.gallopCard(cornerRadius: 14)
            }
            if social.friends.isEmpty && social.requests.isEmpty {
                SocialEmptyState(symbol: "person.2", title: "Your crew starts here", detail: "Add a friend by their Cowchat handle, then invite their agents into a room.")
            }
            if !social.friends.isEmpty {
                Text("Removing a friend keeps already approved rooms. Leave those rooms separately to stop sharing them.").gallopText(.caption, color: SemanticColor.textSecondary)
            }
            if let friend = social.friends.first(where: { $0.id == friendID }) { collaborationForm(friend) }
        }
    }

    private func collaborationForm(_ friend: SocialFriend) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("New room with @\(friend.account.handle)").gallopText(.bodyM)
            TextField("Room name", text: $roomName).textFieldStyle(.roundedBorder)
            agentPicker("Approve your agents", agents: social.agents, selection: $selectedAgents)
            agentPicker("Suggest their agents", agents: friend.agents, selection: $proposedAgents)
            Text("Your friend must approve their own agents. Messages are unencrypted. Accepted participants can read all prior room history. This does not wake agents or authorize tool use.").gallopText(.caption, color: SemanticColor.textSecondary)
            HStack {
                Button("Cancel") { friendID = "" }
                Button("Create room & invite") {
                    Task { await social.perform { api in
                        struct Reply: Decodable { let roomId: String; let invitation: SocialRoomInvitation }
                        let _: Reply = try await api.social(["rooms"], method: "POST", body: ["name": roomName, "friend_id": friend.id, "agent_ids": Array(selectedAgents), "proposed_agent_ids": Array(proposedAgents)])
                        friendID = ""
                    } }
                }.buttonStyle(CowchatPillButtonStyle(prominent: true)).disabled(roomName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || social.busy)
            }
        }.gallopCard(cornerRadius: 14)
    }

    private var agentsPage: some View {
        Group {
            VStack(alignment: .leading, spacing: 10) {
                Label("Give your agent an identity", systemImage: "cpu").gallopText(.bodyMStrong)
                HStack {
                    TextField("Agent name", text: $agentName).textFieldStyle(.roundedBorder)
                    Button("Create agent") { Task { await social.createAgent(name: agentName); agentName = "" } }.disabled(agentName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || social.busy)
                }
                Text("Friends can discover these agents and suggest them for rooms. Only you can approve their participation. Each agent gets its own credential.").gallopText(.caption, color: SemanticColor.textSecondary)
            }.gallopCard(cornerRadius: 14)
            ForEach(social.agents) { agent in
                HStack {
                    SocialIconBadge(symbol: "cpu")
                    Text(agent.name).gallopText(.bodyMStrong)
                    Spacer()
                    Button("Revoke agent", role: .destructive) { Task { await social.perform { api in let _: SocialAcknowledgement = try await api.social(["agents", agent.id], method: "DELETE") } } }
                }.gallopCard(cornerRadius: 14)
            }
        }
    }

    private var invitationsPage: some View {
        Group {
            if social.invitations.isEmpty {
                SocialEmptyState(symbol: "envelope.open", title: "No invitations yet", detail: "When a friend invites you to collaborate, review the room and choose which agents join.")
            }
            ForEach(social.invitations) { invite in
                HStack {
                    VStack(alignment: .leading) {
                        Label(invite.roomName, systemImage: "envelope").gallopText(.bodyMStrong)
                        Text("From @\(invite.from.handle)").gallopText(.caption, color: SemanticColor.textSecondary)
                    }
                    Spacer()
                    Button("Review invitation") { invitation = invite; consentingAgents = [] }.buttonStyle(CowchatPillButtonStyle(prominent: true))
                    Button("Decline") { Task { await social.perform { api in let _: SocialAcknowledgement = try await api.social(["room-invitations", invite.id, "decline"], method: "POST") } } }
                }.gallopCard(cornerRadius: 14)
            }
        }
    }

    private var roomsPage: some View {
        Group {
            if social.rooms.isEmpty {
                SocialEmptyState(symbol: "bubble.left.and.bubble.right", title: "Make room for your crew", detail: "Create a room with a friend or accept an invitation. You choose which of your agents can join.")
            }
            ForEach(social.rooms) { room in
                HStack {
                    SocialIconBadge(symbol: "bubble.left.and.bubble.right")
                    Text(room.name).gallopText(.bodyMStrong)
                    Spacer()
                    Button("Participants") { Task { await social.perform { api in participants = try await api.social(["rooms", room.id, "participants"]); consentingAgents = [] } } }
                }.gallopCard(cornerRadius: 14)
            }
        }
    }

    private var invitationSheet: some View {
        Group {
            if let invite = invitation {
                SocialInvitationReview(invitation: invite, agents: social.agents, selectedAgents: $consentingAgents, busy: social.busy, error: social.errorMessage, onCancel: { invitation = nil }, onAccept: {
                    Task { await social.perform { api in
                        struct Reply: Decodable { let roomId: String }
                        let _: Reply = try await api.social(["room-invitations", invite.id, "accept"], method: "POST", body: ["agent_ids": Array(consentingAgents)])
                        invitation = nil
                    } }
                })
            }
        }
    }

    private var participantsSheet: some View {
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
                agentPicker("Approve additional agents", agents: social.agents.filter { own in !room.agents.contains(where: { $0.id == own.id }) }, selection: $consentingAgents)
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

    private var agentCredentialSheet: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let issued = social.issuedAgent {
                Text("Connect \(issued.agent.name)").gallopText(.h3)
                Text("Copy this credential now. It will not be shown again. Keep it private and give it only to this agent.").gallopText(.bodyM, color: SemanticColor.textSecondary)
                Text(issued.apiKey).font(.system(.caption, design: .monospaced)).textSelection(.enabled).gallopCard(cornerRadius: 14)
                Button("Copy credential") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(issued.apiKey, forType: .string) }
                Text("Pass these flags on every Cowchat agent command. Join only a room where you have approved this agent.").gallopText(.caption, color: SemanticColor.textSecondary)
                Text("--url \(social.endpoint.absoluteString) --key <credential> --name '\(issued.agent.name.replacingOccurrences(of: "'", with: "'\\''"))' --agent-id \(issued.agent.id)")
                    .font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                Button("I saved the credential") { social.dismissAgentCredential() }.buttonStyle(CowchatPillButtonStyle(prominent: true))
            }
        }.padding(24).frame(width: 570).background(SemanticColor.surface500)
        .buttonStyle(CowchatPillButtonStyle())
    }

    private func agentPicker(_ title: String, agents: [SocialAgent], selection: Binding<Set<String>>) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).gallopText(.caption, color: SemanticColor.textSecondary)
            if agents.isEmpty { Text("No available agents.").gallopText(.caption, color: SemanticColor.textTertiary) }
            ForEach(agents) { agent in
                Toggle(agent.name, isOn: Binding(get: { selection.wrappedValue.contains(agent.id) }, set: { value in
                    if value { selection.wrappedValue.insert(agent.id) } else { selection.wrappedValue.remove(agent.id) }
                })).toggleStyle(.checkbox)
            }
        }
    }

    private func requestAction(_ id: String, _ action: String) {
        Task { await social.perform { api in let _: SocialAcknowledgement = try await api.social(["friend-requests", id, action], method: "POST") } }
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

struct SocialInvitationReview: View {
    let invitation: SocialRoomInvitation
    let agents: [SocialAgent]
    @Binding var selectedAgents: Set<String>
    let busy: Bool
    let error: String?
    let onCancel: () -> Void
    let onAccept: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Join \(invitation.roomName)?", systemImage: "envelope.open").gallopText(.h3)
            Text("Invited by @\(invitation.from.handle)").gallopText(.bodyM)
            SocialConsentDetails(members: invitation.members, agents: invitation.agents, slack: invitation.slackConnected)
            if !invitation.proposedAgents.isEmpty {
                Text("Suggested agents: \(invitation.proposedAgents.map(\.name).joined(separator: ", "))").gallopText(.caption)
            }
            Text("Approve your agents").gallopText(.caption, color: SemanticColor.textSecondary)
            ForEach(agents) { agent in
                Toggle(agent.name, isOn: Binding(get: { selectedAgents.contains(agent.id) }, set: { selected in
                    if selected { selectedAgents.insert(agent.id) } else { selectedAgents.remove(agent.id) }
                })).toggleStyle(.checkbox)
            }
            Text("You can join without adding an agent. This does not wake agents or authorize tool use.").gallopText(.caption, color: SemanticColor.textSecondary)
            if let error { Text(error).foregroundStyle(.red).font(.caption) }
            HStack {
                Button("Cancel", action: onCancel)
                Spacer()
                Button("Accept room invitation", action: onAccept).buttonStyle(CowchatPillButtonStyle(prominent: true)).disabled(busy)
            }
        }.padding(24).frame(width: 570).background(SemanticColor.surface500)
        .buttonStyle(CowchatPillButtonStyle())
    }
}

struct SocialProfileEditor: View {
    @Binding var handle: String
    @Binding var displayName: String
    let busy: Bool
    let error: String?
    var identities: [SocialMe.Identity] = []
    var linkProviders: [String] = []
    var linkingProvider: String?
    var onLink: (String) -> Void = { _ in }
    var onCancelLink: () -> Void = {}
    let onCancel: () -> Void
    let onSave: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Edit profile", systemImage: "person.crop.circle").gallopText(.h3)
            TextField("Display name", text: $displayName).textFieldStyle(.roundedBorder)
            TextField("Cowchat handle", text: $handle).textFieldStyle(.roundedBorder)
            Text("Friends find you by this handle. It must be unique on this server.").gallopText(.caption, color: SemanticColor.textSecondary)
            VStack(alignment: .leading, spacing: 10) {
                Label("Linked sign-ins", systemImage: "link").gallopText(.bodyMStrong)
                ForEach(identities, id: \.provider) { identity in
                    Label("\(identity.provider == "github" ? "GitHub" : "Google") · \(identity.label)", systemImage: "checkmark.circle.fill")
                        .gallopText(.caption, color: SemanticColor.textSecondary)
                }
                if let linkingProvider {
                    HStack {
                        ProgressView().controlSize(.small)
                        Text("Finish linking \(linkingProvider == "github" ? "GitHub" : "Google") in your browser.").gallopText(.caption)
                        Button("Cancel linking", action: onCancelLink)
                    }
                } else {
                    ForEach(linkProviders, id: \.self) { provider in
                        Button("Link \(provider == "github" ? "GitHub" : "Google")") { onLink(provider) }.disabled(busy)
                    }
                }
                Text("Link another sign-in to this Cowchat account. Accounts are never merged by email.").gallopText(.caption, color: SemanticColor.textSecondary)
            }.gallopCard(cornerRadius: 14)
            if let error { Text(error).foregroundStyle(.red).font(.caption) }
            HStack {
                Button("Cancel", action: onCancel)
                Spacer()
                Button("Save profile", action: onSave).buttonStyle(CowchatPillButtonStyle(prominent: true)).disabled(handle.isEmpty || displayName.isEmpty || busy || linkingProvider != nil)
            }
        }.padding(24).frame(width: 470).background(SemanticColor.surface500)
        .buttonStyle(CowchatPillButtonStyle())
    }
}

private struct CowchatCollaborateIcon: View {
    var body: some View {
        Canvas { context, size in
            context.scaleBy(x: size.width / 32, y: size.height / 28)
            let brown = Color(red: 0.43, green: 0.25, blue: 0.14)
            let cream = Color(red: 0.96, green: 0.86, blue: 0.69)
            let muzzle = Color(red: 0.76, green: 0.54, blue: 0.34)

            let ears = Path { path in
                path.addEllipse(in: CGRect(x: 0, y: 7, width: 9, height: 5))
                path.addEllipse(in: CGRect(x: 15, y: 7, width: 9, height: 5))
            }
            context.fill(ears, with: .color(brown))
            context.fill(
                RoundedRectangle(cornerRadius: 6).path(in: CGRect(x: 5, y: 5, width: 14, height: 17)),
                with: .color(brown)
            )
            context.fill(
                Path(ellipseIn: CGRect(x: 5, y: 15, width: 14, height: 8)),
                with: .color(muzzle)
            )
            for x in [8.0, 14.0] {
                context.fill(Path(ellipseIn: CGRect(x: x, y: 10, width: 2, height: 2.5)), with: .color(cream))
            }
            for x in [8.0, 13.0] {
                context.fill(Path(ellipseIn: CGRect(x: x, y: 18, width: 1.5, height: 2)), with: .color(brown))
            }

            let chat = Path { path in
                path.move(to: CGPoint(x: 17, y: 11))
                path.addLine(to: CGPoint(x: 28, y: 11))
                path.addQuadCurve(to: CGPoint(x: 31, y: 14), control: CGPoint(x: 31, y: 11))
                path.addLine(to: CGPoint(x: 31, y: 21))
                path.addQuadCurve(to: CGPoint(x: 28, y: 24), control: CGPoint(x: 31, y: 24))
                path.addLine(to: CGPoint(x: 27, y: 24))
                path.addLine(to: CGPoint(x: 27, y: 27))
                path.addLine(to: CGPoint(x: 23, y: 24))
                path.addLine(to: CGPoint(x: 17, y: 24))
                path.addQuadCurve(to: CGPoint(x: 14, y: 21), control: CGPoint(x: 14, y: 24))
                path.addLine(to: CGPoint(x: 14, y: 14))
                path.addQuadCurve(to: CGPoint(x: 17, y: 11), control: CGPoint(x: 14, y: 11))
                path.closeSubpath()
            }
            context.fill(chat, with: .color(SemanticColor.buttonPrimaryDefault))
            context.stroke(chat, with: .color(SemanticColor.buttonPrimaryTextDefault), style: StrokeStyle(lineWidth: 1.5, lineJoin: .round))
        }
        .frame(width: 28, height: 24)
        .accessibilityHidden(true)
    }
}

private enum SocialSection: String, CaseIterable {
    case friends = "Friends"
    case agents = "My agents"
    case invitations = "Invitations"
    case rooms = "Rooms"

    var symbol: String {
        switch self {
        case .friends: "person.2"
        case .agents: "cpu"
        case .invitations: "envelope"
        case .rooms: "bubble.left.and.bubble.right"
        }
    }
}

private struct SocialIconBadge: View {
    let symbol: String
    var prominent = false

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: prominent ? 20 : 16, weight: .medium))
            .foregroundStyle(prominent ? SemanticColor.buttonPrimaryTextDefault : SemanticColor.surfaceGlassOnTextDefault)
            .frame(width: prominent ? 48 : 36, height: prominent ? 48 : 36)
            .background(prominent ? SemanticColor.buttonPrimaryDefault : SemanticColor.surfaceGlassOnDefault, in: RoundedRectangle(cornerRadius: prominent ? 16 : 12))
            .accessibilityHidden(true)
    }
}

private struct SocialPersonAvatar: View {
    let name: String

    var body: some View {
        Text(String(name.split(whereSeparator: { $0.isWhitespace }).prefix(2).compactMap(\.first)))
            .textCase(.uppercase)
            .gallopText(.bodyMStrong, color: SemanticColor.surfaceGlassOnTextDefault)
            .frame(width: 40, height: 40)
            .background(SemanticColor.surfaceGlassOnDefault, in: Circle())
            .overlay(Circle().stroke(SemanticColor.borderDefault, lineWidth: 0.5))
            .accessibilityHidden(true)
    }
}

private struct SocialEmptyState: View {
    let symbol: String
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 12) {
            SocialIconBadge(symbol: symbol)
            Text(title).gallopText(.h4)
            Text(detail).gallopText(.bodyS, color: SemanticColor.textSecondary)
                .multilineTextAlignment(.center).frame(maxWidth: 360)
        }
        .frame(maxWidth: .infinity).padding(.vertical, 32)
    }
}
