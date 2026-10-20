import Foundation

enum GlobalSignupError: LocalizedError, Equatable {
    case signupClosed
    case rateLimited
    case malformedResponse
    case serverError(Int)

    var errorDescription: String? {
        switch self {
        case .signupClosed:
            return "This server doesn't hand out keys automatically. Paste an API key instead."
        case .rateLimited:
            return "The server is limiting new keys from your network. Try again later or paste an existing API key."
        case .malformedResponse:
            return "The server's signup reply was malformed."
        case let .serverError(status):
            return "The server could not create an API key (HTTP \(status))."
        }
    }
}

/// Keeps local, shared global, and account global connections independent.
/// Room identifiers are scoped to the store that owns them.
@MainActor
final class WorkspaceStore: ObservableObject {
    enum Server: String, CaseIterable, Identifiable {
        case local
        case global
        case accountGlobal

        var id: String { rawValue }

        var label: String {
            switch self {
            case .local: return "Local"
            case .global: return "Global"
            case .accountGlobal: return "Global"
            }
        }
    }

    let local: ChatStore
    let notifications: SocialNotifications
    @Published private(set) var global: ChatStore?
    @Published private(set) var accountGlobal: ChatStore?
    @Published private(set) var activeServer: Server = .local
    /// Inline error for the global-server settings card. Connection failures
    /// live on the store; this only carries save/enable problems.
    @Published var globalSetupError: String?
    @Published var searchText = "" {
        didSet {
            local.searchText = searchText
            global?.searchText = searchText
            accountGlobal?.searchText = searchText
        }
    }

    private let preferences: ConnectionProfilePreferences
    private let socialCredentials: any CowchatCredentialStore
    private let defaults: UserDefaults
    private let requestAPIKey: (URL) async throws -> String
    private let makeAccountStore: (ConnectionProfile) -> ChatStore

    lazy var social: SocialStore = {
        let endpoint = URL(string: configuredGlobalValues().url) ?? URL(string: ConnectionProfile.defaultGlobalURLString)!
        let social = SocialStore(endpoint: endpoint, credentials: socialCredentials)
        social.onSessionChanged = { [weak self] session in
            self?.notifications.setSession(session)
            self?.useSocialSession(session)
        }
        social.onSnapshot = { [weak self] snapshot in self?.notifications.receive(snapshot) }
        social.onRoomsChanged = { [weak self] in
            guard let self, self.social.savedSession != nil else { return }
            Task { try? await self.accountGlobal?.refreshRooms() }
        }
        return social
    }()

    func useSocialSession(_ session: SocialSession?) {
        accountGlobal?.shutdownForRemoval()
        accountGlobal = nil
        if let session, let profile = try? ConnectionProfile.socialSession(session) {
            let store = makeAccountStore(profile)
            store.searchText = searchText
            accountGlobal = store
            store.start()
        }
        if activeServer == .accountGlobal, accountGlobal == nil {
            activeServer = global == nil ? .local : .global
        }
    }

    var activeStore: ChatStore {
        switch activeServer {
        case .local: return local
        case .global: return global ?? local
        case .accountGlobal: return accountGlobal ?? local
        }
    }

    var isGlobalEnabled: Bool { global != nil }

    init(
        local: ChatStore,
        preferences: ConnectionProfilePreferences,
        defaults: UserDefaults = .standard,
        global: ChatStore? = nil,
        accountGlobal: ChatStore? = nil,
        makeAccountStore: ((ConnectionProfile) -> ChatStore)? = nil,
        socialCredentials: any CowchatCredentialStore = KeychainCowchatCredentialStore(),
        requestAPIKey: @escaping (URL) async throws -> String = WorkspaceStore.requestAPIKeyOverHTTP
    ) {
        self.local = local
        self.notifications = SocialNotifications(defaults: defaults)
        self.preferences = preferences
        self.socialCredentials = socialCredentials
        self.defaults = defaults
        self.global = global
        self.accountGlobal = accountGlobal
        self.makeAccountStore = makeAccountStore ?? { profile in
            ChatStore(connection: CowchatConnection(profile: profile), defaults: defaults, connectionProfile: profile)
        }
        self.requestAPIKey = requestAPIKey
    }

