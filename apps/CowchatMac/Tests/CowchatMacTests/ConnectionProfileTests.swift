import Foundation
import XCTest
@testable import CowchatMac

private enum TestCredentialStoreFailure: Error, Equatable {
    case read
    case write
}

private final class TestCredentialStore: CowchatCredentialStore {
    private(set) var credentials: [String: String] = [:]
    private(set) var setCalls: [(credential: String?, account: String)] = []
    private(set) var deletionAttempts: [String: Int] = [:]
    var readFailure: TestCredentialStoreFailure?
    var writeFailure: TestCredentialStoreFailure?
    var deletionFailuresRemaining: [String: Int] = [:]

    func credential(for account: String) throws -> String? {
        if let readFailure { throw readFailure }
        return credentials[account]
    }

    func setCredential(_ credential: String?, for account: String) throws {
        if let writeFailure { throw writeFailure }
        if credential == nil {
            deletionAttempts[account, default: 0] += 1
            let failuresRemaining = deletionFailuresRemaining[account, default: 0]
            if failuresRemaining > 0 {
                deletionFailuresRemaining[account] = failuresRemaining - 1
                throw TestCredentialStoreFailure.write
            }
        }
        setCalls.append((credential, account))
        credentials[account] = credential
    }

    func seedCredential(_ credential: String, for account: String) {
        credentials[account] = credential
    }
}

private final class TestCloudAccountIDGenerator {
    private var values: [String]

    init(_ values: [String]) {
        self.values = values
    }

    func next() -> String {
        guard !values.isEmpty else { return "not-a-valid-account-id" }
        return values.removeFirst()
    }
}

private enum TestConnectionPreferenceKeys {
    static let selectedKind = "CowchatMac.connection.selectedKind"
    static let cloudURL = "CowchatMac.connection.cloudURL"
    static let cloudAccountID = "CowchatMac.connection.cloudAccountID"
    static let pendingCloudAccountID = "CowchatMac.connection.pendingCloudAccountID"
    static let retiredCloudAccountIDs = "CowchatMac.connection.retiredCloudAccountIDs"
}

private struct TestBoundCloudCredential: Encodable {
    let endpoint: String
    let apiKey: String
}

private func makeBoundCloudCredential(endpoint: String, apiKey: String) throws -> String {
    let record = TestBoundCloudCredential(endpoint: endpoint, apiKey: apiKey)
    return "cowchat-cloud-credential-v1:"
        + (try JSONEncoder().encode(record)).base64EncodedString()
}

final class ConnectionProfileTests: XCTestCase {
    func testLocalIsTheDefaultProfile() {
        let profile = ConnectionProfile.local

        XCTAssertEqual(profile.kind, .local)
        XCTAssertEqual(profile.localHost, "127.0.0.1")
        XCTAssertEqual(profile.localPort, 9229)
        XCTAssertEqual(profile.apiKey, "")
        XCTAssertNil(profile.cloudAccountID)
        XCTAssertNil(profile.persistentIdentityScope)
        XCTAssertTrue(profile.isConnectable)
        XCTAssertEqual(profile.displayName, "Local")
        XCTAssertEqual(profile.endpointDescription, "127.0.0.1:9229")
    }

