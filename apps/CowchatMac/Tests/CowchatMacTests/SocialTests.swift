import AppKit
import SwiftUI
import XCTest
@testable import CowchatMac

private final class SocialMemoryCredentials: CowchatCredentialStore {
    var value: String?
    func credential(for account: String) throws -> String? { value }
    func setCredential(_ credential: String?, for account: String) throws { value = credential }
}
private final class SocialFixtureProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (Int, String))!
    static var contentType = "application/json"
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            guard let handler = Self.handler else { throw CancellationError() }
            let (status, body) = try handler(request)
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": Self.contentType])!, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data(body.utf8))
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

final class SocialTests: XCTestCase {
    func testFriendInvitationRoutesRejectSpoofedAndMalformedLinks() throws {
        let production = try XCTUnwrap(FriendInvitationRoute.parse(URL(string: "cowchat://friend/Chad-DePue")!))
        XCTAssertEqual(production.slug, "Chad-DePue")
        XCTAssertTrue(production.matches(URL(string: ConnectionProfile.defaultGlobalURLString)!))
        XCTAssertEqual(FriendInvitationRoute.parse(URL(string: "https://chat.cowchat.cowboy.inc/friends/Chad-DePue")!), production)
        for value in ["cowchat://friend/a", "cowchat://friend/Chad/extra", "cowchat://friend/Chad#fragment", "cowchat://friend/Chad?token=secret", "cowchat://friend/Chad?server=wss://one/ws&server=wss://two/ws", "cowchat://user@friend/Chad", "cowchat://friend:443/Chad", "cowchat://friend/Chad?server=http://evil.test/ws", "https://user@chat.cowchat.cowboy.inc/friends/Chad", "https://chat.cowchat.cowboy.inc/friends/Chad?foo=bar", "https://chat.cowchat.cowboy.inc/friends/Chad/", "http://chat.cowchat.cowboy.inc/friends/Chad", "cowchat://friend/Chad%2FDePue"] {
            XCTAssertNil(FriendInvitationRoute.parse(try XCTUnwrap(URL(string: value))), value)
        }
        let foreign = try XCTUnwrap(FriendInvitationRoute.parse(URL(string: "https://evil.test/friends/Chad")!))
        XCTAssertFalse(foreign.matches(production.endpoint))
        let custom = try XCTUnwrap(FriendInvitationRoute.parse(URL(string: "cowchat://friend/Chad?server=wss%3A%2F%2Fcustom.test%3A9443%2Fws")!))
        XCTAssertTrue(custom.matches(URL(string: "wss://custom.test:9443/ws")!))
        XCTAssertFalse(custom.matches(URL(string: "wss://custom.test/ws")!))
    }

    @MainActor
    func testWizardAvailabilityLinksAndExplicitFriendRequestAreSessionFenced() async throws {
        let fixture = try SetupFixture()
        defer { fixture.close() }
        let store = fixture.store()
        await store.loadSetup()
        XCTAssertTrue(store.handleAvailabilitySupported)
        let available = try await store.checkHandle("Chad-DePue")
        XCTAssertTrue(available.available)
        await store.loadFriendLink()
        let link = try XCTUnwrap(store.friendLink)
        XCTAssertEqual(link.url.absoluteString, "https://fixture.example/friends/chad")
        let invitation = try XCTUnwrap(FriendInvitationRoute.parse(URL(string: "https://fixture.example/friends/Friend")!))
        let preview = try await store.previewFriend(invitation)
        XCTAssertEqual(preview.account.id, "friend")
        let snapshot = FriendInvitationConfirmationSnapshot(invitation: invitation, scopeID: try XCTUnwrap(store.savedSession?.scopeId), preview: preview)
        XCTAssertTrue(snapshot.matches(invitation: invitation, scopeID: store.savedSession?.scopeId))
        let replacement = try XCTUnwrap(FriendInvitationRoute.parse(URL(string: "https://fixture.example/friends/Another")!))
        XCTAssertFalse(snapshot.matches(invitation: replacement, scopeID: store.savedSession?.scopeId))
        XCTAssertFalse(snapshot.matches(invitation: invitation, scopeID: "new-account-session"))
        let stale = FriendInvitationConfirmationSnapshot(invitation: invitation, scopeID: "old-account-session", preview: preview)
        let staleSent = await store.requestFriend(confirmation: stale)
        XCTAssertFalse(staleSent)
        XCTAssertEqual(fixture.friendPosts, 0)
        let foreign = try XCTUnwrap(FriendInvitationRoute.parse(URL(string: "https://foreign.test/friends/Friend")!))
        do { _ = try await store.previewFriend(foreign); XCTFail("Foreign endpoint was accepted") } catch {}
        let foreignSent = await store.requestFriend(invitation: foreign)
        XCTAssertFalse(foreignSent)
        XCTAssertEqual(fixture.friendPosts, 0)
        let sent = await store.requestFriend(confirmation: snapshot)
        XCTAssertTrue(sent)
        XCTAssertEqual(fixture.friendPosts, 1)
        let received = expectation(description: "Availability started")
        let release = DispatchSemaphore(value: 0)
        fixture.beforeAvailability = { received.fulfill(); _ = release.wait(timeout: .now() + 3) }
        let check = Task { try await store.checkHandle("Another") }
        await fulfillment(of: [received], timeout: 2)
        store.cancelSignIn()
        release.signal()
        do { _ = try await check.value; XCTFail("Stale session response accepted") } catch {}
        XCTAssertNil(store.friendLink)
        fixture.beforeAvailability = nil
        await store.loadFriendLink()
        try render(CowchatSetupWizard(social: store, initialStep: .friends, onLocal: {}, onOpenHome: { _, _ in false }, onOpenRooms: {}), name: "mac-wizard-share-link")
        try render(FriendInvitationConfirmation(social: store, invitation: invitation, onDismiss: {}), name: "mac-wizard-friend-confirmation")
    }

    @MainActor
    func testFriendLinkFailureSurvivesBackgroundRefreshAndCanRetry() async throws {
        let fixture = try SetupFixture()
        defer { fixture.close() }
        let store = fixture.store()
        await store.loadSetup()
        fixture.failLink = true
        await store.loadFriendLink()
        let failure = try XCTUnwrap(store.friendLinkFailure)
        XCTAssertNil(store.friendLink)
        await store.refresh()
        XCTAssertEqual(store.friendLinkFailure, failure)
        fixture.failLink = false
        await store.loadFriendLink()
        XCTAssertNotNil(store.friendLink)
        XCTAssertNil(store.friendLinkFailure)
    }

