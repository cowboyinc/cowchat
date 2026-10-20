import AppKit
import Foundation

struct SocialSession: Codable, Equatable {
    let endpoint: URL
    let deviceKey: String
    let account: SocialAccount
    let scopeId: String

    init(endpoint: URL, deviceKey: String, account: SocialAccount, scopeId: String = UUID().uuidString.lowercased()) {
        self.endpoint = endpoint
        self.deviceKey = deviceKey
        self.account = account
        self.scopeId = scopeId
    }
}

@MainActor
final class SocialStore: ObservableObject {
    enum Phase: Equatable { case signedOut, awaitingBrowser, signedIn }
    enum OAuthFlow: Equatable {
        case signIn
        case link(provider: String, accountID: String)
    }
    @Published private(set) var phase: Phase = .signedOut
    @Published private(set) var endpoint: URL
    @Published private(set) var account: SocialAccount?
    @Published private(set) var providers: [String] = []
    @Published private(set) var serverNeedsUpdate = false
    @Published private(set) var identities: [SocialMe.Identity] = []
    @Published private(set) var oauthFlow: OAuthFlow?
    @Published private(set) var friends: [SocialFriend] = []
    @Published private(set) var requests: [SocialFriendRequest] = []
    @Published private(set) var agents: [SocialAgent] = []
    @Published private(set) var invitations: [SocialRoomInvitation] = []
    @Published private(set) var rooms: [SocialRoom] = []
    @Published var errorMessage: String?
    @Published var busy = false
    @Published var issuedAgent: SocialCreatedAgent?
    private(set) var savedSession: SocialSession?
    private let credentials: any CowchatCredentialStore
    private let session: URLSession
    private var generation = UUID()
    private var loginTask: Task<Void, Never>?
    private var oauthCallback: SocialOAuthCallback?
    private enum RefreshState { case idle, running, queued }
    private var refreshState = RefreshState.idle
    private var monitoringTask: Task<Void, Never>?
    var onSessionChanged: ((SocialSession?) -> Void)?
    var onRoomsChanged: (() -> Void)?
    var onSnapshot: ((SocialNotificationSnapshot) -> Void)?
    private static let credentialAccount = "cowchat-social-session-v1"

    init(endpoint: URL, credentials: any CowchatCredentialStore = KeychainCowchatCredentialStore(), session: URLSession = SocialAPI.secureSession) {
        self.endpoint = endpoint
        self.credentials = credentials
        self.session = session
        do {
            if let encoded = try credentials.credential(for: Self.credentialAccount),
               let data = Data(base64Encoded: encoded) {
                let saved = try JSONDecoder().decode(SocialSession.self, from: data)
                _ = try SocialAPI(endpoint: saved.endpoint, key: saved.deviceKey)
                guard saved.deviceKey.hasPrefix("csoc_"), UUID(uuidString: saved.scopeId) != nil else { throw CowchatConnectionError.invalidResponse }
                guard saved.endpoint == endpoint else { return }
                savedSession = saved
                account = saved.account
                phase = .signedIn
            }
        } catch { errorMessage = "Your saved account could not be loaded. \(error.localizedDescription)" }
    }

    func configureEndpoint(_ endpoint: URL) {
        guard savedSession == nil, endpoint != self.endpoint else { return }
        cancelSignIn()
        self.endpoint = endpoint
        providers = []
        serverNeedsUpdate = false
        errorMessage = nil
    }

    var availableLinkProviders: [String] {
        providers.filter { provider in !identities.contains(where: { $0.provider == provider }) }
    }

    var linkingProvider: String? {
        if case .link(let provider, _) = oauthFlow { return provider }
        return nil
    }

    private func boundAPI() throws -> SocialAPI { try SocialAPI(endpoint: endpoint, key: savedSession?.deviceKey, session: session) }

    func refresh() async {
        guard refreshState == .idle else { refreshState = .queued; return }
        defer { refreshState = .idle }
        repeat {
            refreshState = .running
            await refreshOnce()
        } while refreshState == .queued && !Task.isCancelled
    }

