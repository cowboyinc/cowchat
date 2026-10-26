import AppKit
import SwiftUI

enum CowchatOnboarding {
    static let currentVersion = 1
    static let completedVersionKey = "CowchatMac.completedOnboardingVersion"
    static let migrationAttemptedKey = "CowchatMac.onboardingMigrationAttempted"

    static func migrateExistingUser(defaults: UserDefaults, hadExistingAgentID: Bool) {
        guard defaults.object(forKey: migrationAttemptedKey) == nil else { return }
        defaults.set(true, forKey: migrationAttemptedKey)
        guard hadExistingAgentID,
              defaults.object(forKey: completedVersionKey) == nil else { return }
        defaults.set(currentVersion, forKey: completedVersionKey)
    }
}

enum WizardStep: Equatable {
    case login, name, friends, ready

    var number: Int {
        switch self { case .login: return 1; case .name: return 2; case .friends: return 3; case .ready: return 4 }
    }
}

struct CowchatOnboardingView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    let onComplete: () -> Void

    var body: some View {
        CowchatSetupWizard(social: workspace.social, onLocal: {
            workspace.activate(server: .local)
            onComplete()
        }, onOpenHome: { setup, session in
            guard await workspace.openStarterHome(roomID: setup.roomId, session: session) else { return false }
            onComplete()
            return true
        }, onOpenRooms: {
            workspace.activate(server: .accountGlobal)
            onComplete()
        })
    }
}

struct CowchatSetupWizard: View {
    @ObservedObject var social: SocialStore
    let onLocal: () -> Void
    let onOpenHome: (SocialSetup, SocialSession) async -> Bool
    let onOpenRooms: () -> Void
    @State private var step: WizardStep
    @State private var name: String
    @State private var handle = ""
    @State private var handleCheck: SocialHandleCheck = .idle
    @State private var copiedLink = false
    @State private var opening = false
    @State private var navigationError: String?

    init(social: SocialStore, initialStep: WizardStep = .login, name: String = "", onLocal: @escaping () -> Void,
         onOpenHome: @escaping (SocialSetup, SocialSession) async -> Bool, onOpenRooms: @escaping () -> Void) {
        self.social = social
        self.onLocal = onLocal
        self.onOpenHome = onOpenHome
        self.onOpenRooms = onOpenRooms
        _step = State(initialValue: initialStep)
        _name = State(initialValue: name)
        _handle = State(initialValue: social.account?.handle ?? "")
    }

