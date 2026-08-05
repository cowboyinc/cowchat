import Foundation
import Security

enum ConnectionProfileError: LocalizedError, Equatable {
    case invalidCloudURL
    case insecureCloudURL
    case missingAPIKey
    case cloudNotConfigured
    case missingCloudURL
    case missingCloudAccountID
    case invalidCloudAccountID
    case invalidSelectedKind(String)
    case cloudAccountIDGenerationFailed
    case cloudAccountIDPersistenceFailed
    case cloudEndpointBindingMismatch
    case legacyCloudCredentialRequiresResave

    var errorDescription: String? {
        switch self {
        case .invalidCloudURL:
            return "Enter a valid Cowchat Cloud WebSocket URL."
        case .insecureCloudURL:
            return "Cowchat Cloud requires a secure wss:// URL."
        case .missingAPIKey:
            return "Enter a Cowchat Cloud API key."
        case .cloudNotConfigured:
            return "Cowchat Cloud has not been configured on this Mac."
        case .missingCloudURL:
            return "The saved Cowchat Cloud URL is missing."
        case .missingCloudAccountID:
            return "The saved Cowchat Cloud account identifier is missing."
        case .invalidCloudAccountID:
            return "The saved Cowchat Cloud account identifier is invalid."
        case let .invalidSelectedKind(kind):
            return "The saved Cowchat connection mode is invalid: \(kind)."
        case .cloudAccountIDGenerationFailed:
            return "Cowchat could not create a secure Cloud account identifier."
        case .cloudAccountIDPersistenceFailed:
            return "Cowchat could not safely persist its Cloud account identifier."
        case .cloudEndpointBindingMismatch:
            return "The saved Cowchat Cloud endpoint does not match its secure credential. Re-enter the Cloud URL and API key."
        case .legacyCloudCredentialRequiresResave:
            return "Re-enter your Cowchat Cloud API key to secure it to this endpoint."
        }
    }
}

struct ConnectionProfile: Equatable, CustomStringConvertible, CustomDebugStringConvertible {
    enum Kind: String, Codable, CaseIterable {
        case local
        case cowchatCloud
    }

    static let local = ConnectionProfile(
        kind: .local,
        localHost: "127.0.0.1",
        localPort: 9229,
        endpointURL: nil,
        apiKey: "",
        cloudAccountID: nil
    )

    let kind: Kind
    let localHost: String?
    let localPort: UInt16?
    let endpointURL: URL?
    let apiKey: String
    /// An opaque, randomly generated identifier assigned when a Cloud profile is saved.
    /// It is deliberately independent of the endpoint and API key.
    let cloudAccountID: String?

    /// A non-secret namespace suitable for Cloud-specific agent IDs and local UI state.
    /// Local keeps the legacy unscoped namespace for compatibility.
    var persistentIdentityScope: String? {
        guard kind == .cowchatCloud, let cloudAccountID else { return nil }
        return "cowchat-cloud.\(cloudAccountID)"
    }

