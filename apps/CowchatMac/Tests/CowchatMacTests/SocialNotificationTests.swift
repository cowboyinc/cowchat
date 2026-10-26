import Combine
import XCTest
@testable import CowchatMac

final class SocialNotificationTests: XCTestCase {
    private let me = SocialAccount(id: "me", handle: "chad", displayName: "Chad")
    private let peer = SocialAccount(id: "peer", handle: "renzo", displayName: "Renzo")

    private func snapshot(requests: [SocialFriendRequest] = [], invitations: [SocialRoomInvitation] = [],
                          account: SocialAccount? = nil, endpoint: String = "wss://fixture.example/ws", scope: String = "session") -> SocialNotificationSnapshot {
        SocialNotificationSnapshot(session: SocialSession(endpoint: URL(string: endpoint)!, deviceKey: "csoc_fixture", account: account ?? me, scopeId: scope), requests: requests, invitations: invitations)
    }

    private func request(_ id: String, outgoing: Bool = false, status: String = "pending") -> SocialFriendRequest {
        SocialFriendRequest(id: id, from: outgoing ? me : peer, to: outgoing ? peer : me, status: status)
    }

    private func invitation(_ id: String, to: SocialAccount? = nil, status: String = "pending") -> SocialRoomInvitation {
        SocialRoomInvitation(id: id, roomId: "room", roomName: "Engineering", from: peer, to: to ?? me,
            proposedAgents: [], members: [], agents: [], historyPolicy: "from_join", encrypted: false, slackConnected: false, status: status)
    }

    func testFirstSnapshotIsSilentAndNewActionableEventsHaveDestinations() {
        var ledger = SocialNotificationLedger()
        XCTAssertTrue(ledger.consume(snapshot(requests: [request("old")], invitations: [invitation("old")])).isEmpty)
        let events = ledger.consume(snapshot(requests: [request("old"), request("new"), request("out", outgoing: true), request("done", outgoing: true, status: "accepted")], invitations: [invitation("old"), invitation("new")]))
        XCTAssertEqual(events.map(\.kind), [.friendRequest, .friendAccepted, .roomInvitation])
        XCTAssertEqual(events.map { $0.kind.destination }, [.friends, .friends, .invitations])
        XCTAssertEqual(events.map(\.body), ["@renzo wants to be your friend.", "@renzo accepted your friend request.", "@renzo invited you to Engineering."])
    }

    func testStatusTransitionAlertsOnceAcrossReorderingDisappearanceAndRestart() throws {
        var ledger = SocialNotificationLedger()
        XCTAssertTrue(ledger.consume(snapshot(requests: [request("out", outgoing: true)])).isEmpty)
        let accepted = snapshot(requests: [request("out", outgoing: true, status: "accepted")])
        XCTAssertEqual(ledger.consume(accepted).map(\.kind), [.friendAccepted])
        XCTAssertTrue(ledger.consume(snapshot()).isEmpty)
        let data = try JSONEncoder().encode(ledger)
        var restored = try JSONDecoder().decode(SocialNotificationLedger.self, from: data)
        XCTAssertTrue(restored.consume(accepted).isEmpty)
        let new = snapshot(requests: [request("two"), request("one"), request("two")])
        XCTAssertEqual(restored.consume(new).count, 2)
        XCTAssertTrue(restored.consume(snapshot(requests: Array(new.requests.reversed()))).isEmpty)
    }

    func testOtherRecipientsOutgoingPendingAndDeclinedEventsAreIgnored() {
        var ledger = SocialNotificationLedger()
        _ = ledger.consume(snapshot())
        let events = ledger.consume(snapshot(requests: [request("out", outgoing: true), request("declined", status: "declined"), request("accepted-incoming", status: "accepted")],
            invitations: [invitation("other", to: peer), invitation("declined", status: "declined")]))
        XCTAssertTrue(events.isEmpty)
    }

    func testPersistenceScopeExcludesCredentialsAndSeparatesAccountsAndServers() {
        let original = snapshot()
        let renewed = snapshot(scope: "new-login")
        XCTAssertEqual(original.accountScope, renewed.accountScope)
        XCTAssertNotEqual(original.deliveryScope, renewed.deliveryScope)
        XCTAssertNotEqual(original.accountScope, snapshot(account: peer).accountScope)
        XCTAssertNotEqual(original.accountScope, snapshot(endpoint: "wss://other.example/ws").accountScope)
        XCTAssertFalse(original.deliveryScope.contains("csoc_"))
    }

    @MainActor
    func testNotificationRouteIsConsumedOnceAndCannotReplayIntoAnotherSession() async throws {
        let suite = "CowchatNotificationRoutes.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let service = SocialNotifications(defaults: defaults)
        service.enabled = false
        let current = snapshot()
        service.setSession(current.session)
        let routed = expectation(description: "One presenter claims the notification route")
        var consumed = 0
        let subscription = service.$destination.receive(on: RunLoop.main).sink { section in
            if let section, service.consumeDestination(section) {
                consumed += 1
                XCTAssertEqual(section, .invitations)
                routed.fulfill()
            }
        }
        XCTAssertTrue(service.route(scope: current.deliveryScope, section: .invitations))
        await fulfillment(of: [routed], timeout: 1)
        XCTAssertEqual(consumed, 1)
        XCTAssertNil(service.destination)
        XCTAssertFalse(service.consumeDestination(.invitations))
        subscription.cancel()
        XCTAssertTrue(service.route(scope: current.deliveryScope, section: .friends))
        service.setSession(nil)
        XCTAssertNil(service.destination)
        XCTAssertFalse(service.route(scope: current.deliveryScope, section: .friends))
        service.setSession(snapshot(account: peer).session)
        XCTAssertFalse(service.route(scope: current.deliveryScope, section: .friends))
    }

    @MainActor
    func testDisabledNotificationsAdvanceBaselineAndRejectOldSessionSnapshots() throws {
        let suite = "CowchatNotificationTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let service = SocialNotifications(defaults: defaults)
        let initial = snapshot()
        service.setSession(initial.session)
        service.enabled = false
        service.receive(initial)
        service.receive(snapshot(requests: [request("while-disabled")]))
        let key = "CowchatMac.notifications.seen.v1." + initial.accountScope
        var ledger = try JSONDecoder().decode(SocialNotificationLedger.self, from: XCTUnwrap(defaults.data(forKey: key)))
        XCTAssertTrue(ledger.consume(snapshot(requests: [request("while-disabled")])).isEmpty)
        let oldData = defaults.data(forKey: key)
        service.setSession(nil)
        service.receive(snapshot(requests: [request("late-response")]))
        XCTAssertEqual(defaults.data(forKey: key), oldData)
        service.friendRequests = false
        service.acceptedRequests = false
        service.roomInvitations = false
        let restored = SocialNotifications(defaults: defaults)
        XCTAssertFalse(restored.enabled)
        XCTAssertFalse(restored.friendRequests)
        XCTAssertFalse(restored.acceptedRequests)
        XCTAssertFalse(restored.roomInvitations)
    }
}
