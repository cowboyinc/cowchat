import XCTest
@testable import CowchatiOS
import CowchatKit

final class SmokeTests: XCTestCase {
    func testSharedRoomCryptoRoundTrips() throws {
        let sealed = try RoomCrypto.encrypt("hello", secret: "s3cret", roomID: "room-1")
        XCTAssertEqual(try RoomCrypto.decrypt(sealed, secret: "s3cret", roomID: "room-1"), "hello")
    }
}