    @MainActor
    func testSetupRecoveryProfilePersistenceAndExplicitCredential() async throws {
        let fixture = try SetupFixture()
        defer { fixture.close() }
        let store = fixture.store()
        await store.loadSetup()
        XCTAssertTrue(store.setupLoaded)
        XCTAssertNil(store.setup)
        XCTAssertTrue(store.setupSupported)
        fixture.loseNextSetupReply = true
        let lost = await store.establishSetup(displayName: "Chad DePue")
        XCTAssertFalse(lost)
        XCTAssertNil(store.setup)
        await store.loadSetup()
        XCTAssertEqual(store.setup?.agentId, "starter-agent")
        XCTAssertEqual(store.setup?.roomId, "starter-home")
        XCTAssertEqual(store.account?.displayName, "Chad DePue")
        XCTAssertEqual(store.savedSession?.account, store.account)
        let restored = fixture.store()
        XCTAssertEqual(restored.account?.displayName, "Chad DePue")
        await restored.loadSetup()
        XCTAssertEqual(restored.setup?.roomId, store.setup?.roomId)
        XCTAssertEqual(fixture.setupPosts, 1)
        XCTAssertEqual(fixture.credentialPosts, 0)
        await store.connectAgent("starter-agent")
        XCTAssertEqual(store.issuedAgent?.apiKey, "csoc_agent_fixture")
        XCTAssertEqual(fixture.credentialPosts, 1)
        try render(CowchatSetupWizard(social: restored, initialStep: .name, name: "Chad DePue", onLocal: {}, onOpenHome: { _, _ in false }, onOpenRooms: {}), name: "mac-wizard-name")
        try render(CowchatSetupWizard(social: restored, initialStep: .friends, onLocal: {}, onOpenHome: { _, _ in false }, onOpenRooms: {}), name: "mac-wizard-friends")
        try render(CowchatSetupWizard(social: restored, initialStep: .ready, onLocal: {}, onOpenHome: { _, _ in false }, onOpenRooms: {}), name: "mac-wizard-ready")
    }

    @MainActor
    func testSetupLoadsDuringColdStartRefresh() async throws {
        let fixture = try SetupFixture()
        defer { fixture.close() }
        let store = fixture.store()
        let received = expectation(description: "Cold refresh capability request")
        let release = DispatchSemaphore(value: 0)
        fixture.beforeFirstCapabilitiesReply = { received.fulfill(); _ = release.wait(timeout: .now() + 3) }
        let refresh = Task { await store.refresh() }
        await fulfillment(of: [received], timeout: 2)
        let load = Task { await store.loadSetup() }
        try await Task.sleep(nanoseconds: 10_000_000)
        release.signal()
        await refresh.value
        await load.value
        XCTAssertTrue(store.setupSupported)
        XCTAssertTrue(store.setupLoaded)
        XCTAssertNil(store.setup)
    }

    @MainActor
    func testSetupUnsupportedHasNoProvisioningFallbackAndCancellationDiscardsReply() async throws {
        let fixture = try SetupFixture()
        defer { fixture.close() }
        fixture.supported = false
        let store = fixture.store()
        await store.loadSetup()
        XCTAssertFalse(store.setupSupported)
        let created = await store.establishSetup(displayName: "Chad")
        XCTAssertFalse(created)
        XCTAssertEqual(fixture.setupPosts, 0)
        fixture.supported = true
        await store.loadSetup()
        let received = expectation(description: "Setup request started")
        let release = DispatchSemaphore(value: 0)
        fixture.beforeSetupReply = { received.fulfill(); _ = release.wait(timeout: .now() + 3) }
        let pending = Task { await store.establishSetup(displayName: "Chad DePue") }
        await fulfillment(of: [received], timeout: 2)
        store.cancelSignIn()
        release.signal()
        let late = await pending.value
        XCTAssertFalse(late)
        XCTAssertNil(store.setup)
        XCTAssertFalse(store.setupLoaded)
        XCTAssertFalse(store.busy)
        XCTAssertNil(store.issuedAgent)
        fixture.beforeSetupReply = nil
        await store.loadSetup()
        let next = await store.establishSetup(displayName: "Chad DePue")
        XCTAssertTrue(next)
        fixture.supported = false
        let unsupported = fixture.store()
        await unsupported.loadSetup()
        try render(CowchatSetupWizard(social: unsupported, onLocal: {}, onOpenHome: { _, _ in false }, onOpenRooms: {}), name: "mac-wizard-unsupported")
        let signedOut = SocialStore(endpoint: fixture.endpoint, credentials: SocialMemoryCredentials(), session: fixture.session)
        await signedOut.refresh()
        try render(CowchatSetupWizard(social: signedOut, onLocal: {}, onOpenHome: { _, _ in false }, onOpenRooms: {}), name: "mac-wizard-welcome")
    }