    var isConnectable: Bool {
        switch kind {
        case .local:
            return true
        case .cowchatCloud:
            return endpointURL != nil
                && !apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    var displayName: String {
        switch kind {
        case .local: return "Local"
        case .cowchatCloud: return "Cowchat Cloud"
        }
    }

    var endpointDescription: String {
        switch kind {
        case .local:
            return "\(localHost ?? "127.0.0.1"):\(localPort ?? 9229)"
        case .cowchatCloud:
            return endpointURL?.absoluteString ?? "Not configured"
        }
    }

    var description: String {
        "\(displayName) (\(endpointDescription))"
    }

    var debugDescription: String { description }

    static func local(host: String, port: UInt16) -> ConnectionProfile {
        ConnectionProfile(
            kind: .local,
            localHost: host,
            localPort: port,
            endpointURL: nil,
            apiKey: "",
            cloudAccountID: nil
        )
    }

    static func cowchatCloud(urlString: String, apiKey: String) throws -> ConnectionProfile {
        guard let url = validatedCloudURL(urlString) else {
            let trimmedURL = urlString.trimmingCharacters(in: .whitespacesAndNewlines)
            if let parsedURL = URL(string: trimmedURL),
               parsedURL.host?.isEmpty == false,
               parsedURL.scheme?.lowercased() != "wss" {
                throw ConnectionProfileError.insecureCloudURL
            }
            throw ConnectionProfileError.invalidCloudURL
        }
        guard !apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw ConnectionProfileError.missingAPIKey
        }
        return ConnectionProfile(
            kind: .cowchatCloud,
            localHost: nil,
            localPort: nil,
            endpointURL: url,
            apiKey: apiKey,
            cloudAccountID: nil
        )
    }

    /// Represents a selected Cloud connection whose secure configuration could not be
    /// loaded. It preserves the selected mode for UI state but cannot be connected.
    static func unavailableCowchatCloud(urlString: String?) -> ConnectionProfile {
        ConnectionProfile(
            kind: .cowchatCloud,
            localHost: nil,
            localPort: nil,
            endpointURL: urlString.flatMap(validatedCloudURL),
            apiKey: "",
            cloudAccountID: nil
        )
    }

    fileprivate func persisted(cloudAccountID: String) throws -> ConnectionProfile {
        guard kind == .cowchatCloud else { return self }
        guard let normalizedID = Self.normalizedCloudAccountID(cloudAccountID) else {
            throw ConnectionProfileError.invalidCloudAccountID
        }
        return ConnectionProfile(
            kind: kind,
            localHost: localHost,
            localPort: localPort,
            endpointURL: endpointURL,
            apiKey: apiKey,
            cloudAccountID: normalizedID
        )
    }

    fileprivate static func normalizedCloudAccountID(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let uuid = UUID(uuidString: trimmed) else { return nil }
        return uuid.uuidString.lowercased()
    }

    fileprivate static func validatedCloudURL(_ urlString: String) -> URL? {
        let trimmedURL = urlString.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var components = URLComponents(string: trimmedURL),
              let host = components.host,
              !host.isEmpty,
              components.scheme?.lowercased() == "wss",
              components.user == nil,
              components.password == nil,
              components.query == nil,
              components.fragment == nil,
              components.port != 0 else {
            return nil
        }
        components.scheme = "wss"
        components.host = host.lowercased()
        if components.port == 443 { components.port = nil }
        return components.url
    }

    private init(
        kind: Kind,
        localHost: String?,
        localPort: UInt16?,
        endpointURL: URL?,
        apiKey: String,
        cloudAccountID: String?
    ) {
        self.kind = kind
        self.localHost = localHost
        self.localPort = localPort
        self.endpointURL = endpointURL
        self.apiKey = apiKey
        self.cloudAccountID = cloudAccountID
    }
}

protocol CowchatCredentialStore {
    func credential(for account: String) throws -> String?
    func setCredential(_ credential: String?, for account: String) throws
}

enum CowchatCredentialStoreError: LocalizedError, Equatable {
    case keychain(OSStatus)
    case invalidCredentialData

    var errorDescription: String? {
        switch self {
        case .keychain:
            return "Cowchat could not access this Mac's secure credential store."
        case .invalidCredentialData:
            return "The saved Cowchat Cloud credential is invalid."
        }
    }
}

struct KeychainCowchatCredentialStore: CowchatCredentialStore {
    private let service: String

    init(service: String = "inc.cowboy.cowchat.credentials") {
        self.service = service
    }

    func credential(for account: String) throws -> String? {
        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else {
            throw CowchatCredentialStoreError.keychain(status)
        }
        guard let data = result as? Data,
              let credential = String(data: data, encoding: .utf8) else {
            throw CowchatCredentialStoreError.invalidCredentialData
        }
        return credential
    }

    func setCredential(_ credential: String?, for account: String) throws {
        let query = baseQuery(account: account)
        guard let credential else {
            let status = SecItemDelete(query as CFDictionary)
            guard status == errSecSuccess || status == errSecItemNotFound else {
                throw CowchatCredentialStoreError.keychain(status)
            }
            return
        }

        let data = Data(credential.utf8)
        let updateStatus = SecItemUpdate(
            query as CFDictionary,
            [
                kSecValueData as String: data,
                kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            ] as CFDictionary
        )
        if updateStatus == errSecSuccess { return }
        guard updateStatus == errSecItemNotFound else {
            throw CowchatCredentialStoreError.keychain(updateStatus)
        }

        var attributes = query
        attributes[kSecValueData as String] = data
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        let addStatus = SecItemAdd(attributes as CFDictionary, nil)
        guard addStatus == errSecSuccess else {
            throw CowchatCredentialStoreError.keychain(addStatus)
        }
    }

    private func baseQuery(account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any,
        ]
    }
}

final class ConnectionProfilePreferences {
    private struct BoundCloudCredential: Codable {
        let endpoint: String
        let apiKey: String
    }

    private enum DecodedCloudCredential {
        case bound(endpointURL: URL, apiKey: String)
        case legacy(apiKey: String)
    }