    var body: some View {
        ZStack {
            SemanticColor.surface500
            VStack(spacing: 18) {
                HStack {
                    Label("COWCHAT", systemImage: "bubble.left.and.bubble.right.fill").gallopText(.bodyMStrong)
                    Spacer()
                    Text("Step \(step.number) of 4").gallopText(.caption, color: SemanticColor.textTertiary)
                }
                Spacer(minLength: 0)
                if let icon = CowchatAppDelegate.applicationIcon() {
                    Image(nsImage: icon).resizable().scaledToFit().frame(width: 64, height: 64)
                        .clipShape(RoundedRectangle(cornerRadius: 20)).accessibilityHidden(true)
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        switch step {
                        case .login: login
                        case .name: confirmName
                        case .friends: inviteFriends
                        case .ready: ready
                        }
                        if let error = navigationError ?? social.errorMessage {
                            Text(error).gallopText(.bodyS).foregroundStyle(.red).textSelection(.enabled)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxWidth: 540)
                Spacer(minLength: 0)
                HStack {
                    Button("Use Local rooms for now") { social.cancelSignIn(); onLocal() }
                        .buttonStyle(CowchatPillButtonStyle())
                    Spacer()
                }
            }
            .padding(32)
        }
        .frame(minWidth: 900, minHeight: 600)
        .task(id: social.savedSession?.scopeId) { await recover() }
        .onDisappear { social.cancelSignIn() }
        .onChange(of: social.savedSession?.account.id) { _ in
            step = .login
            name = ""
            handle = ""
            handleCheck = .idle
            copiedLink = false
            navigationError = nil
            opening = false
        }
    }

    private var login: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Howdy. Make yourself at home.").gallopText(.h3)
            Text("Cowchat lets you communicate with your existing agents. Connect Claude, Codex, or another agent, bring them into rooms, and collaborate with your friends and their agents.")
                .gallopText(.bodyL, color: SemanticColor.textSecondary).fixedSize(horizontal: false, vertical: true)
            if social.serverNeedsUpdate || (social.phase == .signedIn && !social.setupSupported) {
                Text("This server needs an update before guided setup is available.").gallopText(.bodyM)
                Button("Try again") { Task { await recover() } }.buttonStyle(CowchatPillButtonStyle(prominent: true))
            } else if social.phase == .awaitingBrowser {
                ProgressView("Finish signing in in your browser")
                Button("Cancel sign-in") { social.cancelSignIn() }
            } else if social.phase == .signedIn {
                ProgressView("Finding your home…")
                Button("Try again") { Task { await recover() } }
            } else {
                ForEach(social.providers, id: \.self) { provider in
                    Button("Continue with \(provider == "github" ? "GitHub" : "Google")") { social.signIn(provider: provider) }
                        .buttonStyle(CowchatPillButtonStyle(prominent: true))
                }
                if social.providers.isEmpty {
                    Text("Sign-in is unavailable on this server right now.").gallopText(.bodyS, color: SemanticColor.textSecondary).fixedSize(horizontal: false, vertical: true)
                    Button("Try again") { Task { await recover() } }
                }
            }
        }
    }

    private var confirmName: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("What should we call you?").gallopText(.h3)
            TextField("Your name", text: $name).textFieldStyle(.roundedBorder).font(.system(size: 20))
            if social.handleAvailabilitySupported {
                HStack {
                    Text("@").gallopText(.bodyL)
                    TextField("Your username", text: $handle).textFieldStyle(.roundedBorder)
                        .accessibilityLabel("Your username")
                }
                Text(handleCheck.message).gallopText(.bodyS, color: SemanticColor.textSecondary)
                    .task(id: handle + "|" + (social.savedSession?.scopeId ?? "")) { await checkHandle() }
            }
            VStack(alignment: .leading, spacing: 12) {
                Label(SocialStarterNames.bot(name), systemImage: "cpu").gallopText(.bodyMStrong)
                Label(SocialStarterNames.home(name), systemImage: "house").gallopText(.bodyMStrong)
                Text("Home may receive a number if that name is already taken.").gallopText(.caption, color: SemanticColor.textSecondary)
            }.frame(maxWidth: .infinity, alignment: .leading).gallopCard(cornerRadius: 14)
            Text("We’ll create a bot identity for your existing agent and a private Home. This does not start an agent. This Home is private but not end-to-end encrypted. Cowchat also supports end-to-end encrypted rooms.")
                .gallopText(.bodyS, color: SemanticColor.textSecondary).fixedSize(horizontal: false, vertical: true)
            Button(social.busy ? "Getting things ready…" : "Create my Home") {
                Task { if await social.establishSetup(displayName: name, handle: social.handleAvailabilitySupported ? SocialAPI.normalizedHandle(handle) : nil) { step = .friends } }
            }
            .buttonStyle(CowchatPillButtonStyle(prominent: true))
            .disabled(social.busy || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (social.handleAvailabilitySupported && !handleCheck.permits(SocialAPI.normalizedHandle(handle))))
        }
    }

    private var inviteFriends: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Bring your people.").gallopText(.h3)
            Text("Share your invitation link. Your friends can open it in Cowchat and send you a friend request. You choose who to approve. Friendship does not grant access to your Home or bot.")
                .gallopText(.bodyM, color: SemanticColor.textSecondary).fixedSize(horizontal: false, vertical: true)
            if let link = social.friendLink {
                Text(link.url.absoluteString).gallopText(.bodyMStrong).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading).gallopCard(cornerRadius: 14)
                Button(copiedLink ? "Invitation link copied" : "Copy invitation link") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(link.url.absoluteString, forType: .string)
                    copiedLink = true
                }.buttonStyle(CowchatPillButtonStyle(prominent: true))
            } else if !social.friendLinksSupported {
                Text("Update your Cowchat server to share invitation links.").gallopText(.bodyS)
            } else if let failure = social.friendLinkFailure {
                Text(failure).gallopText(.bodyS, color: SemanticColor.textSecondary)
                Button("Try loading my invitation again") { Task { await social.loadFriendLink() } }
            } else { ProgressView("Getting your invitation link…") }
            HStack {
                Spacer()
                Button(copiedLink ? "Continue" : "Skip for now") { step = .ready }
                    .buttonStyle(CowchatPillButtonStyle()).disabled(social.busy)
            }
        }
        .task(id: social.savedSession?.scopeId) { await social.loadFriendLink() }
    }

    private var ready: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Your corner of the pasture.").gallopText(.h3)
            if let setup = social.setup {
                Label(setup.room?.name ?? setup.roomName, systemImage: "house.fill").gallopText(.bodyL)
                Text(setup.agent == nil ? "Your original bot is unavailable. Setup will not recreate revoked identities." : "\(setup.agent?.name ?? setup.agentName) is your bot identity. Connect it to Claude, Codex, or another agent from Friends & agents → My agents.")
                    .gallopText(.bodyM, color: SemanticColor.textSecondary).fixedSize(horizontal: false, vertical: true)
                if !setup.botAdmitted { Text("Your bot is not currently approved for Home. Review its access in Participants.").gallopText(.bodyS, color: SemanticColor.textSecondary).fixedSize(horizontal: false, vertical: true) }
                if setup.room != nil {
                    Button(opening ? "Opening…" : "Open my Home") {
                        guard let session = social.savedSession else { return }
                        opening = true
                        Task {
                            let opened = await onOpenHome(setup, session)
                            if social.savedSession?.scopeId == session.scopeId {
                                opening = false
                                if !opened { navigationError = "Home could not be opened. Try again, or open your rooms." }
                            }
                        }
                    }.buttonStyle(CowchatPillButtonStyle(prominent: true)).disabled(opening)
                } else {
                    Text("Your original Home is unavailable. Setup keeps your current access decisions.").gallopText(.bodyS, color: SemanticColor.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
                Button("Invite friends") { step = .friends }.buttonStyle(CowchatPillButtonStyle())
                Button("Open my rooms") { onOpenRooms() }.buttonStyle(CowchatPillButtonStyle())
            }
        }
    }

    private func checkHandle() async {
        let input = SocialAPI.normalizedHandle(handle)
        let scope = social.savedSession?.scopeId
        guard FriendInvitationRoute.validSlug(input) else { handleCheck = .invalid(input); return }
        handleCheck = .checking(input)
        do {
            try await Task.sleep(nanoseconds: 300_000_000)
            try Task.checkCancellation()
            let result = try await social.checkHandle(input)
            guard !Task.isCancelled, input == SocialAPI.normalizedHandle(handle), scope == social.savedSession?.scopeId else { return }
            handleCheck = !result.valid ? .invalid(input) : result.available ? .available(input) : .unavailable(input)
        } catch {
            guard !Task.isCancelled, input == SocialAPI.normalizedHandle(handle), scope == social.savedSession?.scopeId else { return }
            handleCheck = .failed(input)
        }
    }

    private func recover() async {
        let scope = social.savedSession?.scopeId
        await social.loadSetup()
        guard scope == social.savedSession?.scopeId, !Task.isCancelled else { return }
        guard social.phase == .signedIn, social.setupLoaded else { return }
        name = social.account?.displayName ?? ""
        handle = social.account?.handle ?? ""
        step = social.setup == nil ? .name : .ready
    }
}