    func testCloudRequiresTLSURLAndNonemptyKey() throws {
        for insecureURL in [
            "ws://cloud.example/ws",
            "http://cloud.example/ws",
            "https://cloud.example/ws",
        ] {
            XCTAssertThrowsError(
                try ConnectionProfile.cowchatCloud(
                    urlString: insecureURL,
                    apiKey: "secret"
                )
            ) { error in
                XCTAssertEqual(error as? ConnectionProfileError, .insecureCloudURL)
            }
        }

        XCTAssertThrowsError(
            try ConnectionProfile.cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "  "
            )
        ) { error in
            XCTAssertEqual(error as? ConnectionProfileError, .missingAPIKey)
        }
    }

    func testCloudCanonicalizesHostSchemeAndDefaultTLSPort() throws {
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "WSS://CLOUD.EXAMPLE:443/ws",
            apiKey: "secret"
        )

        XCTAssertEqual(profile.endpointURL?.absoluteString, "wss://cloud.example/ws")
    }

    func testCloudRejectsCredentialsAndParametersInURL() {
        for invalidURL in [
            "wss://user@cloud.example/ws",
            "wss://cloud.example/ws?key=secret",
            "wss://cloud.example/ws#secret",
            "wss://cloud.example:0/ws",
        ] {
            XCTAssertThrowsError(
                try ConnectionProfile.cowchatCloud(
                    urlString: invalidURL,
                    apiKey: "secret"
                )
            ) { error in
                XCTAssertEqual(error as? ConnectionProfileError, .invalidCloudURL)
            }
        }
    }

    func testCloudDescriptionNeverContainsAPIKey() throws {
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "key-that-must-stay-secret"
        )

        XCTAssertEqual(profile.kind, .cowchatCloud)
        XCTAssertEqual(profile.displayName, "Cowchat Cloud")
        XCTAssertEqual(profile.endpointDescription, "wss://cloud.example/ws")
        XCTAssertFalse(profile.endpointDescription.contains(profile.apiKey))
        XCTAssertFalse(profile.description.contains(profile.apiKey))
        XCTAssertFalse(profile.debugDescription.contains(profile.apiKey))
        XCTAssertNil(profile.cloudAccountID)
        XCTAssertNil(profile.persistentIdentityScope)
        XCTAssertTrue(profile.isConnectable)
    }

    func testUnavailableCloudProfilePreservesOnlyASafeDisplayEndpoint() {
        let unavailable = ConnectionProfile.unavailableCowchatCloud(
            urlString: "  wss://cloud.example/ws  "
        )

        XCTAssertEqual(unavailable.kind, .cowchatCloud)
        XCTAssertEqual(unavailable.endpointURL?.absoluteString, "wss://cloud.example/ws")
        XCTAssertEqual(unavailable.endpointDescription, "wss://cloud.example/ws")
        XCTAssertEqual(unavailable.apiKey, "")
        XCTAssertNil(unavailable.cloudAccountID)
        XCTAssertNil(unavailable.persistentIdentityScope)
        XCTAssertFalse(unavailable.isConnectable)

        let unsafe = ConnectionProfile.unavailableCowchatCloud(
            urlString: "wss://user:secret@cloud.example/ws?key=secret"
        )
        XCTAssertNil(unsafe.endpointURL)
        XCTAssertEqual(unsafe.endpointDescription, "Not configured")
        XCTAssertFalse(unsafe.description.contains("secret"))
        XCTAssertFalse(unsafe.isConnectable)
    }

    func testUnavailableCloudProfileCannotBeSavedAsAConnection() {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        let unavailable = ConnectionProfile.unavailableCowchatCloud(
            urlString: "wss://cloud.example/ws"
        )

        XCTAssertThrowsError(try fixture.preferences.save(unavailable)) { error in
            XCTAssertEqual(error as? ConnectionProfileError, .missingAPIKey)
        }
        XCTAssertEqual(try? fixture.preferences.loadSelectedKind(), .local)
        XCTAssertTrue(fixture.credentialStore.credentials.isEmpty)
    }

    func testPreferencesDefaultToLocal() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }

        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), .local)
    }

    func testCloudRoundTripKeepsKeyOutOfUserDefaults() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        let secret = "cloud-key-never-in-defaults"
        let profile = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: secret
        )

        let saved = try fixture.preferences.save(profile)

        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), saved)
        XCTAssertNotNil(saved.cloudAccountID)
        XCTAssertEqual(
            saved.persistentIdentityScope,
            saved.cloudAccountID.map { "cowchat-cloud.\($0)" }
        )
        let persistedStrings = fixture.defaults.dictionaryRepresentation().values
            .compactMap { $0 as? String }
        XCTAssertFalse(persistedStrings.contains(secret))
        XCTAssertNotEqual(fixture.credentialStore.credentials[saved.cloudAccountID!], secret)
    }

    func testDefaultsOnlyEndpointTamperingCannotRedirectBoundCredential() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        let saved = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://trusted.example/ws",
                apiKey: "key-that-must-not-be-redirected"
            )
        )
        let protectedCredential = fixture.credentialStore.credentials[saved.cloudAccountID!]

        fixture.defaults.set(
            "wss://attacker.example/ws",
            forKey: "CowchatMac.connection.cloudURL"
        )

        XCTAssertEqual(
            fixture.preferences.loadSavedCloudURL(),
            "wss://attacker.example/ws"
        )
        XCTAssertThrowsError(try fixture.preferences.loadSelectedProfile()) { error in
            XCTAssertEqual(error as? ConnectionProfileError, .cloudEndpointBindingMismatch)
        }
        XCTAssertEqual(
            fixture.credentialStore.credentials[saved.cloudAccountID!],
            protectedCredential
        )

        fixture.defaults.set(
            "wss://trusted.example/ws",
            forKey: "CowchatMac.connection.cloudURL"
        )
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), saved)
    }

    func testLegacyUnboundCredentialFailsClosedUntilExplicitlyResaved() throws {
        let accountID = "11111111-1111-4111-8111-111111111111"
        let fixture = makePreferences(accountIDs: [
            "22222222-2222-4222-8222-222222222222",
        ])
        defer { fixture.cleanup() }
        fixture.defaults.set(
            ConnectionProfile.Kind.cowchatCloud.rawValue,
            forKey: "CowchatMac.connection.selectedKind"
        )
        fixture.defaults.set(
            "wss://legacy.example/ws",
            forKey: "CowchatMac.connection.cloudURL"
        )
        fixture.defaults.set(
            accountID,
            forKey: "CowchatMac.connection.cloudAccountID"
        )
        fixture.credentialStore.seedCredential("legacy-raw-key", for: accountID)

        XCTAssertEqual(
            fixture.preferences.loadSavedCloudURL(),
            "wss://legacy.example/ws"
        )
        XCTAssertThrowsError(try fixture.preferences.loadSelectedProfile()) { error in
            XCTAssertEqual(
                error as? ConnectionProfileError,
                .legacyCloudCredentialRequiresResave
            )
        }

        let migrated = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://legacy.example/ws",
                apiKey: "legacy-raw-key"
            )
        )
        XCTAssertEqual(migrated.cloudAccountID, accountID)
        XCTAssertNotEqual(fixture.credentialStore.credentials[accountID], "legacy-raw-key")
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), migrated)
    }

    func testSelectingLocalPreservesConfiguredCloudProfile() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        let cloud = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "switch-back-key"
        )

        let savedCloud = try fixture.preferences.save(cloud)
        try fixture.preferences.save(.local)

        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), .local)
        XCTAssertEqual(try fixture.preferences.loadConfiguredCloudProfile(), savedCloud)
    }

    func testClearingCloudConfigurationFailsBackToLocal() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "discarded-key"
            )
        )

        try fixture.preferences.clearCloudConfiguration()

        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), .local)
        XCTAssertNil(try fixture.preferences.loadConfiguredCloudProfile())
        XCTAssertTrue(fixture.credentialStore.credentials.isEmpty)
    }

    func testOpaqueCloudAccountIDIsStableUntilEndpointOrKeyChanges() throws {
        let ids = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
            "33333333-3333-4333-8333-333333333333",
        ]
        let fixture = makePreferences(accountIDs: ids)
        defer { fixture.cleanup() }

        let original = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "secret-one"
            )
        )
        let unchanged = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "secret-one"
            )
        )
        let changedKey = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "secret-two"
            )
        )
        let changedEndpoint = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://other.example/ws",
                apiKey: "secret-two"
            )
        )

        XCTAssertEqual(original.cloudAccountID, ids[0])
        XCTAssertEqual(unchanged.cloudAccountID, original.cloudAccountID)
        XCTAssertEqual(changedKey.cloudAccountID, ids[1])
        XCTAssertEqual(changedEndpoint.cloudAccountID, ids[2])
        XCTAssertNotEqual(changedKey.cloudAccountID, original.cloudAccountID)
        XCTAssertNotEqual(changedEndpoint.cloudAccountID, changedKey.cloudAccountID)
        XCTAssertFalse(changedEndpoint.cloudAccountID!.contains("secret"))
        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), [ids[2]])
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), changedEndpoint)

        // Saving unchanged credentials still executes the Keychain update path so an
        // existing item receives the current accessibility policy.
        XCTAssertEqual(fixture.credentialStore.setCalls[1].account, ids[0])
        XCTAssertNotNil(fixture.credentialStore.setCalls[1].credential)
        XCTAssertNotEqual(fixture.credentialStore.setCalls[1].credential, "secret-one")
    }

    func testPendingCredentialBeforeActivePointerIsDeletedOnLaterLoad() throws {
        let activeAccountID = "11111111-1111-4111-8111-111111111111"
        let pendingAccountID = "22222222-2222-4222-8222-222222222222"
        let fixture = makePreferences(accountIDs: [activeAccountID])
        defer { fixture.cleanup() }
        let original = try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "old-key")
        )
        fixture.credentialStore.seedCredential(
            try makeBoundCloudCredential(
                endpoint: "wss://other.example/ws",
                apiKey: "orphaned-key"
            ),
            for: pendingAccountID
        )
        fixture.defaults.set(
            pendingAccountID,
            forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
        )

        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), original)
        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), [activeAccountID])
        XCTAssertNil(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            )
        )
        XCTAssertEqual(fixture.credentialStore.deletionAttempts[pendingAccountID], 1)
    }

    func testPendingMarkerForCommittedCredentialPreservesActiveAndDeletesRetired() throws {
        let retiredAccountID = "11111111-1111-4111-8111-111111111111"
        let activeAccountID = "22222222-2222-4222-8222-222222222222"
        let fixture = makePreferences(accountIDs: [retiredAccountID])
        defer { fixture.cleanup() }
        try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "old-key")
        )
        fixture.credentialStore.seedCredential(
            try makeBoundCloudCredential(
                endpoint: "wss://other.example/ws",
                apiKey: "new-key"
            ),
            for: activeAccountID
        )
        fixture.defaults.set(
            [retiredAccountID],
            forKey: TestConnectionPreferenceKeys.retiredCloudAccountIDs
        )
        fixture.defaults.set(
            "wss://other.example/ws",
            forKey: TestConnectionPreferenceKeys.cloudURL
        )
        fixture.defaults.set(
            activeAccountID,
            forKey: TestConnectionPreferenceKeys.cloudAccountID
        )
        fixture.defaults.set(
            activeAccountID,
            forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
        )
        fixture.defaults.set(
            ConnectionProfile.Kind.cowchatCloud.rawValue,
            forKey: TestConnectionPreferenceKeys.selectedKind
        )

        let loaded = try fixture.preferences.loadSelectedProfile()

        XCTAssertEqual(loaded.endpointURL?.absoluteString, "wss://other.example/ws")
        XCTAssertEqual(loaded.apiKey, "new-key")
        XCTAssertEqual(loaded.cloudAccountID, activeAccountID)
        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), [activeAccountID])
        XCTAssertNil(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            )
        )
        XCTAssertNil(
            fixture.defaults.stringArray(
                forKey: TestConnectionPreferenceKeys.retiredCloudAccountIDs
            )
        )
        XCTAssertNil(fixture.credentialStore.deletionAttempts[activeAccountID])
        XCTAssertEqual(fixture.credentialStore.deletionAttempts[retiredAccountID], 1)
    }

    func testPendingCredentialDeletionFailureIsRetriedOnLaterLoad() throws {
        let activeAccountID = "11111111-1111-4111-8111-111111111111"
        let pendingAccountID = "22222222-2222-4222-8222-222222222222"
        let fixture = makePreferences(accountIDs: [activeAccountID])
        defer { fixture.cleanup() }
        let original = try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "old-key")
        )
        fixture.credentialStore.seedCredential(
            try makeBoundCloudCredential(
                endpoint: "wss://other.example/ws",
                apiKey: "orphaned-key"
            ),
            for: pendingAccountID
        )
        fixture.defaults.set(
            pendingAccountID,
            forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
        )
        fixture.credentialStore.deletionFailuresRemaining[pendingAccountID] = 1

        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), original)
        XCTAssertEqual(
            Set(fixture.credentialStore.credentials.keys),
            [activeAccountID, pendingAccountID]
        )
        XCTAssertEqual(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            ),
            pendingAccountID
        )
        XCTAssertEqual(fixture.credentialStore.deletionAttempts[pendingAccountID], 1)

        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), original)
        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), [activeAccountID])
        XCTAssertNil(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            )
        )
        XCTAssertEqual(fixture.credentialStore.deletionAttempts[pendingAccountID], 2)
    }

    func testChangedSaveReusesPendingAccountWhenOrphanDeletionFails() throws {
        let activeAccountID = "11111111-1111-4111-8111-111111111111"
        let pendingAccountID = "22222222-2222-4222-8222-222222222222"
        let fixture = makePreferences(accountIDs: [activeAccountID])
        defer { fixture.cleanup() }
        try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "old-key")
        )
        fixture.credentialStore.seedCredential(
            try makeBoundCloudCredential(
                endpoint: "wss://abandoned.example/ws",
                apiKey: "abandoned-key"
            ),
            for: pendingAccountID
        )
        fixture.defaults.set(
            pendingAccountID,
            forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
        )
        fixture.credentialStore.deletionFailuresRemaining[pendingAccountID] = 1

        let saved = try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://other.example/ws", apiKey: "new-key")
        )

        XCTAssertEqual(saved.cloudAccountID, pendingAccountID)
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), saved)
        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), [pendingAccountID])
        XCTAssertNil(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            )
        )
        XCTAssertNil(
            fixture.defaults.stringArray(
                forKey: TestConnectionPreferenceKeys.retiredCloudAccountIDs
            )
        )
        XCTAssertEqual(fixture.credentialStore.deletionAttempts[pendingAccountID], 1)
    }

    func testRetiredCredentialDeletionFailureIsRetriedOnLaterLoad() throws {
        let ids = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
        ]
        let fixture = makePreferences(accountIDs: ids)
        defer { fixture.cleanup() }
        try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "old-key")
        )
        fixture.credentialStore.deletionFailuresRemaining[ids[0]] = 1

        let current = try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "new-key")
        )

        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), Set(ids))
        XCTAssertEqual(
            fixture.defaults.stringArray(
                forKey: "CowchatMac.connection.retiredCloudAccountIDs"
            ),
            [ids[0]]
        )
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), current)
        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), [ids[1]])
        XCTAssertNil(
            fixture.defaults.stringArray(
                forKey: "CowchatMac.connection.retiredCloudAccountIDs"
            )
        )
        XCTAssertEqual(fixture.credentialStore.deletionAttempts[ids[0]], 2)
    }

    func testRetiredCredentialDeletionIsRetriedWhenSavingLocal() throws {
        let ids = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
        ]
        let fixture = makePreferences(accountIDs: ids)
        defer { fixture.cleanup() }
        try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "old-key")
        )
        fixture.credentialStore.deletionFailuresRemaining[ids[0]] = 1
        try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "new-key")
        )

        try fixture.preferences.save(.local)

        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), [ids[1]])
        XCTAssertNil(
            fixture.defaults.stringArray(
                forKey: "CowchatMac.connection.retiredCloudAccountIDs"
            )
        )
    }

    func testClearingCloudRetriesRetiredCredentialDeletion() throws {
        let ids = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
        ]
        let fixture = makePreferences(accountIDs: ids)
        defer { fixture.cleanup() }
        try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "old-key")
        )
        fixture.credentialStore.deletionFailuresRemaining[ids[0]] = 2
        try fixture.preferences.save(
            .cowchatCloud(urlString: "wss://cloud.example/ws", apiKey: "new-key")
        )

        try fixture.preferences.clearCloudConfiguration()

        XCTAssertTrue(fixture.credentialStore.credentials.isEmpty)
        XCTAssertNil(
            fixture.defaults.stringArray(
                forKey: "CowchatMac.connection.retiredCloudAccountIDs"
            )
        )
        XCTAssertNil(try fixture.preferences.loadConfiguredCloudProfile())
    }

    func testSelectedCloudModeRemainsObservableWhenConfigurationIsMissing() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        fixture.defaults.set(
            ConnectionProfile.Kind.cowchatCloud.rawValue,
            forKey: "CowchatMac.connection.selectedKind"
        )

        XCTAssertEqual(try fixture.preferences.loadSelectedKind(), .cowchatCloud)
        XCTAssertThrowsError(try fixture.preferences.loadSelectedProfile()) { error in
            XCTAssertEqual(error as? ConnectionProfileError, .cloudNotConfigured)
        }
    }

    func testPartialCloudConfigurationReportsItsSpecificError() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        fixture.defaults.set(
            ConnectionProfile.Kind.cowchatCloud.rawValue,
            forKey: "CowchatMac.connection.selectedKind"
        )
        fixture.defaults.set(
            "wss://cloud.example/ws",
            forKey: "CowchatMac.connection.cloudURL"
        )

        XCTAssertEqual(try fixture.preferences.loadSelectedKind(), .cowchatCloud)
        XCTAssertThrowsError(try fixture.preferences.loadSelectedProfile()) { error in
            XCTAssertEqual(error as? ConnectionProfileError, .missingCloudAccountID)
        }
    }

    func testInvalidSelectedModeDoesNotSilentlyBecomeLocal() {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        let pendingAccountID = "22222222-2222-4222-8222-222222222222"
        fixture.defaults.set("remote-ish", forKey: "CowchatMac.connection.selectedKind")
        fixture.defaults.set(
            pendingAccountID,
            forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
        )
        fixture.credentialStore.seedCredential("pending-key", for: pendingAccountID)

        XCTAssertThrowsError(try fixture.preferences.loadSelectedKind()) { error in
            XCTAssertEqual(
                error as? ConnectionProfileError,
                .invalidSelectedKind("remote-ish")
            )
        }
        XCTAssertThrowsError(try fixture.preferences.loadSelectedProfile()) { error in
            XCTAssertEqual(
                error as? ConnectionProfileError,
                .invalidSelectedKind("remote-ish")
            )
        }
        XCTAssertTrue(fixture.credentialStore.credentials.isEmpty)
        XCTAssertNil(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            )
        )
    }

    func testSelectedCloudModeSurvivesInjectedCredentialReadFailure() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        let saved = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "read-failure-key"
            )
        )
        fixture.credentialStore.readFailure = .read

        XCTAssertEqual(try fixture.preferences.loadSelectedKind(), .cowchatCloud)
        XCTAssertEqual(fixture.preferences.loadSavedCloudURL(), "wss://cloud.example/ws")
        XCTAssertThrowsError(try fixture.preferences.loadSelectedProfile()) { error in
            XCTAssertEqual(error as? TestCredentialStoreFailure, .read)
        }

        fixture.credentialStore.readFailure = nil
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), saved)
    }

    func testInjectedCredentialReadFailurePreventsProfileReplacement() throws {
        let fixture = makePreferences(accountIDs: [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
        ])
        defer { fixture.cleanup() }
        let original = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "old-key"
            )
        )
        let writesBeforeFailure = fixture.credentialStore.setCalls.count
        fixture.credentialStore.readFailure = .read

        XCTAssertThrowsError(
            try fixture.preferences.save(
                .cowchatCloud(
                    urlString: "wss://other.example/ws",
                    apiKey: "new-key"
                )
            )
        ) { error in
            XCTAssertEqual(error as? TestCredentialStoreFailure, .read)
        }
        XCTAssertEqual(fixture.credentialStore.setCalls.count, writesBeforeFailure)

        fixture.credentialStore.readFailure = nil
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), original)
    }

    func testInjectedCredentialWriteFailureLeavesExistingProfileIntact() throws {
        let accountIDs = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
        ]
        let fixture = makePreferences(accountIDs: accountIDs)
        defer { fixture.cleanup() }
        let original = try fixture.preferences.save(
            .cowchatCloud(
                urlString: "wss://cloud.example/ws",
                apiKey: "old-key"
            )
        )
        fixture.credentialStore.writeFailure = .write

        XCTAssertThrowsError(
            try fixture.preferences.save(
                .cowchatCloud(
                    urlString: "wss://other.example/ws",
                    apiKey: "new-key"
                )
            )
        ) { error in
            XCTAssertEqual(error as? TestCredentialStoreFailure, .write)
        }
        XCTAssertEqual(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            ),
            accountIDs[1]
        )

        fixture.credentialStore.writeFailure = nil
        XCTAssertEqual(try fixture.preferences.loadSelectedKind(), .cowchatCloud)
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), original)
        XCTAssertEqual(Set(fixture.credentialStore.credentials.keys), [original.cloudAccountID!])
        XCTAssertNil(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            )
        )
    }

    func testInjectedCredentialWriteFailureDoesNotCommitFirstCloudProfile() throws {
        let fixture = makePreferences()
        defer { fixture.cleanup() }
        fixture.credentialStore.writeFailure = .write

        XCTAssertThrowsError(
            try fixture.preferences.save(
                .cowchatCloud(
                    urlString: "wss://cloud.example/ws",
                    apiKey: "never-committed"
                )
            )
        ) { error in
            XCTAssertEqual(error as? TestCredentialStoreFailure, .write)
        }

        XCTAssertEqual(try fixture.preferences.loadSelectedKind(), .local)
        XCTAssertNil(try fixture.preferences.loadConfiguredCloudProfile())
        XCTAssertTrue(fixture.credentialStore.credentials.isEmpty)
        XCTAssertNotNil(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            )
        )

        fixture.credentialStore.writeFailure = nil
        XCTAssertEqual(try fixture.preferences.loadSelectedProfile(), .local)
        XCTAssertNil(
            fixture.defaults.string(
                forKey: TestConnectionPreferenceKeys.pendingCloudAccountID
            )
        )
    }

    @MainActor
    func testConnectionCanBeReconfiguredWithoutBreakingLegacyInitializer() throws {
        let connection = CowchatConnection(port: 9330)
        XCTAssertEqual(connection.profile, .local(host: "127.0.0.1", port: 9330))

        let cloud = try ConnectionProfile.cowchatCloud(
            urlString: "wss://cloud.example/ws",
            apiKey: "connection-key"
        )
        connection.reconfigure(profile: cloud)

        XCTAssertEqual(connection.profile, cloud)
    }

    private func makePreferences(
        accountIDs: [String] = ["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"]
    ) -> (
        preferences: ConnectionProfilePreferences,
        defaults: UserDefaults,
        credentialStore: TestCredentialStore,
        cleanup: () -> Void
    ) {
        let suiteName = "ConnectionProfileTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        let credentialStore = TestCredentialStore()
        let idGenerator = TestCloudAccountIDGenerator(accountIDs)
        return (
            ConnectionProfilePreferences(
                defaults: defaults,
                credentialStore: credentialStore,
                makeCloudAccountID: idGenerator.next
            ),
            defaults,
            credentialStore,
            { defaults.removePersistentDomain(forName: suiteName) }
        )
    }
}