    func startMonitoring(interval: Duration = .seconds(30)) {
        guard monitoringTask == nil else { return }
        monitoringTask = Task { [weak self] in
            while !Task.isCancelled {
                if let self, self.savedSession != nil, !self.busy { await self.refresh() }
                do { try await Task.sleep(for: interval) } catch { break }
            }
        }
    }

    func stopMonitoring() {
        monitoringTask?.cancel()
        monitoringTask = nil
    }

    deinit { monitoringTask?.cancel() }

    private func refreshOnce() async {
        guard oauthFlow == nil, !Task.isCancelled else { return }
        let ticket = generation
        do {
            let boundAPI = try boundAPI()
            let capabilities: SocialCapabilities = try await boundAPI.social(["capabilities"])
            guard ticket == generation, !Task.isCancelled else { return }
            serverNeedsUpdate = false
            providers = capabilities.providers.filter { ["github", "google"].contains($0) }
            errorMessage = nil
            guard savedSession != nil else { return }
            let me: SocialMe = try await boundAPI.social(["me"])
            guard ticket == generation, !Task.isCancelled else { return }
            let friendList: SocialFriends = try await boundAPI.social(["friends"])
            guard ticket == generation, !Task.isCancelled else { return }
            let requestList: SocialRequests = try await boundAPI.social(["friend-requests"])
            guard ticket == generation, !Task.isCancelled else { return }
            let agentList: SocialAgents = try await boundAPI.social(["agents"])
            guard ticket == generation, !Task.isCancelled else { return }
            let invitationList: SocialInvitations = try await boundAPI.social(["room-invitations"])
            guard ticket == generation, !Task.isCancelled else { return }
            let roomList: SocialRooms = try await boundAPI.social(["rooms"])
            guard ticket == generation, !Task.isCancelled else { return }
            account = me.account
            identities = me.identities
            friends = friendList.friends
            requests = requestList.requests.filter { $0.status == "pending" }
            agents = agentList.agents
            invitations = invitationList.invitations.filter { $0.status == "pending" && $0.to.id == me.account.id }
            rooms = roomList.rooms
            errorMessage = nil
            if let savedSession {
                onSnapshot?(SocialNotificationSnapshot(session: savedSession, requests: requestList.requests, invitations: invitationList.invitations))
            }
        } catch {
            guard ticket == generation, !Task.isCancelled else { return }
            if case SocialAPIError.friendsUnavailable = error {
                serverNeedsUpdate = true
                providers = []
                errorMessage = nil
                return
            }
            errorMessage = error.localizedDescription
        }
    }

    @discardableResult
    func signIn(provider: String, open: @escaping (URL) -> Bool = { NSWorkspace.shared.open($0) }) -> Task<Void, Never>? {
        guard providers.contains(provider), phase == .signedOut else { return nil }
        return startOAuth(provider: provider, flow: .signIn, open: open)
    }

    @discardableResult
    func linkProvider(_ provider: String, open: @escaping (URL) -> Bool = { NSWorkspace.shared.open($0) }) -> Task<Void, Never>? {
        guard phase == .signedIn, !busy, oauthFlow == nil, let savedSession,
              availableLinkProviders.contains(provider) else { return nil }
        return startOAuth(provider: provider, flow: .link(provider: provider, accountID: savedSession.account.id), open: open)
    }