    private enum Keys {
        static let selectedKind = "CowchatMac.connection.selectedKind"
        static let cloudURL = "CowchatMac.connection.cloudURL"
        static let cloudAccountID = "CowchatMac.connection.cloudAccountID"
        static let pendingCloudAccountID = "CowchatMac.connection.pendingCloudAccountID"
        static let retiredCloudAccountIDs = "CowchatMac.connection.retiredCloudAccountIDs"
    }

    private static let boundCredentialPrefix = "cowchat-cloud-credential-v1:"

    private let defaults: UserDefaults
    private let credentialStore: any CowchatCredentialStore
    private let makeCloudAccountID: () -> String

    init(
        defaults: UserDefaults = .standard,
        credentialStore: any CowchatCredentialStore = KeychainCowchatCredentialStore(),
        makeCloudAccountID: @escaping () -> String = { UUID().uuidString.lowercased() }
    ) {
        self.defaults = defaults
        self.credentialStore = credentialStore
        self.makeCloudAccountID = makeCloudAccountID
    }

    func loadSelectedKind() throws -> ConnectionProfile.Kind {
        guard let rawKind = defaults.string(forKey: Keys.selectedKind) else {
            return .local
        }
        guard let kind = ConnectionProfile.Kind(rawValue: rawKind) else {
            throw ConnectionProfileError.invalidSelectedKind(rawKind)
        }
        return kind
    }

    func loadSelectedProfile() throws -> ConnectionProfile {
        let activeAccountID = storedCloudAccountID()
        retryPendingCredentialCleanup(excluding: activeAccountID)
        retryRetiredCredentialCleanup(excluding: activeAccountID)
        let selectedKind = try loadSelectedKind()
        guard selectedKind == .cowchatCloud else {
            return .local
        }
        guard let profile = try loadConfiguredCloudProfile(retryingCleanup: false) else {
            throw ConnectionProfileError.cloudNotConfigured
        }
        return profile
    }

    /// Reads the non-secret endpoint without touching Keychain. This lets startup keep
    /// Cloud visibly selected when its credential is unavailable or malformed.
    func loadSavedCloudURL() -> String? {
        guard let value = defaults.string(forKey: Keys.cloudURL) else { return nil }
        return ConnectionProfile.validatedCloudURL(value)?.absoluteString
    }

    func loadConfiguredCloudProfile() throws -> ConnectionProfile? {
        try loadConfiguredCloudProfile(retryingCleanup: true)
    }

    private func loadConfiguredCloudProfile(
        retryingCleanup: Bool
    ) throws -> ConnectionProfile? {
        let storedURL = defaults.string(forKey: Keys.cloudURL)
        let storedAccountID = defaults.string(forKey: Keys.cloudAccountID)
        let accountID = storedAccountID.flatMap(ConnectionProfile.normalizedCloudAccountID)
        if retryingCleanup {
            retryPendingCredentialCleanup(excluding: accountID)
            retryRetiredCredentialCleanup(excluding: accountID)
        }
        guard storedURL != nil || storedAccountID != nil else {
            return nil
        }
        guard let urlString = storedURL,
              let displayEndpointURL = ConnectionProfile.validatedCloudURL(urlString) else {
            throw ConnectionProfileError.missingCloudURL
        }
        guard storedAccountID != nil else {
            throw ConnectionProfileError.missingCloudAccountID
        }
        guard let accountID else {
            throw ConnectionProfileError.invalidCloudAccountID
        }
        guard let storedCredential = try credentialStore.credential(for: accountID) else {
            throw ConnectionProfileError.missingAPIKey
        }
        let endpointURL: URL
        let apiKey: String
        switch try decodeCredential(storedCredential) {
        case let .bound(boundEndpointURL, boundAPIKey):
            guard boundEndpointURL.absoluteString == displayEndpointURL.absoluteString else {
                throw ConnectionProfileError.cloudEndpointBindingMismatch
            }
            endpointURL = boundEndpointURL
            apiKey = boundAPIKey
        case .legacy:
            // The in-flight implementation stored only the key. There is no protected
            // evidence that the defaults URL is its intended endpoint, so require an
            // explicit re-save instead of silently trusting a redirectable preference.
            throw ConnectionProfileError.legacyCloudCredentialRequiresResave
        }
        return try ConnectionProfile
            .cowchatCloud(urlString: endpointURL.absoluteString, apiKey: apiKey)
            .persisted(cloudAccountID: accountID)
    }