    private final class SetupFixture {
        let endpoint = URL(string: "wss://fixture.example/ws")!
        let credentials = SocialMemoryCredentials()
        let session: URLSession
        var established = false
        var friendPosts = 0
        var failLink = false
        var beforeAvailability: (() -> Void)?
        var supported = true
        var setupPosts = 0
        var credentialPosts = 0
        var loseNextSetupReply = false
        var beforeSetupReply: (() -> Void)?
        var beforeFirstCapabilitiesReply: (() -> Void)?
        var capabilityReads = 0
        let agent = #"{"id":"starter-agent","owner_id":"me","name":"Chad_DePue_bot","discoverable":false}"#
        var setupJSON: String {
            established ? "{\"setup\":{\"agent_id\":\"starter-agent\",\"agent_name\":\"Chad_DePue_bot\",\"room_id\":\"starter-home\",\"room_name\":\"Chad_DePue_Home\",\"agent\":\(agent),\"room\":{\"room_id\":\"starter-home\",\"name\":\"Chad_DePue_Home\",\"owner_id\":\"me\"},\"bot_admitted\":true}}" : "{\"setup\":null}"
        }
        init() throws {
            let config = URLSessionConfiguration.ephemeral
            config.protocolClasses = [SocialFixtureProtocol.self]
            session = URLSession(configuration: config)
            let account = SocialAccount(id: "me", handle: "chad", displayName: "Chad")
            credentials.value = try JSONEncoder().encode(SocialSession(endpoint: endpoint, deviceKey: "csoc_setup_fixture", account: account)).base64EncodedString()
            SocialFixtureProtocol.handler = { request in
                switch request.url!.lastPathComponent {
                case "capabilities":
                    self.capabilityReads += 1
                    if self.capabilityReads == 1 { self.beforeFirstCapabilitiesReply?() }
                    return (200, self.supported ? #"{"providers":["google"],"setup_version":1,"handle_availability":true,"setup_handle":true,"friend_links":true}"# : #"{"providers":["google"]}"#)
                case "availability":
                    self.beforeAvailability?()
                    return (200, #"{"handle":"Chad-DePue","available":true,"valid":true}"#)
                case "friend-link":
                    if self.failLink { throw URLError(.networkConnectionLost) }
                    return (200, #"{"slug":"chad","url":"https://fixture.example/friends/chad"}"#)
                case "Friend": return (200, #"{"account":{"id":"friend","handle":"Friend","display_name":"Your Friend"},"relationship":"none"}"#)
                case "request":
                    self.friendPosts += 1
                    return (200, #"{"request":{"id":"request","from":{"id":"me","handle":"chad","display_name":"Chad"},"to":{"id":"friend","handle":"Friend","display_name":"Your Friend"},"status":"pending"}}"#)
                case "setup":
                    if request.httpMethod == "POST" {
                        XCTAssertEqual(try SocialTests.body(request)["display_name"] as? String, "Chad DePue")
                        self.setupPosts += 1
                        self.established = true
                        self.beforeSetupReply?()
                        if self.loseNextSetupReply { self.loseNextSetupReply = false; throw URLError(.networkConnectionLost) }
                    }
                    return (200, self.setupJSON)
                case "credentials":
                    XCTAssertEqual(request.url!.path, "/social/v1/agents/starter-agent/credentials")
                    self.credentialPosts += 1
                    return (200, "{\"agent\":\(self.agent),\"api_key\":\"csoc_agent_fixture\"}")
                case "me": return (200, "{\"account\":{\"id\":\"me\",\"handle\":\"chad\",\"display_name\":\"\(self.established ? "Chad DePue" : "Chad")\"},\"identities\":[]}")
                default:
                    let path = request.url!.lastPathComponent
                    let key = path == "friend-requests" ? "requests" : path == "room-invitations" ? "invitations" : path
                    return (200, "{\"\(key)\":[]}")
                }
            }
        }
        @MainActor func store() -> SocialStore { SocialStore(endpoint: endpoint, credentials: credentials, session: session) }
        func close() { session.invalidateAndCancel(); SocialFixtureProtocol.handler = nil }
    }

    @MainActor
    func testRoomCreationSupportsSoloAndFriendInvitesAndPreservesFailures() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SocialFixtureProtocol.self]
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel(); SocialFixtureProtocol.handler = nil }
        let credentials = SocialMemoryCredentials()
        let endpoint = URL(string: "wss://fixture.example/ws")!
        let account = SocialAccount(id: "me", handle: "chad", displayName: "Chad")
        credentials.value = try JSONEncoder().encode(SocialSession(endpoint: endpoint, deviceKey: "csoc_fixture", account: account)).base64EncodedString()
        var postCount = 0
        var roomList: [[String: String]] = []
        SocialFixtureProtocol.handler = { request in
            if request.httpMethod == "POST" {
                XCTAssertEqual(request.url?.path, "/social/v1/rooms")
                XCTAssertEqual(request.value(forHTTPHeaderField: "x-cowchat-key"), "csoc_fixture")
                let body = try Self.body(request)
                postCount += 1
                if postCount == 3 { return (500, #"{"error":"Room creation unavailable"}"#) }
                if postCount == 1 {
                    XCTAssertNil(body["friend_id"])
                    XCTAssertEqual(body["name"] as? String, "Solo room")
                    XCTAssertEqual(body["agent_ids"] as? [String], ["mine"])
                    XCTAssertEqual(body["proposed_agent_ids"] as? [String], [])
                } else {
                    XCTAssertEqual(body["friend_id"] as? String, "friend")
                    XCTAssertEqual(body["proposed_agent_ids"] as? [String], ["theirs"])
                }
                let id = "room-\(postCount)"
                roomList.append(["room_id": id, "name": body["name"] as! String, "owner_id": "me"])
                let invitation = postCount == 1 ? "null" : "{\"id\":\"invite\"}"
                return (200, "{\"room_id\":\"\(id)\",\"invitation\":\(invitation)}")
            }
            switch request.url!.lastPathComponent {
            case "capabilities": return (200, #"{"providers":["google"]}"#)
            case "me": return (200, #"{"account":{"id":"me","handle":"chad","display_name":"Chad"},"identities":[]}"#)
            case "friends": return (200, #"{"friends":[{"account":{"id":"friend","handle":"renzo","display_name":"Renzo"},"agents":[]}] }"#)
            case "friend-requests": return (200, #"{"requests":[]}"#)
            case "agents": return (200, #"{"agents":[]}"#)
            case "room-invitations": return (200, #"{"invitations":[]}"#)
            case "rooms": return (200, String(data: try JSONSerialization.data(withJSONObject: ["rooms": roomList]), encoding: .utf8)!)
            default: throw CowchatConnectionError.invalidResponse
            }
        }
        let store = SocialStore(endpoint: endpoint, credentials: credentials, session: session)
        await store.refresh()
        var changes = 0
        store.onRoomsChanged = { changes += 1 }
        var draft = SocialRoomDraft(name: "  Solo room  ", agentIDs: ["mine"])
        let soloCreated = await store.createRoom(draft)
        XCTAssertEqual(soloCreated, "room-1")
        XCTAssertEqual(store.rooms.first?.id, "room-1")
        draft.name = "Shared room"
        draft.friendID = "friend"
        draft.proposedAgentIDs = ["theirs"]
        let sharedCreated = await store.createRoom(draft)
        XCTAssertEqual(sharedCreated, "room-2")
        XCTAssertEqual(store.rooms.count, 2)
        let failed = await store.createRoom(draft)
        XCTAssertNil(failed)
        XCTAssertEqual(changes, 2)
        XCTAssertEqual(store.rooms.count, 2)
        XCTAssertTrue(store.errorMessage?.contains("Room creation unavailable") == true)
        try render(FriendsView(social: store, initialSection: .rooms), name: "mac-rooms-populated-create-action")
        roomList = []
        let emptyStore = SocialStore(endpoint: endpoint, credentials: credentials, session: session)
        await emptyStore.refresh()
        try render(FriendsView(social: emptyStore, initialSection: .rooms), name: "mac-rooms-empty-create-action")
        try render(SocialRoomCreationView(social: emptyStore), name: "mac-room-create-solo")
        try render(SocialRoomCreationView(social: emptyStore, draft: draft), name: "mac-room-create-friend")
    }

    @MainActor
    func testNativeNotificationSettingsRendering() throws {
        let suite = "CowchatNotificationSettings.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let notifications = SocialNotifications(defaults: defaults)
        try render(VStack(alignment: .leading, spacing: 24) {
            Text("Notifications").gallopText(.h3)
            SocialNotificationSettings(notifications: notifications)
            Spacer()
        }.padding(28).background(SemanticColor.surface600), name: "mac-notification-settings")
    }

    @MainActor
    func testAppMonitoringPublishesAcceptedRequestsWithoutOpeningFriendsAndStops() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SocialFixtureProtocol.self]
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel(); SocialFixtureProtocol.handler = nil }
        let credentials = SocialMemoryCredentials()
        let endpoint = URL(string: "wss://fixture.example/ws")!
        let account = SocialAccount(id: "me", handle: "chad", displayName: "Chad")
        credentials.value = try JSONEncoder().encode(SocialSession(endpoint: endpoint, deviceKey: "csoc_fixture", account: account)).base64EncodedString()
        SocialFixtureProtocol.handler = { request in
            let body: String
            switch request.url!.lastPathComponent {
            case "capabilities": body = #"{"providers":["google"]}"#
            case "me": body = #"{"account":{"id":"me","handle":"chad","display_name":"Chad"},"identities":[]}"#
            case "friends": body = #"{"friends":[]}"#
            case "friend-requests": body = #"{"requests":[{"id":"accepted","from":{"id":"me","handle":"chad","display_name":"Chad"},"to":{"id":"peer","handle":"renzo","display_name":"Renzo"},"status":"accepted"}]}"#
            case "agents": body = #"{"agents":[]}"#
            case "room-invitations": body = #"{"invitations":[]}"#
            case "rooms": body = #"{"rooms":[]}"#
            default: throw CowchatConnectionError.invalidResponse
            }
            return (200, body)
        }
        let store = SocialStore(endpoint: endpoint, credentials: credentials, session: session)
        let polled = expectation(description: "App lifetime polling emits complete snapshots")
        var snapshots = 0
        store.onSnapshot = { snapshot in
            snapshots += 1
            XCTAssertEqual(snapshot.requests.first?.status, "accepted")
            XCTAssertEqual(snapshot.session.account.id, "me")
            if snapshots == 2 { polled.fulfill() }
        }
        store.startMonitoring(interval: .milliseconds(10))
        store.startMonitoring(interval: .milliseconds(10))
        await fulfillment(of: [polled], timeout: 2)
        store.stopMonitoring()
        let stoppedAt = snapshots
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(snapshots, stoppedAt)
        XCTAssertTrue(store.requests.isEmpty)
        XCTAssertNil(store.errorMessage)
        let resumed = expectation(description: "Polling can restart after cancellation")
        store.onSnapshot = { _ in resumed.fulfill(); store.stopMonitoring() }
        store.startMonitoring(interval: .milliseconds(10))
        await fulfillment(of: [resumed], timeout: 2)
    }

    @MainActor
    func testOlderServerWebsiteFallbackShowsUpdateRequiredAndRecovers() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SocialFixtureProtocol.self]
        let session = URLSession(configuration: config)
        defer {
            session.invalidateAndCancel()
            SocialFixtureProtocol.handler = nil
            SocialFixtureProtocol.contentType = "application/json"
        }
        let store = SocialStore(endpoint: URL(string: "wss://fixture.example/ws")!, credentials: SocialMemoryCredentials(), session: session)
        SocialFixtureProtocol.contentType = "text/html; charset=utf-8"
        for status in [200, 404] {
            SocialFixtureProtocol.handler = { request in
                XCTAssertEqual(request.url?.path, "/social/v1/capabilities")
                return (status, "<!doctype html><html><title>Cowchat | AI Agent Coordination</title></html>")
            }
            await store.refresh()
            XCTAssertTrue(store.serverNeedsUpdate)
            XCTAssertTrue(store.providers.isEmpty)
            XCTAssertNil(store.errorMessage)
        }
        SocialFixtureProtocol.contentType = "application/json"
        SocialFixtureProtocol.handler = { _ in (200, #"{"providers":[]}"#) }
        await store.refresh()
        XCTAssertFalse(store.serverNeedsUpdate)
        XCTAssertNil(store.errorMessage)
        XCTAssertTrue(store.providers.isEmpty)

        SocialFixtureProtocol.handler = { _ in (200, #"{"providers":["google"]}"#) }
        await store.refresh()
        XCTAssertFalse(store.serverNeedsUpdate)
        XCTAssertEqual(store.providers, ["google"])

        SocialFixtureProtocol.handler = { _ in (500, #"{"error":"Temporarily unavailable"}"#) }
        await store.refresh()
        XCTAssertFalse(store.serverNeedsUpdate)
        XCTAssertTrue(store.errorMessage?.contains("Temporarily unavailable") == true)

        SocialFixtureProtocol.contentType = "text/html"
        SocialFixtureProtocol.handler = { _ in (200, "<!doctype html><html>Cowchat</html>") }
        await store.refresh()
        try render(FriendsView(social: store), name: "mac-friends-server-update")
    }

    func testCopiedHandleCanBePastedIntoFriendRequest() {
        XCTAssertEqual(SocialAPI.normalizedHandle(" @renzo "), "renzo")
        XCTAssertEqual(SocialAPI.normalizedHandle("renzo"), "renzo")
    }

    func testPKCEUsesRFC7636Challenge() {
        XCTAssertEqual(SocialAPI.challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        let verifier = try! SocialAPI.verifier()
        XCTAssertEqual(verifier.count, 43)
        XCTAssertNotEqual(verifier, try! SocialAPI.verifier())
    }

    func testRequestsBindDeviceKeyToHTTPSOriginAndExcludeItFromOAuthExchange() throws {
        let api = try SocialAPI(endpoint: URL(string: "wss://chat.example:8443/ws")!, key: "csoc_secret")
        let request = try api.makeRequest(["social", "v1", "rooms", "room name", "participants"])
        XCTAssertEqual(request.url?.absoluteString, "https://chat.example:8443/social/v1/rooms/room%20name/participants")
        XCTAssertEqual(request.value(forHTTPHeaderField: "x-cowchat-key"), "csoc_secret")
        XCTAssertFalse(request.url!.absoluteString.contains("secret"))
        let exchange = try api.makeRequest(["auth", "social", "exchange"], auth: false, method: "POST", body: ["request_id": "request", "code_verifier": "verifier"])
        XCTAssertNil(exchange.value(forHTTPHeaderField: "x-cowchat-key"))
        XCTAssertThrowsError(try SocialAPI(endpoint: URL(string: "ws://chat.example/ws")!))
        XCTAssertThrowsError(try SocialAPI(endpoint: URL(string: "wss://user:secret@chat.example/ws")!))
    }

    func testRedirectDelegateRefusesEvenSameOriginRedirect() {
        let delegate = SocialHTTPDelegate()
        let url = URL(string: "https://chat.example/social/v1/me")!
        let session = URLSession(configuration: .ephemeral)
        defer { session.invalidateAndCancel() }
        var called = false
        delegate.urlSession(session, task: session.dataTask(with: url), willPerformHTTPRedirection: HTTPURLResponse(url: url, statusCode: 302, httpVersion: nil, headerFields: [:])!, newRequest: URLRequest(url: URL(string: "https://attacker.example/")!)) { request in
            called = true
            XCTAssertNil(request)
        }
        XCTAssertTrue(called)
    }

    @MainActor
    func testSavedSessionRejectsDifferentEndpointAndFencesChatNamespace() throws {
        let credentials = SocialMemoryCredentials()
        let account = SocialAccount(id: UUID().uuidString.lowercased(), handle: "chad", displayName: "Chad")
        let saved = SocialSession(endpoint: URL(string: "wss://protected.example/ws")!, deviceKey: "csoc_device", account: account)
        credentials.value = try JSONEncoder().encode(saved).base64EncodedString()
        let store = SocialStore(endpoint: URL(string: "wss://changed.example/ws")!, credentials: credentials)
        XCTAssertEqual(store.endpoint.host, "changed.example")
        XCTAssertEqual(store.phase, .signedOut)
        XCTAssertNil(store.savedSession)
        let profile = try ConnectionProfile.socialSession(saved)
        XCTAssertEqual(profile.persistentIdentityScope, "cowchat-cloud.\(saved.scopeId)")
        XCTAssertEqual(profile.apiKey, saved.deviceKey)
    }

    @MainActor
    func testRefreshAndNativeRenderingWithRealJSONFixtures() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SocialFixtureProtocol.self]
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel(); SocialFixtureProtocol.handler = nil }
        let credentials = SocialMemoryCredentials()
        let id = "11111111-1111-4111-8111-111111111111"
        let account = SocialAccount(id: id, handle: "chad", displayName: "Chad")
        let endpoint = URL(string: "wss://fixture.example/ws")!
        credentials.value = try JSONEncoder().encode(SocialSession(endpoint: endpoint, deviceKey: "csoc_fixture", account: account)).base64EncodedString()
        SocialFixtureProtocol.handler = { request in
            let value: String
            switch request.url!.path {
            case "/social/v1/capabilities": value = #"{"providers":["github","google"],"cowboy_login":false}"#
            case "/social/v1/me": value = #"{"account":{"id":"11111111-1111-4111-8111-111111111111","handle":"chad","display_name":"Chad"},"identities":[]}"#
            case "/social/v1/friends": value = #"{"friends":[{"account":{"id":"friend","handle":"renzo","display_name":"Renzo"},"agents":[{"id":"agent","owner_id":"friend","name":"Coordinator","discoverable":true}]}]}"#
            case "/social/v1/friend-requests": value = #"{"requests":[{"id":"request","from":{"id":"friend","handle":"rachel","display_name":"Rachel"},"to":{"id":"11111111-1111-4111-8111-111111111111","handle":"chad","display_name":"Chad"},"status":"pending","created_at":"now"}]}"#
            case "/social/v1/agents": value = #"{"agents":[]}"#
            case "/social/v1/room-invitations": value = #"{"invitations":[]}"#
            case "/social/v1/rooms": value = #"{"rooms":[]}"#
            case "/social/v1/logout": value = #"{"ok":true}"#
            default: throw CowchatConnectionError.invalidResponse
            }
            return (200, value)
        }
        let store = SocialStore(endpoint: endpoint, credentials: credentials, session: session)
        await store.refresh()
        XCTAssertNil(store.errorMessage)
        XCTAssertEqual(store.friends.first?.account.handle, "renzo")
        XCTAssertEqual(store.requests.first?.from.handle, "rachel")
        try render(FriendsView(social: store), name: "mac-friends-signed-in")
        var loggedOut = false
        store.onSessionChanged = { value in loggedOut = value == nil }
        await store.signOut()
        XCTAssertTrue(loggedOut)
        XCTAssertNil(credentials.value)
        XCTAssertTrue(store.friends.isEmpty)
        XCTAssertEqual(store.phase, .signedOut)
        try render(FriendsView(social: store), name: "mac-friends-sign-in")
    }

    @MainActor
    func testSignOutFencesInFlightOperationAndResetsBusy() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SocialFixtureProtocol.self]
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel(); SocialFixtureProtocol.handler = nil }
        let credentials = SocialMemoryCredentials()
        let endpoint = URL(string: "wss://fixture.example/ws")!
        let account = SocialAccount(id: UUID().uuidString, handle: "chad", displayName: "Chad")
        credentials.value = try JSONEncoder().encode(SocialSession(endpoint: endpoint, deviceKey: "csoc_fixture", account: account)).base64EncodedString()
        SocialFixtureProtocol.handler = { request in
            XCTAssertEqual(request.url?.path, "/social/v1/logout")
            return (200, "{\"ok\":true}")
        }
        let store = SocialStore(endpoint: endpoint, credentials: credentials, session: session)
        let work = Task { await store.perform { _ in try await Task.sleep(nanoseconds: 50_000_000) } }
        await Task.yield()
        XCTAssertTrue(store.busy)
        await store.signOut()
        await work.value
        XCTAssertFalse(store.busy)
        XCTAssertEqual(store.phase, .signedOut)
        XCTAssertNil(store.account)
    }

    @MainActor
    func testOAuthRefusesProviderImpersonationAndNeverStoresCredential() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SocialFixtureProtocol.self]
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel(); SocialFixtureProtocol.handler = nil }
        SocialFixtureProtocol.handler = { request in
            if request.url?.path == "/social/v1/capabilities" { return (200, "{\"providers\":[\"github\"]}") }
            XCTAssertEqual(request.url?.path, "/auth/social/start")
            XCTAssertNil(request.value(forHTTPHeaderField: "x-cowchat-key"))
            return (200, "{\"request_id\":\"request\",\"authorization_url\":\"https://attacker.example/login\",\"expires_in\":600}")
        }
        let credentials = SocialMemoryCredentials()
        let store = SocialStore(endpoint: URL(string: "wss://fixture.example/ws")!, credentials: credentials, session: session)
        await store.refresh()
        var opened = false
        store.signIn(provider: "github", open: { _ in opened = true; return true })
        for _ in 0..<100 where store.phase == .awaitingBrowser { try await Task.sleep(nanoseconds: 1_000_000) }
        XCTAssertFalse(opened)
        XCTAssertEqual(store.phase, .signedOut)
        XCTAssertNotNil(store.errorMessage)
        XCTAssertNil(credentials.value)
    }

    @MainActor
    func testNativeLoopbackCallbackRequiresMatchingRequestAndHost() async throws {
        let callback = SocialOAuthCallback()
        let port = try await callback.start()
        defer { callback.cancel() }
        callback.expect(requestID: "request-one")
        let code = String(repeating: "a", count: 64)
        let badURL = URL(string: "http://127.0.0.1:\(port)/cowchat/oauth?request_id=wrong&completion_code=\(code)")!
        let (_, badResponse) = try await URLSession.shared.data(from: badURL)
        XCTAssertEqual((badResponse as? HTTPURLResponse)?.statusCode, 400)
        let pending = Task { try await callback.wait(requestID: "request-one", expiresIn: 30) }
        let url = URL(string: "http://127.0.0.1:\(port)/cowchat/oauth?request_id=request-one&completion_code=\(code)")!
        let (data, response) = try await URLSession.shared.data(from: url)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        XCTAssertFalse(String(decoding: data, as: UTF8.self).contains(code))
        let received = try await pending.value
        XCTAssertEqual(received, code)
        let wrongHost = "GET /cowchat/oauth?request_id=request-one&completion_code=\(code) HTTP/1.1\r\nHost: attacker.example\r\n\r\n"
        XCTAssertNil(SocialOAuthCallback.completionCode(from: wrongHost, port: port, requestID: "request-one"))
        let duplicate = "GET /cowchat/oauth?request_id=request-one&completion_code=\(code)&completion_code=\(code) HTTP/1.1\r\nHost: 127.0.0.1:\(port)\r\n\r\n"
        XCTAssertNil(SocialOAuthCallback.completionCode(from: duplicate, port: port, requestID: "request-one"))
    }

    @MainActor
    func testNativeCallbackCancellationReturnsToCaller() async throws {
        let callback = SocialOAuthCallback()
        _ = try await callback.start()
        let pending = Task { try await callback.wait(requestID: "request", expiresIn: 30) }
        await Task.yield()
        callback.cancel()
        do { _ = try await pending.value; XCTFail("Cancelled callback unexpectedly completed") }
        catch { XCTAssertTrue(error is CancellationError) }
    }

    @MainActor
    func testOAuthCompletesOnlyAfterNativeCallbackAndExchangesPKCEProof() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SocialFixtureProtocol.self]
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel(); SocialFixtureProtocol.handler = nil }
        var callbackPort = 0
        var challenge = ""
        var exchanges = 0
        let code = String(repeating: "b", count: 64)
        let accountJSON = "{\"id\":\"11111111-1111-4111-8111-111111111111\",\"handle\":\"chad\",\"display_name\":\"Chad\"}"
        SocialFixtureProtocol.handler = { request in
            switch request.url!.path {
            case "/social/v1/capabilities": return (200, "{\"providers\":[\"github\"]}")
            case "/auth/social/start":
                let body = try Self.body(request)
                callbackPort = body["callback_port"] as! Int
                challenge = body["code_challenge"] as! String
                XCTAssertTrue((1024...65535).contains(callbackPort))
                XCTAssertNil(request.value(forHTTPHeaderField: "x-cowchat-key"))
                return (200, "{\"request_id\":\"request\",\"authorization_url\":\"https://github.com/login/oauth/authorize\",\"expires_in\":600}")
            case "/auth/social/exchange":
                let body = try Self.body(request)
                XCTAssertEqual(body["completion_code"] as? String, code)
                XCTAssertEqual(body["request_id"] as? String, "request")
                XCTAssertEqual(SocialAPI.challenge(body["code_verifier"] as! String), challenge)
                exchanges += 1
                return (200, "{\"status\":\"complete\",\"device_key\":\"csoc_created\",\"account\":\(accountJSON)}")
            case "/social/v1/me": return (200, "{\"account\":\(accountJSON),\"identities\":[]}")
            default:
                let key = request.url!.lastPathComponent == "friend-requests" ? "requests" : request.url!.lastPathComponent == "room-invitations" ? "invitations" : request.url!.lastPathComponent
                return (200, "{\"\(key)\":[]}")
            }
        }
        let credentials = SocialMemoryCredentials()
        let store = SocialStore(endpoint: URL(string: "wss://fixture.example/ws")!, credentials: credentials, session: session)
        await store.refresh()
        var activated: SocialSession?
        store.onSessionChanged = { activated = $0 }
        let login = store.signIn(provider: "github", open: { url in
            XCTAssertEqual(url.host, "github.com")
            XCTAssertEqual(exchanges, 0)
            Task {
                let callback = URL(string: "http://127.0.0.1:\(callbackPort)/cowchat/oauth?request_id=request&completion_code=\(code)")!
                _ = try await URLSession.shared.data(from: callback)
            }
            return true
        })
        for _ in 0..<300 where store.phase != .signedIn { try await Task.sleep(nanoseconds: 1_000_000) }
        await login?.value
        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertEqual(exchanges, 1)
        XCTAssertEqual(activated?.deviceKey, "csoc_created")
        XCTAssertEqual(activated?.account.handle, "chad")
        XCTAssertNotNil(credentials.value)
        store.cancelSignIn()
    }

