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

/// First run's steps, in the app's order: sign in, the name (which makes Home), the agents, and the wait for the first
/// session.
enum WizardStep: Equatable, CaseIterable {
    case login, name, agents, ready
}

struct CowchatOnboardingView: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    let onComplete: () -> Void

    var body: some View {
        CowchatSetupWizard(social: workspace.social, firstRun: workspace.firstRun, agents: workspace.agentConnections,
                           onLocal: {
            workspace.activate(server: .local)
            onComplete()
        }, onOpenHome: { setup, session in
            guard await workspace.openStarterHome(roomID: setup.roomId, session: session) else { return false }
            onComplete()
            return true
        }, onSignOut: {
            await workspace.signOut()
        })
        // The window is one surface while first run shows: the title bar takes the page's colour and holds only the
        // traffic lights.
        .toolbarBackground(SemanticColor.surface500, for: .windowToolbar)
    }
}

/// First run, drawn as Cowboy's sign-in pages are: one centred column holding the app's icon, a Heading 2 title and a
/// raised card. Every button is the 36 pill. Inside a card a button takes the card's width; under a card it keeps its
/// own width, centred, 24 below. There is no Back.
struct CowchatSetupWizard: View {
    @ObservedObject var social: SocialStore
    let onLocal: () -> Void
    let onOpenHome: (SocialSetup, SocialSession) async -> Bool
    let onSignOut: () async -> Void
    @ObservedObject private var firstRun: FirstRun
    @ObservedObject private var agents: AgentConnections
    @State private var name: String
    @State private var handle = ""
    @State private var handleCheck: SocialHandleCheck = .idle
    @State private var opening = false
    @State private var navigationError: String?

    /// The workspace passes its own first run and agents; on their own, as in a render, the wizard starts at
    /// `initialStep` with nothing connected.
    init(social: SocialStore, firstRun: FirstRun? = nil, agents: AgentConnections? = nil,
         initialStep: WizardStep = .login, name: String = "", onLocal: @escaping () -> Void,
         onOpenHome: @escaping (SocialSetup, SocialSession) async -> Bool,
         onSignOut: @escaping () async -> Void = {}) {
        self.social = social
        self.onLocal = onLocal
        self.onOpenHome = onOpenHome
        self.onSignOut = onSignOut
        self.firstRun = firstRun ?? FirstRun(step: initialStep)
        self.agents = agents ?? AgentConnections()
        _name = State(initialValue: name)
        _handle = State(initialValue: social.account?.handle ?? "")
    }

    var body: some View {
        SetupStage {
            switch firstRun.step {
            case .login: login
            case .name: confirmName
            case .agents: connectAgents
            case .ready: waitForSession
            }
        }
        .frame(minWidth: 900, minHeight: 600)
        .task(id: social.savedSession?.scopeId) { await recover() }
        .onDisappear { social.closeOnboarding() }
        .onChange(of: social.savedSession?.account.id) { _ in
            firstRun.reset()
            name = ""
            handle = ""
            handleCheck = .idle
            navigationError = nil
            opening = false
        }
    }

    // MARK: Signed out