    private func startOAuth(provider: String, flow: OAuthFlow, open: @escaping (URL) -> Bool) -> Task<Void, Never> {
        cancelSignIn()
        let ticket = generation
        let previousSession = savedSession
        oauthFlow = flow
        if flow == .signIn { phase = .awaitingBrowser }
        errorMessage = nil
        let task = Task {
            do {
                let boundAPI = try boundAPI()
                let callback = SocialOAuthCallback()
                oauthCallback = callback
                defer { callback.cancel(); if ticket == generation { oauthCallback = nil } }
                let port = try await callback.start()
                let verifier = try SocialAPI.verifier()
                let start: SocialOAuthStart = try await boundAPI.request(["auth", "social", "start"], auth: flow != .signIn, method: "POST", body: ["provider": provider, "code_challenge": SocialAPI.challenge(verifier), "callback_port": Int(port)])
                guard ticket == generation, !Task.isCancelled else { return }
                callback.expect(requestID: start.requestId)
                guard start.authorizationUrl.scheme == "https", start.authorizationUrl.user == nil, start.authorizationUrl.password == nil,
                      start.authorizationUrl.host == (provider == "github" ? "github.com" : "accounts.google.com"),
                      start.authorizationUrl.port == nil || start.authorizationUrl.port == 443,
                      open(start.authorizationUrl) else { throw CowchatConnectionError.transport("Could not open secure browser sign-in.") }
                let completionCode = try await callback.wait(requestID: start.requestId, expiresIn: start.expiresIn)
                try Task.checkCancellation()
                let result: SocialOAuthExchange = try await boundAPI.request(["auth", "social", "exchange"], auth: false, method: "POST", body: ["request_id": start.requestId, "code_verifier": verifier, "completion_code": completionCode])
                guard ticket == generation, !Task.isCancelled else { return }
                guard result.status == "complete", let key = result.deviceKey, key.hasPrefix("csoc_"), let account = result.account else { throw CowchatConnectionError.invalidResponse }
                if case .link(_, let accountID) = flow, account.id != accountID {
                    throw CowchatConnectionError.transport("The linked provider returned a different Cowchat account. Your current sign-in was kept.")
                }
                let scopeID = previousSession.flatMap { $0.deviceKey == key ? $0.scopeId : nil } ?? UUID().uuidString.lowercased()
                let saved = SocialSession(endpoint: endpoint, deviceKey: key, account: account, scopeId: scopeID)
                try credentials.setCredential(try JSONEncoder().encode(saved).base64EncodedString(), for: Self.credentialAccount)
                savedSession = saved
                self.account = account
                phase = .signedIn
                oauthFlow = nil
                onSessionChanged?(saved)
                await refresh()
            } catch {
                guard ticket == generation, !Task.isCancelled else { return }
                phase = previousSession == nil ? .signedOut : .signedIn
                oauthFlow = nil
                errorMessage = error.localizedDescription
            }
        }
        loginTask = task
        return task
    }

    func cancelSignIn() {
        generation = UUID()
        oauthCallback?.cancel()
        oauthCallback = nil
        loginTask?.cancel()
        loginTask = nil
        oauthFlow = nil
        if phase == .awaitingBrowser { phase = .signedOut }
    }

    func signOut() async {
        let oldAPI = try? boundAPI()
        cancelSignIn()
        busy = false
        do { try credentials.setCredential(nil, for: Self.credentialAccount) }
        catch { errorMessage = error.localizedDescription; return }
        savedSession = nil
        account = nil
        identities = []
        friends = []; requests = []; agents = []; invitations = []; rooms = []; issuedAgent = nil
        phase = .signedOut
        let ticket = generation
        onSessionChanged?(nil)
        do { if let oldAPI { let _: SocialAcknowledgement = try await oldAPI.social(["logout"], method: "POST") } }
        catch { if ticket == generation { errorMessage = "Signed out on this Mac. The server could not confirm session revocation. \(error.localizedDescription)" } }
    }

    func perform(_ operation: (SocialAPI) async throws -> Void) async {
        guard !busy, phase == .signedIn, oauthFlow == nil else { return }
        let ticket = generation
        busy = true
        errorMessage = nil
        defer { if ticket == generation { busy = false } }
        do {
            try await operation(try boundAPI())
            guard ticket == generation else { return }
            await refresh()
            onRoomsChanged?()
        } catch { if ticket == generation { errorMessage = error.localizedDescription } }
    }

    func createAgent(name: String) async {
        let ticket = generation
        await perform { api in
            let created: SocialCreatedAgent = try await api.social(["agents"], method: "POST", body: ["name": name, "discoverable": true])
            guard ticket == self.generation else { return }
            self.issuedAgent = created
        }
    }

    func dismissAgentCredential() { issuedAgent = nil }
}