    @MainActor
    func testLinkProviderAuthenticatesPinnedOriginAndRotatesSameAccountSession() async throws {
        let fixture = try LinkFixture()
        defer { fixture.close() }
        let store = fixture.store()
        await store.refresh()
        XCTAssertEqual(store.identities.map(\.provider), ["github"])
        XCTAssertEqual(store.availableLinkProviders, ["google"])
        try render(SocialProfileEditor(handle: .constant("chad"), displayName: .constant("Chad"), busy: false, error: nil, identities: store.identities, linkProviders: store.availableLinkProviders, onCancel: {}, onSave: {}), name: "mac-friends-link-provider")
        XCTAssertNil(store.linkProvider("github"))
        var activated: SocialSession?
        store.onSessionChanged = { activated = $0 }
        let linking = store.linkProvider("google", open: { url in
            XCTAssertEqual(url.host, "accounts.google.com")
            XCTAssertEqual(store.savedSession, fixture.original)
            XCTAssertEqual(store.phase, .signedIn)
            Task { await fixture.complete() }
            return true
        })
        await linking?.value
        XCTAssertEqual(fixture.starts, 1)
        XCTAssertEqual(fixture.exchanges, 1)
        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertNil(store.errorMessage)
        XCTAssertNil(store.linkingProvider)
        XCTAssertEqual(activated?.account.id, fixture.original.account.id)
        XCTAssertEqual(activated?.deviceKey, "csoc_linked")
        XCTAssertNotEqual(activated?.scopeId, fixture.original.scopeId)
        XCTAssertEqual(try fixture.savedCredential()?.deviceKey, "csoc_linked")
        XCTAssertEqual(store.identities.map(\.provider), ["github", "google"])
        XCTAssertTrue(store.availableLinkProviders.isEmpty)
        try render(SocialProfileEditor(handle: .constant("chad"), displayName: .constant("Chad"), busy: false, error: nil, identities: store.identities, linkProviders: store.availableLinkProviders, onCancel: {}, onSave: {}), name: "mac-friends-linked-providers")
    }