    /// The signed-out page: what Cowchat does, then a door for each provider, and Local rooms under the card. While
    /// the browser is out, the pressed door waits and the link cancels. Once signed in, the card finds the account's
    /// Home.
    private var login: some View {
        VStack(spacing: 0) {
            SetupTitle(
                // copy:
                title: "Chat with your coding agents",
                // copy:
                line: "Talk to your Claude Code and Codex sessions from one place, and see what your team runs.")
            if social.serverNeedsUpdate || (social.phase == .signedIn && !social.setupSupported) {
                // copy:
                SetupCard { notice("This server needs an update before guided setup is available.") }
            } else if social.phase == .signedIn {
                SetupCard {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small).frame(width: 16, height: 16)
                        // copy:
                        Text("Finding your Home").gallopText(.bodyM, color: SemanticColor.textSecondary)
                    }
                    .frame(maxWidth: .infinity)
                }
            } else if social.providers.isEmpty {
                // copy:
                SetupCard { notice("Sign-in is unavailable on this server right now.") }
            } else {
                SetupCard { doors }
            }
            failure(navigationError ?? social.errorMessage,
                    retry: navigationError != nil || social.phase == .signedIn ? { Task { await recover() } } : nil)
            switch social.phase {
            case .signedOut:
                // copy:
                SetupLink(title: "Use Local rooms") { social.cancelSignIn(); onLocal() }.padding(.top, 14)
            case .awaitingBrowser:
                // copy:
                SetupLink(title: "Cancel") { social.cancelSignIn() }.padding(.top, 14)
            case .signedIn:
                EmptyView()
            }
        }
    }

    /// A door for each provider the server offers: the provider's mark and name, centred together. The pressed door
    /// waits for the browser with a spinner in place of its mark, and both doors are disabled meanwhile.
    private var doors: some View {
        VStack(spacing: 12) {
            ForEach(social.providers, id: \.self) { provider in
                let waiting = social.phase == .awaitingBrowser && social.signInProvider == provider
                Button { social.signIn(provider: provider) } label: {
                    HStack(spacing: 8) {
                        if waiting {
                            ProgressView().controlSize(.small).frame(width: 16, height: 16)
                        } else {
                            ProviderMark(provider: provider)
                        }
                        // copy:
                        Text(waiting ? "Waiting for your browser"
                             : "Continue with \(provider == "github" ? "GitHub" : "Google")")
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(CowchatPillButtonStyle(identity: true))
                .disabled(social.phase != .signedOut)
            }
        }
    }

    /// A card that can only say why sign-in can't go on, with Try again as its one button.
    private func notice(_ line: String) -> some View {
        VStack(spacing: 20) {
            CentredText(text: line, style: .bodyM, color: SemanticColor.textPrimary)
            // copy:
            Button { Task { await recover() } } label: { Text("Try again").frame(maxWidth: .infinity) }
                .buttonStyle(CowchatPillButtonStyle(prominent: true))
        }
        .frame(maxWidth: .infinity)
    }

    // MARK: Name

    /// The name and handle the account will go by. Continue makes the account's Home and bot.
    private var confirmName: some View {
        VStack(spacing: 0) {
            // copy:
            SetupTitle(title: "What should we call you?")
            SetupCard {
                VStack(alignment: .leading, spacing: 0) {
                    // copy:
                    SetupField(placeholder: "Your name", text: $name)
                    if social.handleAvailabilitySupported {
                        // copy:
                        SetupField(placeholder: "Your username", text: $handle, leading: "@") { handleStatus }
                            .padding(.top, 12)
                            .task(id: handle + "|" + (social.savedSession?.scopeId ?? "")) { await checkHandle() }
                        if let problem = handleProblem {
                            Text(problem)
                                .gallopText(.bodyS, color: SemanticColor.textError)
                                .fixedSize(horizontal: false, vertical: true)
                                .padding(.top, 6)
                        }
                    }
                    Button(action: createHome) {
                        HStack(spacing: 8) {
                            if social.busy { ProgressView().controlSize(.small).frame(width: 16, height: 16) }
                            // copy:
                            Text("Continue")
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(CowchatPillButtonStyle(prominent: true))
                    .disabled(!canCreateHome)
                    .padding(.top, 20)
                }
            }
            failure(navigationError ?? social.errorMessage)
            // copy:
            SetupLink(title: "Use another account") { Task { await onSignOut() } }.padding(.top, 14)
        }
    }

    /// The handle check's mark at the field's end: a spinner while it checks, and a tick once the handle is free.
    @ViewBuilder private var handleStatus: some View {
        switch handleCheck {
        case .checking: ProgressView().controlSize(.small).frame(width: 16, height: 16)
        case .available: SuccessTick()
        default: EmptyView()
        }
    }

    /// Why the handle can't be used: taken, not a handle, or not checked.
    private var handleProblem: String? {
        switch handleCheck {
        case .unavailable, .invalid, .failed: return handleCheck.message
        default: return nil
        }
    }

    private var canCreateHome: Bool {
        !social.busy && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (!social.handleAvailabilitySupported || handleCheck.permits(SocialAPI.normalizedHandle(handle)))
    }

    private func createHome() {
        let handle = social.handleAvailabilitySupported ? SocialAPI.normalizedHandle(handle) : nil
        Task { if await social.establishSetup(displayName: name, handle: handle) { firstRun.homeCreated() } }
    }

    // MARK: Agents

    /// Claude Code and Codex, each with its own Connect. Skip leaves both for later; once one is connected, or waits
    /// for the person to trust its hooks, the button reads Continue.
    private var connectAgents: some View {
        VStack(spacing: 0) {
            // copy:
            SetupTitle(title: "Connect your agents")
            SetupCard(list: true) { AgentHookList(connections: agents) }
            failure(agents.errorMessage)
            Group {
                if agents.hasConnected {
                    // copy:
                    Button("Continue") { firstRun.agentsDone() }.buttonStyle(CowchatPillButtonStyle(prominent: true))
                } else {
                    // copy:
                    Button("Skip") { firstRun.skipAgents() }.buttonStyle(CowchatPillButtonStyle())
                }
            }
            .fixedSize()
            .padding(.top, 24)
        }
        .task { await agents.refresh() }
    }

    // MARK: The first session

    /// The agents as they stand, and a last row that waits for the first session to join Home, which then opens
    /// (`FirstRun.sessionsChanged`). Open Home skips the wait.
    private var waitForSession: some View {
        VStack(spacing: 0) {
            // copy:
            SetupTitle(title: "Start Claude Code or Codex")
            SetupCard(list: true) {
                VStack(spacing: 0) {
                    AgentHookList(connections: agents)
                    AgentHookSeparator()
                    HStack(spacing: 12) {
                        // The spinner sits in the marks' column, so the line starts where the agents' names do.
                        ProgressView().controlSize(.small).frame(width: 24, height: 16)
                        // copy:
                        Text("Waiting for your first session").gallopText(.bodyM, color: SemanticColor.textTertiary)
                        Spacer(minLength: 0)
                    }
                    .frame(height: 44)
                }
            }
            failure(navigationError ?? agents.errorMessage)
            // copy:
            Button(action: openHome) { Text("Open Home") }
                .buttonStyle(CowchatPillButtonStyle())
                .fixedSize()
                .disabled(opening || social.setup == nil)
                .padding(.top, 24)
        }
    }

    // MARK: Shared

    /// A failure under the card: one centred line in Text/Error, 12 below, and Try again 24 under it when trying again
    /// helps.
    @ViewBuilder
    private func failure(_ message: String?, retry: (() -> Void)? = nil) -> some View {
        if let message {
            CentredText(text: message, style: .bodyS, color: SemanticColor.textError)
                .textSelection(.enabled)
                .padding(.top, 12)
            if let retry {
                // copy:
                Button("Try again", action: retry).buttonStyle(CowchatPillButtonStyle()).fixedSize().padding(.top, 24)
            }
        }
    }

    private func openHome() {
        guard let setup = social.setup, let session = social.savedSession else { return }
        opening = true
        navigationError = nil
        Task {
            let opened = await onOpenHome(setup, session)
            if social.savedSession?.scopeId == session.scopeId {
                opening = false
                // copy:
                if !opened { navigationError = "Home could not be opened." }
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

    /// Loads the account's setup as the wizard appears or the account changes. An account that comes back with its
    /// Home already made skips the rest of first run, and Home opens.
    private func recover() async {
        let scope = social.savedSession?.scopeId
        navigationError = nil
        await social.loadSetup()
        guard scope == social.savedSession?.scopeId, !Task.isCancelled else { return }
        guard social.phase == .signedIn, social.setupLoaded else { return }
        name = social.account?.displayName ?? ""
        handle = social.account?.handle ?? ""
        if firstRun.recovered(hasSetup: social.setup != nil) { openHome() }
    }
}

/// The column every step sits in, as Cowboy's sign-in pages place theirs: centred, at most 560 wide with at least 24
/// at the sides, and centred in the window below the title bar, then raised 38. A column taller than the window less 64
/// scrolls, with 32 above and below.
private struct SetupStage<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        GeometryReader { proxy in
            ScrollView {
                content
                    .frame(maxWidth: 560)
                    .padding(.horizontal, 24)
                    .padding(.vertical, 32)
                    .frame(maxWidth: .infinity)
                    .alignmentGuide(VerticalAlignment.center) { column in
                        column.height / 2 + min(38, max(0, (proxy.size.height - column.height) / 2))
                    }
                    .frame(minHeight: proxy.size.height)
            }
        }
        .background(SemanticColor.surface500)
    }
}

/// The app's icon at 48, 24, then the step's title in Heading 2, centred, with 28 under them. Only the signed-out page
/// has a line under its title, 10 below and at most 400 wide, because it is the one screen that needs it to act.
private struct SetupTitle: View {
    let title: String
    var line: String?

    var body: some View {
        VStack(spacing: 0) {
            if let icon = CowchatAppDelegate.applicationIcon() {
                Image(nsImage: icon).resizable().scaledToFit().frame(width: 48, height: 48)
                    .clipShape(RoundedRectangle(cornerRadius: 15, style: .continuous))
                    .accessibilityHidden(true)
                    .padding(.bottom, 24)
            }
            CentredText(text: title, style: .h2, color: SemanticColor.textPrimary)
            if let line {
                CentredText(text: line, style: .bodyL, color: SemanticColor.textSecondary)
                    .frame(maxWidth: 400)
                    .padding(.top, 10)
            }
        }
        .padding(.bottom, 28)
    }
}

/// Centred lines in a Gallop type role. `gallopText` sets the exact line box with `.lineHeight` on macOS 26 and later,
/// and with it a centred line that wraps is drawn about 27 points right of its frame and cut off at the frame's end
/// (Home's quiet state shows the same). So centred text takes Gallop's earlier line box, from its line spacing and
/// padding, which centres correctly.
private struct CentredText: View {
    let text: String
    let style: GallopTextStyle
    let color: Color
    @Environment(\.gallopFontProvider) private var provider

    var body: some View {
        let layout = GallopLineLayout(targetLineHeight: style.lineHeight,
                                      naturalLineHeight: provider.naturalLineHeight(for: style))
        Text(text)
            .font(provider.font(for: style))
            .tracking(style.tracking)
            .lineSpacing(layout.lineSpacing)
            .padding(.vertical, layout.verticalPadding)
            .foregroundStyle(color)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// Cowboy's sign-in card: Surface/600 with corners of 16 and no border, raised by two shadows because SwiftUI's
/// `.shadow` has no spread: Hay/600 inset 4 at the sides, 4 lower and blurred 2, and a 1-point Bison/100 at blur 1. It
/// holds 28 on every side, or for a list 28 at the sides and 16 above and below.
private struct SetupCard<Content: View>: View {
    var list = false
    @ViewBuilder let content: Content

    init(list: Bool = false, @ViewBuilder content: () -> Content) {
        self.list = list
        self.content = content()
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 16, style: .continuous)
        content
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 28)
            .padding(.vertical, list ? 16 : 28)
            .background {
                ZStack {
                    shape.fill(Palette.hay600).padding(.horizontal, 4).offset(y: 4).blur(radius: 2)
                    shape.fill(SemanticColor.surface600).shadow(color: Palette.bison100, radius: 1)
                }
            }
    }
}

/// A 40 field as Settings draws its own: Body M on a Textfield/Default capsule with a 1-point Border/Default edge and
/// 13 at the sides. A leading mark, such as the handle's "@", sits in Text/Tertiary, and a trailing one 11 from the end.
private struct SetupField<Trailing: View>: View {
    let placeholder: String
    @Binding var text: String
    var leading: String?
    @ViewBuilder let trailing: Trailing

    init(placeholder: String, text: Binding<String>, leading: String? = nil,
         @ViewBuilder trailing: () -> Trailing = { EmptyView() }) {
        self.placeholder = placeholder
        _text = text
        self.leading = leading
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: 2) {
            if let leading { Text(leading).gallopText(.bodyM, color: SemanticColor.textTertiary) }
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .accessibilityLabel(placeholder)
            trailing
        }
        .padding(.leading, 13)
        .padding(.trailing, Trailing.self == EmptyView.self ? 13 : 11)
        .frame(height: 40)
        .background(SemanticColor.textfieldDefault, in: Capsule())
        .overlay { Capsule().strokeBorder(SemanticColor.borderDefault, lineWidth: 1) }
    }
}

/// A choice beside the card's, as Cowboy's pages draw theirs: Body S in Text/Tertiary, centred, and Text/Secondary
/// underlined on hover.
private struct SetupLink: View {
    let title: String
    let action: () -> Void
    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            Text(title)
                .underline(isHovering)
                .gallopText(.bodyS, color: isHovering ? SemanticColor.textSecondary : SemanticColor.textTertiary)
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
    }
}

/// A sign-in provider's mark at 20, from `Bundled/Icons/providers`: Google's in its own colours, GitHub's in the
/// door's Text/Primary.
private struct ProviderMark: View {
    let provider: String

    var body: some View {
        Group {
            if let image = Self.image(for: provider) {
                Image(nsImage: image).resizable().scaledToFit().foregroundStyle(SemanticColor.textPrimary)
            } else {
                Color.clear
            }
        }
        .frame(width: 20, height: 20)
        .accessibilityHidden(true)
    }

    @MainActor private static var images: [String: NSImage] = [:]

    @MainActor private static func image(for provider: String) -> NSImage? {
        if let image = images[provider] { return image }
        guard let url = SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates)?
            .url(forResource: "Icons/providers/\(provider)", withExtension: "svg"),
            let image = NSImage(contentsOf: url) else { return nil }
        image.isTemplate = provider == "github"
        images[provider] = image
        return image
    }
}