    convenience init() {
        let defaults = UserDefaults.standard
        let preferences = ConnectionProfilePreferences(
            defaults: defaults,
            credentialStore: KeychainCowchatCredentialStore()
        )
        #if DEBUG
        // Dev builds run as a bare `swift build` binary with no
        // Contents/Helpers, so allow the fixed Homebrew install locations as
        // helper fallbacks. Deliberately empty in release: the shipped app
        // only ever spawns the helper it was signed with.
        let devFallbackServerPaths = [
            "/opt/homebrew/bin/cowchat-server",
            "/usr/local/bin/cowchat-server",
        ]
        #else
        let devFallbackServerPaths: [String] = []
        #endif
        let local = ChatStore(
            connection: CowchatConnection(profile: .local),
            defaults: defaults,
            connectionProfile: .local,
            localServerSupervisor: LocalServerSupervisor(
                devFallbackServerPaths: devFallbackServerPaths
            )
        )
        var global: ChatStore?
        if preferences.isGlobalEnabled() {
            global = Self.makeGlobalStore(preferences: preferences, defaults: defaults)
            if global == nil {
                // Enabled but nothing stored at all: an interrupted first-time
                // setup. Fall back to off rather than showing a broken server.
                preferences.setGlobalEnabled(false)
            }
        }
        self.init(local: local, preferences: preferences, defaults: defaults, global: global)
    }

    func start() {
        local.start()
        notifications.setSession(social.savedSession)
        social.startMonitoring()
        global?.start()
        if let session = social.savedSession { useSocialSession(session) }
    }

    func shutdownForAppTermination() async {
        social.stopMonitoring()
        await local.shutdownOwnedLocalServerForAppTermination()
    }

    func store(for server: Server) -> ChatStore? {
        switch server {
        case .local: return local
        case .global: return global
        case .accountGlobal: return accountGlobal
        }
    }

    func isSelected(_ room: Room, on server: Server) -> Bool {
        activeServer == server && store(for: server)?.selectedRoomID == room.id
    }

    /// Makes a server's selection the one the chat pane shows (e.g. after
    /// creating a room on the non-active server).
    func activate(server: Server) {
        guard store(for: server) != nil else { return }
        activeServer = server
    }

    func select(room: Room, on server: Server) async {
        guard let store = store(for: server) else { return }
        activeServer = server
        await store.select(room: room)
    }

    /// Saved global config for prefilling the settings fields. Falls back to
    /// Cowboy's well-known server when nothing is configured yet.
    func configuredGlobalValues() -> (url: String, apiKey: String) {
        do {
            guard let profile = try preferences.loadConfiguredCloudProfile() else {
                return (savedOrDefaultGlobalURL(), "")
            }
            return (profile.endpointURL?.absoluteString ?? savedOrDefaultGlobalURL(), profile.apiKey)
        } catch {
            return (savedOrDefaultGlobalURL(), "")
        }
    }

    /// Connects to a global server, minting an API key from its self-serve
    /// signup endpoint when the user left the key field empty.
    @discardableResult
    func connectToGlobal(url: String, apiKey: String) async -> Bool {
        let trimmedKey = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmedKey.isEmpty else {
            return saveGlobalConfiguration(url: url, apiKey: trimmedKey)
        }
        globalSetupError = nil
        guard let signupURL = Self.signupURL(forCloudURLString: url) else {
            globalSetupError = ConnectionProfileError.invalidCloudURL.localizedDescription
            return false
        }
        do {
            let mintedKey = try await requestAPIKey(signupURL)
            return saveGlobalConfiguration(url: url, apiKey: mintedKey)
        } catch {
            globalSetupError = error.localizedDescription
            return false
        }
    }

    /// `wss://host[:port]/…` → `https://host[:port]/api/keys`.
    nonisolated static func signupURL(forCloudURLString urlString: String) -> URL? {
        apiURL(forCloudURLString: urlString, path: "/api/keys")
    }

    /// `wss://host[:port]/…` → `https://host[:port]/api/invites/redeem`.
    nonisolated static func inviteRedeemURL(forCloudURLString urlString: String) -> URL? {
        apiURL(forCloudURLString: urlString, path: "/api/invites/redeem")
    }