    @MainActor
    func testLinkRejectsDifferentAccountAndKeepsOriginalCredential() async throws {
        let fixture = try LinkFixture()
        defer { fixture.close() }
        fixture.exchangeAccountID = "22222222-2222-4222-8222-222222222222"
        let store = fixture.store()
        await store.refresh()
        var sessionChanges = 0
        store.onSessionChanged = { _ in sessionChanges += 1 }
        let linking = store.linkProvider("google", open: { _ in
            Task { await fixture.complete() }
            return true
        })
        await linking?.value
        XCTAssertEqual(fixture.exchanges, 1)
        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertNotNil(store.errorMessage)
        XCTAssertEqual(store.savedSession, fixture.original)
        XCTAssertEqual(try fixture.savedCredential(), fixture.original)
        XCTAssertEqual(store.identities.map(\.provider), ["github"])
        XCTAssertEqual(sessionChanges, 0)
    }

    @MainActor
    func testLinkCancellationAndServerFailureKeepOriginalCredential() async throws {
        let fixture = try LinkFixture()
        defer { fixture.close() }
        let store = fixture.store()
        await store.refresh()
        var opened = false
        let pending = store.linkProvider("google", open: { _ in opened = true; return true })
        for _ in 0..<300 where !opened { try await Task.sleep(nanoseconds: 1_000_000) }
        XCTAssertTrue(opened)
        XCTAssertEqual(store.linkingProvider, "google")
        store.cancelSignIn()
        await pending?.value
        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertNil(store.linkingProvider)
        XCTAssertEqual(store.savedSession, fixture.original)
        XCTAssertEqual(try fixture.savedCredential(), fixture.original)
        XCTAssertEqual(fixture.exchanges, 0)

        fixture.exchangeStatus = 409
        let failed = store.linkProvider("google", open: { _ in
            Task { await fixture.complete() }
            return true
        })
        await failed?.value
        XCTAssertNotNil(store.errorMessage)
        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertNil(store.linkingProvider)
        XCTAssertEqual(store.savedSession, fixture.original)
        XCTAssertEqual(try fixture.savedCredential(), fixture.original)
    }

