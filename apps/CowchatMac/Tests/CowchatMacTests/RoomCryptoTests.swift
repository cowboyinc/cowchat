import XCTest
@testable import CowchatMac

final class RoomCryptoTests: XCTestCase {
    // Also asserted by cowchat-core::crypto::tests::mac_interop_fixture.
    private let fixture = "cow1:AAECAwQFBgcICQoLslE85HeKJnKelFHeUqQa6pf4bJGu5ieHEkdN+PvuzvDrLKZke2UY2bHHmkLIy0w"

    func testRustWireFormatAndAuthentication() throws {
        XCTAssertEqual(try RoomCrypto.decrypt(fixture, secret: "fixture-secret", roomID: "room-test"), "Hello from Rust-compatible cow1")
        XCTAssertThrowsError(try RoomCrypto.decrypt(fixture, secret: "wrong", roomID: "room-test"))
        XCTAssertThrowsError(try RoomCrypto.decrypt(fixture, secret: "fixture-secret", roomID: "other-room"))
        XCTAssertThrowsError(try RoomCrypto.decrypt(String(fixture.dropLast()) + "A", secret: "fixture-secret", roomID: "room-test"))
        XCTAssertThrowsError(try RoomCrypto.decrypt("cow1:invalid", secret: "fixture-secret", roomID: "room-test"))
    }

    func testNativeEncryptionUsesFreshNonces() throws {
        let a = try RoomCrypto.encrypt("hello 🐮", secret: "secret", roomID: "room")
        let b = try RoomCrypto.encrypt("hello 🐮", secret: "secret", roomID: "room")
        XCTAssertNotEqual(a, b)
        XCTAssertEqual(try RoomCrypto.decrypt(a, secret: "secret", roomID: "room"), "hello 🐮")
    }

    func testKeychainAccountsSeparateEndpointsAndRooms() {
        let a = RoomCrypto.credentialAccount(profile: .local, roomID: "one")
        XCTAssertNotEqual(a, RoomCrypto.credentialAccount(profile: .local, roomID: "two"))
        XCTAssertNotEqual(a, RoomCrypto.credentialAccount(profile: .local(host: "127.0.0.1", port: 9230), roomID: "one"))
    }
}
