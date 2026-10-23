import Foundation
import XCTest
@testable import CowchatMac

private final class MigrationCredentials: CowchatCredentialStore {
    var values: [String: String] = [:]
    var failWrite: Int?
    var writes = 0
    func credential(for account: String) throws -> String? { values[account] }
    func accounts(withPrefix prefix: String) throws -> [String] { values.keys.filter { $0.hasPrefix(prefix) }.sorted() }
    func setCredential(_ value: String?, for account: String) throws {
        writes += 1
        if writes == failWrite { throw CowchatCredentialStoreError.keychain(-1) }
        values[account] = value
    }
}

private final class MigrationHTTP: URLProtocol {
    static var handler: ((URLRequest) throws -> (Int, String))!
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, body) = try Self.handler(request)
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data(body.utf8))
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

@MainActor
final class CompanyEndpointMigrationTests: XCTestCase {
    private let old = ConnectionProfile.legacyGlobalURLString
    private let new = ConnectionProfile.defaultGlobalURLString
    private let id = "11111111-1111-4111-8111-111111111111"
    private let socialID = "22222222-2222-4222-8222-222222222222"
    private var defaults: UserDefaults!
    private var suite: String!
    private var credentials: MigrationCredentials!
    private var preferences: ConnectionProfilePreferences!
    private var session: URLSession!