    private final class LinkFixture {
        let endpoint = URL(string: "wss://fixture.example:8443/ws")!
        let credentials = SocialMemoryCredentials()
        let session: URLSession
        let original: SocialSession
        var callbackPort = 0
        var starts = 0
        var exchanges = 0
        var exchangeStatus = 200
        var exchangeAccountID = "11111111-1111-4111-8111-111111111111"
        private let completionCode = String(repeating: "c", count: 64)
        private var challenge = ""

        init() throws {
            let config = URLSessionConfiguration.ephemeral
            config.protocolClasses = [SocialFixtureProtocol.self]
            session = URLSession(configuration: config)
            original = SocialSession(endpoint: endpoint, deviceKey: "csoc_original", account: SocialAccount(id: exchangeAccountID, handle: "chad", displayName: "Chad"))
            credentials.value = try JSONEncoder().encode(original).base64EncodedString()
            SocialFixtureProtocol.handler = { request in
                XCTAssertEqual(request.url?.scheme, "https")
                XCTAssertEqual(request.url?.host, "fixture.example")
                XCTAssertEqual(request.url?.port, 8443)
                switch request.url!.path {
                case "/social/v1/capabilities": return (200, #"{"providers":["github","google"]}"#)
                case "/auth/social/start":
                    XCTAssertEqual(request.value(forHTTPHeaderField: "x-cowchat-key"), "csoc_original")
                    let body = try SocialTests.body(request)
                    XCTAssertEqual(body["provider"] as? String, "google")
                    self.callbackPort = body["callback_port"] as! Int
                    self.challenge = body["code_challenge"] as! String
                    self.starts += 1
                    return (200, #"{"request_id":"link-request","authorization_url":"https://accounts.google.com/o/oauth2/v2/auth","expires_in":600}"#)
                case "/auth/social/exchange":
                    XCTAssertNil(request.value(forHTTPHeaderField: "x-cowchat-key"))
                    let body = try SocialTests.body(request)
                    XCTAssertEqual(body["completion_code"] as? String, self.completionCode)
                    XCTAssertEqual(body["request_id"] as? String, "link-request")
                    XCTAssertEqual(SocialAPI.challenge(body["code_verifier"] as! String), self.challenge)
                    self.exchanges += 1
                    if self.exchangeStatus != 200 { return (self.exchangeStatus, #"{"error":"Identity already linked to another account."}"#) }
                    return (200, "{\"status\":\"complete\",\"device_key\":\"csoc_linked\",\"account\":\(self.accountJSON(self.exchangeAccountID))}")
                case "/social/v1/me":
                    XCTAssertEqual(request.value(forHTTPHeaderField: "x-cowchat-key"), self.exchanges > 0 ? "csoc_linked" : "csoc_original")
                    let google = self.exchanges > 0 ? #",{"provider":"google","subject":"google-stable-id","label":"Chad Google"}"# : ""
                    return (200, "{\"account\":\(self.accountJSON(self.original.account.id)),\"identities\":[{\"provider\":\"github\",\"subject\":\"github-stable-id\",\"label\":\"chadd\"}\(google)]}")
                default:
                    let path = request.url!.lastPathComponent
                    let key = path == "friend-requests" ? "requests" : path == "room-invitations" ? "invitations" : path
                    return (200, "{\"\(key)\":[]}")
                }
            }
        }

        @MainActor
        func store() -> SocialStore { SocialStore(endpoint: endpoint, credentials: credentials, session: session) }

        func savedCredential() throws -> SocialSession? {
            guard let value = credentials.value, let data = Data(base64Encoded: value) else { return nil }
            return try JSONDecoder().decode(SocialSession.self, from: data)
        }

        func complete() async {
            do {
                let url = URL(string: "http://127.0.0.1:\(callbackPort)/cowchat/oauth?request_id=link-request&completion_code=\(completionCode)")!
                let (_, response) = try await URLSession.shared.data(from: url)
                XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
            } catch { XCTFail("Link callback failed: \(error)") }
        }

        func close() { session.invalidateAndCancel(); SocialFixtureProtocol.handler = nil }

        private func accountJSON(_ id: String) -> String {
            "{\"id\":\"\(id)\",\"handle\":\"chad\",\"display_name\":\"Chad\"}"
        }
    }

    private static func body(_ request: URLRequest) throws -> [String: Any] {
        var data = request.httpBody ?? Data()
        if data.isEmpty, let stream = request.httpBodyStream {
            stream.open()
            defer { stream.close() }
            var buffer = [UInt8](repeating: 0, count: 4096)
            while stream.hasBytesAvailable {
                let count = stream.read(&buffer, maxLength: buffer.count)
                guard count > 0 else { break }
                data.append(contentsOf: buffer.prefix(count))
            }
        }
        return try JSONSerialization.jsonObject(with: data) as! [String: Any]
    }

    @MainActor
    func testNativeInvitationAndParticipantConsentRendering() throws {
        let account = SocialAccount(id: "owner", handle: "renzo", displayName: "Renzo")
        let recipient = SocialAccount(id: "recipient", handle: "chad", displayName: "Chad")
        let coordinator = SocialAgent(id: "coordinator", ownerId: account.id, name: "Renzo's coordinator", discoverable: true)
        let own = SocialAgent(id: "reviewer", ownerId: recipient.id, name: "Chad's reviewer", discoverable: true)
        try render(SocialProfileEditor(handle: .constant("chad"), displayName: .constant("Chad"), busy: false, error: nil, onCancel: {}, onSave: {}), name: "mac-friends-edit-profile")
        let invitation = SocialRoomInvitation(id: "invitation", roomId: "room", roomName: "Release planning", from: account, to: recipient, proposedAgents: [own], members: [account], agents: [coordinator], historyPolicy: "full", encrypted: false, slackConnected: false, status: "pending")
        try render(SocialInvitationReview(invitation: invitation, agents: [own], selectedAgents: .constant([]), busy: false, error: nil, onCancel: {}, onAccept: {}), name: "mac-friends-invitation")
        try render(VStack(alignment: .leading, spacing: 16) {
            Text("Release planning").gallopText(.h3)
            SocialConsentDetails(members: [account, recipient], agents: [coordinator, own], slack: false)
        }.padding(24).frame(width: 570).background(SemanticColor.surface500), name: "mac-friends-participants")
    }

    @MainActor
    func testServerCallbackContractFixtureAccepted() throws {
        guard let path = ProcessInfo.processInfo.environment["COWCHAT_SOCIAL_CALLBACK_FIXTURE"] else {
            throw XCTSkip("Run scripts/test-social-contract.sh to check the real server callback contract.")
        }
        struct Fixture: Decodable {
            let requestId: String
            let callbackUrl: URL
            let callbackPort: UInt16
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let fixture = try decoder.decode(Fixture.self, from: Data(contentsOf: URL(fileURLWithPath: path)))
        XCTAssertEqual(fixture.callbackUrl.scheme, "http")
        XCTAssertEqual(fixture.callbackUrl.host, "127.0.0.1")
        XCTAssertEqual(fixture.callbackUrl.port, Int(fixture.callbackPort))
        let components = try XCTUnwrap(URLComponents(url: fixture.callbackUrl, resolvingAgainstBaseURL: false))
        let target = components.percentEncodedPath + (components.percentEncodedQuery.map { "?" + $0 } ?? "")
        let header = "GET \(target) HTTP/1.1\r\nHost: 127.0.0.1:\(fixture.callbackPort)\r\n\r\n"
        let code = try XCTUnwrap(SocialOAuthCallback.completionCode(from: header, port: fixture.callbackPort, requestID: fixture.requestId))
        XCTAssertFalse(code.isEmpty)
        XCTAssertNil(SocialOAuthCallback.completionCode(from: header, port: fixture.callbackPort, requestID: "wrong-request"))
    }

    @MainActor
    private func render<V: View>(_ content: V, name: String) throws {
        _ = NSApplication.shared
        let directory = URL(fileURLWithPath: "/tmp/cowchat-friends-work")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for scheme in [ColorScheme.light, .dark] {
            let view = NSHostingView(rootView: content.preferredColorScheme(scheme))
            view.frame = NSRect(x: 0, y: 0, width: name.hasPrefix("mac-wizard") ? 1000 : 700, height: 650)
            let window = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
            window.appearance = NSAppearance(named: scheme == .light ? .aqua : .darkAqua)
            window.contentView = view
            view.layoutSubtreeIfNeeded()
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            XCTAssertGreaterThan(data.count, 5000)
            let suffix = scheme == .dark ? "-dark" : ""
            try data.write(to: directory.appendingPathComponent("\(name)\(suffix).png"))
        }
    }
}
