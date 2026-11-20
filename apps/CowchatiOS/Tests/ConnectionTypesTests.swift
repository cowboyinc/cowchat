import XCTest
@testable import CowchatiOS
import CowchatKit

final class ConnectionTypesTests: XCTestCase {
    func testDetectsABareInvite() {
        XCTAssertEqual(PastedCredential.detect("  cinv_abc-123_X \n"), .invite("cinv_abc-123_X"))
        XCTAssertEqual(PastedCredential.detect("cguest_invite_Zz9"), .invite("cguest_invite_Zz9"))
    }

    func testFindsTheInviteInsideAConnectPrompt() {
        let prompt = "Join my Cowchat room. Run: cowchat join --invite cinv_tok-EN_1 and say hello."
        XCTAssertEqual(PastedCredential.detect(prompt), .invite("cinv_tok-EN_1"))
    }

    func testDetectsAnAccountKey() {
        XCTAssertEqual(PastedCredential.detect("csoc_device123"), .accountKey("csoc_device123"))
    }

    func testTreatsAnyOtherSingleTokenAsAKey() {
        XCTAssertEqual(PastedCredential.detect(" 9f8e7d6c5b4a \n"), .key("9f8e7d6c5b4a"))
    }

    func testRejectsEmptyAndProseWithoutAToken() {
        XCTAssertEqual(PastedCredential.detect("   \n"), .unrecognized)
        XCTAssertEqual(PastedCredential.detect("ask Ryan for an invite"), .unrecognized)
    }

    func testFoundTextNeverContainsTheSecret() {
        for pasted in ["cinv_secretsecret", "csoc_secretsecret", "secretsecret"] {
            let found = PastedCredential.detect(pasted).found
            XCTAssertNotNil(found)
            XCTAssertFalse(found!.contains("secretsecret"))
        }
        XCTAssertNil(PastedCredential.detect("").found)
    }

    func testConnectionCredentialMapsFromAPaste() {
        XCTAssertEqual(PastedCredential.invite("cinv_a").credential, .invite("cinv_a"))
        XCTAssertEqual(PastedCredential.accountKey("csoc_a").credential, .key("csoc_a"))
        XCTAssertEqual(PastedCredential.key("k").credential, .key("k"))
        XCTAssertNil(PastedCredential.unrecognized.credential)
    }

    func testServersCompareByAddressNotSpelling() {
        XCTAssertTrue(ServerChoice.global(url: "wss://Chat.Example.com/").isSameServer(as: .global(url: "wss://chat.example.com")))
        XCTAssertFalse(ServerChoice.global(url: "wss://a.example.com").isSameServer(as: .global(url: "wss://b.example.com")))
        XCTAssertTrue(ServerChoice.tcp(host: "Mac.local", port: 9229).isSameServer(as: .tcp(host: "mac.local", port: 9229)))
        XCTAssertFalse(ServerChoice.tcp(host: "mac.local", port: 9229).isSameServer(as: .tcp(host: "mac.local", port: 9230)))
        XCTAssertFalse(ServerChoice.tcp(host: "a", port: 1).isSameServer(as: .global(url: "wss://a")))
    }

    func testServerLabel() {
        XCTAssertEqual(ServerChoice.global(url: "wss://global.cowchat.example/ws").label, "global.cowchat.example")
        XCTAssertEqual(ServerChoice.tcp(host: "mac.local", port: 9229).label, "mac.local:9229")
    }

    func testConnectionRecordRoundTrips() throws {
        let record = ConnectionRecord(id: "c1", server: .tcp(host: "h", port: 1), name: "Home", displayName: "Ryan")
        let decoded = try JSONDecoder().decode(ConnectionRecord.self, from: JSONEncoder().encode(record))
        XCTAssertEqual(decoded, record)
    }

    func testRoomRefsWithTheSameRoomIDOnTwoConnectionsDiffer() {
        XCTAssertNotEqual(RoomRef(connectionID: "a", roomID: "lobby"), RoomRef(connectionID: "b", roomID: "lobby"))
    }

    func testConnectionRowDoesNotRepeatTheServerWhenItIsTheName() {
        let server = ServerChoice.tcp(host: "mac.local", port: 9229)
        let unnamed = ConnectionRecord(id: "a", server: server, name: "mac.local:9229", displayName: "iPhone")
        let named = ConnectionRecord(id: "b", server: server, name: "Home", displayName: "iPhone")
        XCTAssertEqual(ConnectionState.subtitle(unnamed, source: .key(ending: "9f3a"), phase: .connected), "Key ••••9f3a · Connected")
        XCTAssertEqual(ConnectionState.subtitle(named, source: .key(ending: nil), phase: .connected), "Key · mac.local:9229 · Connected")
    }

    func testConnectionRowSaysHowTheConnectionWasMade() {
        let record = ConnectionRecord(id: "a", server: .defaultGlobal, name: ServerChoice.defaultGlobal.label, displayName: "iPhone")
        XCTAssertEqual(ConnectionState.subtitle(record, source: .account(handle: "ryan"), phase: .connected), "@ryan · Connected")
        XCTAssertEqual(ConnectionState.subtitle(record, source: .account(handle: nil), phase: .reconnecting), "Account · Reconnecting")
        XCTAssertEqual(ConnectionState.subtitle(record, source: .invite, phase: .connected), "Invited · Connected")
    }

    func testAConnectionSavedBeforeInvitesWereTrackedStillLoads() throws {
        let old = #"{"id":"c1","server":{"tcp":{"host":"h","port":1}},"name":"Home","displayName":"Ryan"}"#
        let decoded = try JSONDecoder().decode(ConnectionRecord.self, from: Data(old.utf8))
        XCTAssertFalse(decoded.fromInvite)
    }
}