    override func setUp() async throws {
        suite = "CompanyMigrationTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)!
        credentials = MigrationCredentials()
        preferences = ConnectionProfilePreferences(defaults: defaults, credentialStore: credentials, makeCloudAccountID: { self.id })
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MigrationHTTP.self]
        session = URLSession(configuration: config, delegate: SocialHTTPDelegate(), delegateQueue: nil)
        MigrationHTTP.handler = { request in
            XCTAssertEqual(request.url?.scheme, "https")
            XCTAssertEqual(request.url?.host, "chat.cowchat.cowboylabs.net")
            XCTAssertNotNil(request.value(forHTTPHeaderField: "x-cowchat-key"))
            if request.url?.path == "/social/v1/me" {
                return (200, #"{"account":{"id":"person","handle":"martin","display_name":"Martin"},"identities":[]}"#)
            }
            XCTAssertEqual(request.url?.path, "/api/agents")
            return (200, #"{"agents":[]}"#)
        }
    }

    override func tearDown() async throws {
        session.invalidateAndCancel()
        defaults.removePersistentDomain(forName: suite)
        MigrationHTTP.handler = nil
    }

    private func seed(social: Bool = true, cloud: Bool = true) throws -> [ConnectionProfile] {
        var profiles: [ConnectionProfile] = []
        if cloud {
            profiles.append(try preferences.save(.cowchatCloud(urlString: old, apiKey: "fixture-key")))
        }
        if social {
            let saved = SocialSession(endpoint: URL(string: old)!, deviceKey: "csoc_fixture", account: SocialAccount(id: "person", handle: "martin", displayName: "Martin"), scopeId: socialID)
            credentials.values[SocialSession.credentialAccount] = try JSONEncoder().encode(saved).base64EncodedString()
            profiles.append(try .socialSession(saved))
        }
        for profile in profiles {
            credentials.values[RoomCrypto.credentialAccount(profile: profile, roomID: "room")] = "shared-secret"
            defaults.set("unchanged-agent", forKey: "agent.\(profile.persistentIdentityScope!)")
        }
        credentials.writes = 0
        return profiles
    }

    func testPreservesBothIdentitiesAndCopiesEncryptedRoomKeys() async throws {
        let originals = try seed()
        let result1 = try await preferences.migrateCompanyEndpoint(session: session)
        XCTAssertTrue(result1)
        let cloud = try XCTUnwrap(preferences.loadConfiguredCloudProfile())
        let encoded = try XCTUnwrap(credentials.values[SocialSession.credentialAccount])
        let social = try JSONDecoder().decode(SocialSession.self, from: XCTUnwrap(Data(base64Encoded: encoded)))
        let moved = [cloud, try ConnectionProfile.socialSession(social)]
        for (before, after) in zip(originals, moved) {
            XCTAssertEqual(before.cloudAccountID, after.cloudAccountID)
            XCTAssertEqual(before.persistentIdentityScope, after.persistentIdentityScope)
            XCTAssertEqual(before.apiKey, after.apiKey)
            XCTAssertEqual(after.endpointDescription, new)
            let oldKey = credentials.values[RoomCrypto.credentialAccount(profile: before, roomID: "room")]
            let newKey = credentials.values[RoomCrypto.credentialAccount(profile: after, roomID: "room")]
            XCTAssertEqual(oldKey, newKey)
            let encrypted = try RoomCrypto.encrypt("history", secret: XCTUnwrap(oldKey), roomID: "room")
            XCTAssertEqual(try RoomCrypto.decrypt(encrypted, secret: XCTUnwrap(newKey), roomID: "room"), "history")
            XCTAssertEqual(defaults.string(forKey: "agent.\(after.persistentIdentityScope!)"), "unchanged-agent")
        }
        let result2 = try await preferences.migrateCompanyEndpoint(session: session)
        XCTAssertFalse(result2)
    }

    func testEveryInterruptedKeychainWriteCanResumeWithoutRotatingIdentity() async throws {
        for failAt in 1...6 {
            credentials.values = [:]
            defaults.removePersistentDomain(forName: suite)
            _ = try seed()
            credentials.failWrite = failAt
            do { _ = try await preferences.migrateCompanyEndpoint(session: session) } catch {}
            credentials.failWrite = nil
            _ = try await preferences.migrateCompanyEndpoint(session: session)
            XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.endpointDescription, new)
            XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.cloudAccountID, id)
            XCTAssertNil(credentials.values["company-endpoint-migration-v1"])
        }
    }

    func testStartupRecoversNewCredentialWithOldDisplayURLIncludingSocialOnly() async throws {
        for hasCloud in [true, false] {
            credentials.values = [:]
            defaults.removePersistentDomain(forName: suite)
            _ = try seed(cloud: hasCloud)
            // Keep the completed journal, then reproduce a crash before the defaults write.
            credentials.failWrite = hasCloud ? 6 : 4
            do { _ = try await preferences.migrateCompanyEndpoint(session: session) } catch {}
            XCTAssertNotNil(credentials.values["company-endpoint-migration-v1"])
            if hasCloud { defaults.set(old, forKey: "CowchatMac.connection.cloudURL") }
            else { defaults.removeObject(forKey: "CowchatMac.connection.cloudURL") }
            credentials.failWrite = nil
            let restarted = ConnectionProfilePreferences(defaults: defaults, credentialStore: credentials)
            let cloud = try? restarted.loadConfiguredCloudProfile()
            if hasCloud { XCTAssertEqual(cloud?.cloudAccountID, id) }
            else { XCTAssertNil(cloud) }
            XCTAssertEqual(restarted.loadSavedCloudURL(), new)
            XCTAssertNil(credentials.values["company-endpoint-migration-v1"])
            let encoded = try XCTUnwrap(credentials.values[SocialSession.credentialAccount])
            let social = try JSONDecoder().decode(SocialSession.self, from: XCTUnwrap(Data(base64Encoded: encoded)))
            XCTAssertEqual(social.endpoint.absoluteString, new)
            XCTAssertEqual(social.scopeId, socialID)
        }
    }

    func testOfflineUnauthorizedRedirectAndPublicLookingResponseKeepOldProfile() async throws {
        _ = try seed()
        let before = credentials.values
        for (status, body) in [(401, "{}"), (302, "{}"), (200, #"{"status":"ok"}"#)] {
            MigrationHTTP.handler = { _ in (status, body) }
            do { _ = try await preferences.migrateCompanyEndpoint(session: session); XCTFail("Accepted bad probe") } catch {}
            XCTAssertEqual(credentials.values, before)
            XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.endpointDescription, old)
        }
        MigrationHTTP.handler = { _ in throw URLError(.notConnectedToInternet) }
        do { _ = try await preferences.migrateCompanyEndpoint(session: session); XCTFail("Accepted offline probe") } catch {}
        XCTAssertEqual(credentials.values, before)
    }

    func testSocialOnlySessionUsesAuthenticatedSocialRouteAndSameScope() async throws {
        _ = try seed(cloud: false)
        let result3 = try await preferences.migrateCompanyEndpoint(session: session)
        XCTAssertTrue(result3)
        let encoded = try XCTUnwrap(credentials.values[SocialSession.credentialAccount])
        let saved = try JSONDecoder().decode(SocialSession.self, from: XCTUnwrap(Data(base64Encoded: encoded)))
        XCTAssertEqual(saved.scopeId, socialID)
        XCTAssertEqual(saved.endpoint.absoluteString, new)
        XCTAssertEqual(preferences.loadSavedCloudURL(), new)
    }

    func testWrongSocialAccountAndExistingDestinationRoomKeyFailClosed() async throws {
        let profiles = try seed()
        MigrationHTTP.handler = { request in
            if request.url?.path == "/api/agents" { return (200, #"{"agents":[]}"#) }
            return (200, #"{"account":{"id":"different","handle":"x","display_name":"X"},"identities":[]}"#)
        }
        do { _ = try await preferences.migrateCompanyEndpoint(session: session); XCTFail("Accepted another account") } catch {}
        XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.endpointDescription, old)
        credentials.values[SocialSession.credentialAccount] = nil
        MigrationHTTP.handler = { _ in (200, #"{"agents":[]}"#) }
        let scope = SocialSession(endpoint: URL(string: new)!, deviceKey: profiles[0].apiKey, account: SocialAccount(id: "unused", handle: "x", displayName: "X"), scopeId: id)
        let destination = try ConnectionProfile.socialSession(scope)
        let target = RoomCrypto.credentialAccount(profile: destination, roomID: "room")
        credentials.values[target] = "different-secret"
        do { _ = try await preferences.migrateCompanyEndpoint(session: session); XCTFail("Overwrote destination") } catch {}
        XCTAssertEqual(credentials.values[target], "different-secret")
        XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.endpointDescription, old)
    }

    func testArbitraryPreferenceCannotRedirectAndMissingPreferenceRecoversProtectedEndpoint() async throws {
        _ = try seed(social: false)
        for endpoint in ["wss://evil.example/ws", new] {
            defaults.set(endpoint, forKey: "CowchatMac.connection.cloudURL")
            let result4 = try await preferences.migrateCompanyEndpoint(session: session)
            XCTAssertFalse(result4)
            XCTAssertThrowsError(try preferences.loadConfiguredCloudProfile())
        }
        defaults.removeObject(forKey: "CowchatMac.connection.cloudURL")
        XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.endpointDescription, old)
        let migrated = try await preferences.migrateCompanyEndpoint(session: session)
        XCTAssertTrue(migrated)
        XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.endpointDescription, new)
    }

    func testForeignSocialSessionIsPreservedWhileSharedCompanyProfileMoves() async throws {
        _ = try seed(social: false)
        let foreign = SocialSession(endpoint: URL(string: "wss://other.example/ws")!, deviceKey: "csoc_foreign", account: SocialAccount(id: "foreign", handle: "foreign", displayName: "Foreign"), scopeId: socialID)
        let encoded = try JSONEncoder().encode(foreign).base64EncodedString()
        credentials.values[SocialSession.credentialAccount] = encoded
        let migrated = try await preferences.migrateCompanyEndpoint(session: session)
        XCTAssertTrue(migrated)
        XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.endpointDescription, new)
        XCTAssertEqual(credentials.values[SocialSession.credentialAccount], encoded)
    }

    func testRedirectDelegateNeverForwardsCredentialToAnotherHost() async throws {
        let request = URLRequest(url: URL(string: "https://other.example/api/agents")!)
        var redirected: URLRequest? = request
        SocialHTTPDelegate().urlSession(session, task: session.dataTask(with: request), willPerformHTTPRedirection: HTTPURLResponse(url: URL(string: "https://chat.cowchat.cowboylabs.net/api/agents")!, statusCode: 302, httpVersion: nil, headerFields: nil)!, newRequest: request) { redirected = $0 }
        XCTAssertNil(redirected)
    }

    func testSocialOnlyRollbackKeepsScopeAndCanExplicitlyMoveForwardAgain() async throws {
        _ = try seed(cloud: false)
        let handler = MigrationHTTP.handler
        _ = try await preferences.migrateCompanyEndpoint(session: session)
        MigrationHTTP.handler = { request in
            XCTAssertEqual(request.url?.host, "chat.cowchat.cowboy.inc")
            XCTAssertEqual(request.url?.path, "/social/v1/me")
            return (200, #"{"account":{"id":"person","handle":"martin","display_name":"Martin"},"identities":[]}"#)
        }
        let rolledBack = try await preferences.migrateCompanyEndpoint(rollback: true, session: session)
        XCTAssertTrue(rolledBack)
        XCTAssertEqual(preferences.loadSavedCloudURL(), old)
        MigrationHTTP.handler = handler
        let explicitRetry = try await preferences.migrateCompanyEndpoint(automatic: false, session: session)
        XCTAssertTrue(explicitRetry)
        let encoded = try XCTUnwrap(credentials.values[SocialSession.credentialAccount])
        let saved = try JSONDecoder().decode(SocialSession.self, from: XCTUnwrap(Data(base64Encoded: encoded)))
        XCTAssertEqual(saved.scopeId, socialID)
        XCTAssertEqual(saved.endpoint.absoluteString, new)
    }

    func testRollbackPreservesIdentityAndDoesNotImmediatelyMigrateAgain() async throws {
        _ = try seed()
        _ = try await preferences.migrateCompanyEndpoint(session: session)
        MigrationHTTP.handler = { request in
            XCTAssertEqual(request.url?.host, "chat.cowchat.cowboy.inc")
            return request.url?.path == "/api/agents"
                ? (200, #"{"agents":[]}"#)
                : (200, #"{"account":{"id":"person","handle":"martin","display_name":"Martin"},"identities":[]}"#)
        }
        let result6 = try await preferences.migrateCompanyEndpoint(rollback: true, session: session)
        XCTAssertTrue(result6)
        XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.cloudAccountID, id)
        XCTAssertEqual(try preferences.loadConfiguredCloudProfile()?.endpointDescription, old)
        let result7 = try await preferences.migrateCompanyEndpoint(session: session)
        XCTAssertFalse(result7)
    }
}