    /// Persists a profile and returns its canonical saved representation. Cloud callers
    /// must use the returned value because it contains the stable opaque account ID.
    @discardableResult
    func save(_ profile: ConnectionProfile) throws -> ConnectionProfile {
        switch profile.kind {
        case .local:
            let activeAccountID = storedCloudAccountID()
            retryPendingCredentialCleanup(excluding: activeAccountID)
            retryRetiredCredentialCleanup(excluding: activeAccountID)
            defaults.set(ConnectionProfile.Kind.local.rawValue, forKey: Keys.selectedKind)
            return .local
        case .cowchatCloud:
            guard let endpointURL = profile.endpointURL else {
                throw ConnectionProfileError.invalidCloudURL
            }
            guard !profile.apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw ConnectionProfileError.missingAPIKey
            }

            let activeAccountID = storedCloudAccountID()
            retryPendingCredentialCleanup(excluding: activeAccountID)
            retryRetiredCredentialCleanup(excluding: activeAccountID)
            let previous = try loadCloudRecordForSave()
            let unchanged = previous?.endpointURL == endpointURL
                && previous?.apiKey == profile.apiKey
            let accountID: String
            if unchanged, let previousAccountID = previous?.accountID {
                accountID = previousAccountID
            } else {
                accountID = try preparePendingCloudAccountID(
                    excluding: previous?.accountID
                )
            }

            // The endpoint and key are one Keychain-protected record. UserDefaults is
            // only a display value and pointer, so changing it cannot redirect the key.
            let boundCredential = try encodeCredential(
                endpointURL: endpointURL,
                apiKey: profile.apiKey
            )
            try credentialStore.setCredential(boundCredential, for: accountID)
            if let previousAccountID = previous?.accountID, previousAccountID != accountID {
                // Record retirement before changing the active pointer. If the process
                // exits between writes, cleanup excludes whichever account is active.
                addRetiredCloudAccountID(previousAccountID)
                try synchronizeDefaultsForCloudAccountID()
            }
            defaults.set(endpointURL.absoluteString, forKey: Keys.cloudURL)
            defaults.set(accountID, forKey: Keys.cloudAccountID)
            defaults.set(ConnectionProfile.Kind.cowchatCloud.rawValue, forKey: Keys.selectedKind)
            try synchronizeDefaultsForCloudAccountID()
            retryPendingCredentialCleanup(excluding: accountID)
            retryRetiredCredentialCleanup(excluding: accountID)
            _ = defaults.synchronize()
            return try profile.persisted(cloudAccountID: accountID)
        }
    }

    func clearCloudConfiguration() throws {
        let accountID = storedCloudAccountID()
        retryPendingCredentialCleanup(excluding: accountID)
        retryRetiredCredentialCleanup(excluding: accountID)
        if let accountID {
            // Delete first so a Keychain failure leaves a loadable configuration.
            try credentialStore.setCredential(nil, for: accountID)
        }
        defaults.removeObject(forKey: Keys.cloudURL)
        defaults.removeObject(forKey: Keys.cloudAccountID)
        if defaults.string(forKey: Keys.selectedKind) == ConnectionProfile.Kind.cowchatCloud.rawValue {
            defaults.set(ConnectionProfile.Kind.local.rawValue, forKey: Keys.selectedKind)
        }
        retryRetiredCredentialCleanup(excluding: nil)
        retryPendingCredentialCleanup(excluding: nil)
    }

    private func loadCloudRecordForSave() throws -> (
        endpointURL: URL?,
        apiKey: String?,
        accountID: String
    )? {
        guard let storedAccountID = defaults.string(forKey: Keys.cloudAccountID),
              let accountID = ConnectionProfile.normalizedCloudAccountID(storedAccountID) else {
            return nil
        }
        let storedCredential = try credentialStore.credential(for: accountID)
        let endpointURL: URL?
        let apiKey: String?
        if let storedCredential {
            switch try decodeCredential(storedCredential) {
            case let .bound(boundEndpointURL, boundAPIKey):
                endpointURL = boundEndpointURL
                apiKey = boundAPIKey
            case let .legacy(legacyAPIKey):
                endpointURL = defaults.string(forKey: Keys.cloudURL)
                    .flatMap(ConnectionProfile.validatedCloudURL)
                apiKey = legacyAPIKey
            }
        } else {
            endpointURL = nil
            apiKey = nil
        }
        return (endpointURL, apiKey, accountID)
    }

    private func encodeCredential(endpointURL: URL, apiKey: String) throws -> String {
        let record = BoundCloudCredential(
            endpoint: endpointURL.absoluteString,
            apiKey: apiKey
        )
        let data = try JSONEncoder().encode(record)
        return Self.boundCredentialPrefix + data.base64EncodedString()
    }

    private func decodeCredential(_ value: String) throws -> DecodedCloudCredential {
        guard value.hasPrefix(Self.boundCredentialPrefix) else {
            return .legacy(apiKey: value)
        }
        let encoded = value.dropFirst(Self.boundCredentialPrefix.count)
        guard let data = Data(base64Encoded: String(encoded)),
              let record = try? JSONDecoder().decode(BoundCloudCredential.self, from: data),
              let endpointURL = ConnectionProfile.validatedCloudURL(record.endpoint),
              endpointURL.absoluteString == record.endpoint,
              !record.apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw CowchatCredentialStoreError.invalidCredentialData
        }
        return .bound(endpointURL: endpointURL, apiKey: record.apiKey)
    }

    private func storedCloudAccountID() -> String? {
        defaults.string(forKey: Keys.cloudAccountID)
            .flatMap(ConnectionProfile.normalizedCloudAccountID)
    }

    private func storedPendingCloudAccountID() -> String? {
        defaults.string(forKey: Keys.pendingCloudAccountID)
            .flatMap(ConnectionProfile.normalizedCloudAccountID)
    }

    private func preparePendingCloudAccountID(
        excluding previousAccountID: String?
    ) throws -> String {
        let accountID: String
        if let pendingAccountID = storedPendingCloudAccountID(),
           pendingAccountID != previousAccountID {
            accountID = pendingAccountID
        } else {
            accountID = try nextCloudAccountID(excluding: previousAccountID)
            defaults.set(accountID, forKey: Keys.pendingCloudAccountID)
        }
        try synchronizeDefaultsForCloudAccountID()
        return accountID
    }

    private func synchronizeDefaultsForCloudAccountID() throws {
        guard defaults.synchronize() else {
            throw ConnectionProfileError.cloudAccountIDPersistenceFailed
        }
    }

    private func retryPendingCredentialCleanup(excluding activeAccountID: String?) {
        guard let stored = defaults.string(forKey: Keys.pendingCloudAccountID) else { return }
        guard let pendingAccountID = ConnectionProfile.normalizedCloudAccountID(stored) else {
            defaults.removeObject(forKey: Keys.pendingCloudAccountID)
            return
        }
        if pendingAccountID == activeAccountID {
            // The active pointer committed before the transaction marker was
            // cleared. The credential is live and must not be deleted.
            defaults.removeObject(forKey: Keys.pendingCloudAccountID)
            return
        }
        do {
            try credentialStore.setCredential(nil, for: pendingAccountID)
            defaults.removeObject(forKey: Keys.pendingCloudAccountID)
        } catch {
            // Keep the marker so a later load/save/clear can retry. A subsequent
            // changed-profile save may safely reuse and overwrite this account.
        }
    }

    private func addRetiredCloudAccountID(_ accountID: String) {
        guard let accountID = ConnectionProfile.normalizedCloudAccountID(accountID) else { return }
        var retired = Set(defaults.stringArray(forKey: Keys.retiredCloudAccountIDs) ?? [])
        retired.insert(accountID)
        defaults.set(retired.sorted(), forKey: Keys.retiredCloudAccountIDs)
    }

    private func retryRetiredCredentialCleanup(excluding activeAccountID: String?) {
        let stored = defaults.stringArray(forKey: Keys.retiredCloudAccountIDs) ?? []
        guard !stored.isEmpty else { return }

        var remaining: Set<String> = []
        for value in stored {
            guard let accountID = ConnectionProfile.normalizedCloudAccountID(value) else {
                continue
            }
            if accountID == activeAccountID {
                remaining.insert(accountID)
                continue
            }
            do {
                try credentialStore.setCredential(nil, for: accountID)
            } catch {
                remaining.insert(accountID)
            }
        }

        if remaining.isEmpty {
            defaults.removeObject(forKey: Keys.retiredCloudAccountIDs)
        } else {
            defaults.set(remaining.sorted(), forKey: Keys.retiredCloudAccountIDs)
        }
    }

    private func nextCloudAccountID(excluding previousAccountID: String?) throws -> String {
        for _ in 0..<4 {
            guard let candidate = ConnectionProfile.normalizedCloudAccountID(makeCloudAccountID()) else {
                continue
            }
            if candidate != previousAccountID { return candidate }
        }
        throw ConnectionProfileError.cloudAccountIDGenerationFailed
    }
}