    /// `wss://host[:port]/…` → `https://host[:port]/api/blobs/<id>`.
    nonisolated static func blobURL(forCloudURLString urlString: String, blobID: String) -> URL? {
        guard blobID.range(of: "^[A-Za-z0-9-]+$", options: .regularExpression) != nil else {
            return nil
        }
        return apiURL(forCloudURLString: urlString, path: "/api/blobs/\(blobID)")
    }

    private nonisolated static func apiURL(
        forCloudURLString urlString: String,
        path: String
    ) -> URL? {
        guard let profile = try? ConnectionProfile.cowchatCloud(
            urlString: urlString,
            apiKey: "placeholder-for-validation"
        ), let endpointURL = profile.endpointURL,
            var components = URLComponents(url: endpointURL, resolvingAgainstBaseURL: false)
        else { return nil }
        components.scheme = "https"
        components.path = path
        return components.url
    }

    nonisolated static func requestAPIKeyOverHTTP(_ signupURL: URL) async throws -> String {
        var request = URLRequest(url: signupURL)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: ["label": "Cowchat Mac"])
        request.timeoutInterval = 15
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        switch status {
        case 201:
            guard let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let key = payload["api_key"] as? String, !key.isEmpty else {
                throw GlobalSignupError.malformedResponse
            }
            return key
        case 401, 403:
            throw GlobalSignupError.signupClosed
        case 429:
            throw GlobalSignupError.rateLimited
        default:
            throw GlobalSignupError.serverError(status)
        }
    }

    @discardableResult
    func saveGlobalConfiguration(url: String, apiKey: String) -> Bool {
        globalSetupError = nil
        do {
            let candidate = try ConnectionProfile.cowchatCloud(urlString: url, apiKey: apiKey)
            if let session = social.savedSession, candidate.endpointURL != session.endpoint {
                globalSetupError = "Sign out of your Cowchat account before switching servers. Your shared global connection is unchanged."
                return false
            }
            if let global {
                guard global.saveAndUseCowchatCloud(url: url, apiKey: apiKey) else {
                    globalSetupError = global.errorMessage
                    return false
                }
                social.configureEndpoint(candidate.endpointURL!)
                return true
            }
            let saved = try preferences.save(candidate)
            attachGlobalStore(profile: saved, configurationError: nil)
            social.configureEndpoint(saved.endpointURL!)
            return true
        } catch {
            globalSetupError = error.localizedDescription
            return false
        }
    }

    func disableGlobalRooms() {
        globalSetupError = nil
        preferences.setGlobalEnabled(false)
        if activeServer == .global { activeServer = accountGlobal == nil ? .local : .accountGlobal }
        global?.shutdownForRemoval()
        global = nil
    }

    private func attachGlobalStore(profile: ConnectionProfile, configurationError: Error?) {
        let store = ChatStore(
            connection: CowchatConnection(profile: profile),
            defaults: defaults,
            connectionProfile: profile,
            connectionPreferences: preferences,
            connectionConfigurationError: configurationError
        )
        global = store
        store.searchText = searchText
        store.start()
    }

    private static func makeGlobalStore(
        preferences: ConnectionProfilePreferences,
        defaults: UserDefaults
    ) -> ChatStore? {
        let profile: ConnectionProfile
        let configurationError: Error?
        do {
            guard let loaded = try preferences.loadConfiguredCloudProfile() else { return nil }
            profile = loaded
            configurationError = nil
        } catch {
            // Keep the server visible with its saved endpoint; reconnect is
            // the user-approved point to retry the Keychain read.
            configurationError = error
            profile = .unavailableCowchatCloud(urlString: preferences.loadSavedCloudURL())
        }
        return ChatStore(
            connection: CowchatConnection(profile: profile),
            defaults: defaults,
            connectionProfile: profile,
            connectionPreferences: preferences,
            connectionConfigurationError: configurationError
        )
    }

    private func savedOrDefaultGlobalURL() -> String {
        preferences.loadSavedCloudURL() ?? ConnectionProfile.defaultGlobalURLString
    }

}
