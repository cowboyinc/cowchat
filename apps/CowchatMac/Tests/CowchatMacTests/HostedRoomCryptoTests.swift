import XCTest
@testable import CowchatMac

final class HostedRoomCryptoTests: XCTestCase {
    private struct Fixture: Decodable {
        struct Vector: Decodable {
            let room_id: String
            let key_epoch: String
            let message_id: String
            let text: String
            let wire: String
        }
        struct Invalid: Decodable { let name: String; let wire: String }
        let vectors: [Vector]
        let invalid: [Invalid]
    }
    private let key = Data((0..<32).map { UInt8($0) })
    private let context = HostedRoomCrypto.Context(roomID: "room-test", keyEpoch: 0, messageID: "message-1")

    private func fixture() throws -> Fixture {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../../../fixtures/hosted-room-message.json")
        return try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: url))
    }

    func testIndependentVectorsAndNativeEncryption() throws {
        for vector in try fixture().vectors {
            let scope = HostedRoomCrypto.Context(roomID: vector.room_id, keyEpoch: try XCTUnwrap(UInt64(vector.key_epoch)), messageID: vector.message_id)
            XCTAssertEqual(try HostedRoomCrypto.decrypt(vector.wire, key: key, context: scope), vector.text)
            let sealed = try HostedRoomCrypto.encrypt(vector.text, key: key, context: scope)
            XCTAssertEqual(try HostedRoomCrypto.decrypt(sealed, key: key, context: scope), vector.text)
        }
    }

    func testMalformedPayloadsAndWrongContextAreRejected() throws {
        let data = try fixture()
        for invalid in data.invalid {
            XCTAssertThrowsError(try HostedRoomCrypto.decrypt(invalid.wire, key: key, context: context), invalid.name)
        }
        let wire = data.vectors[0].wire
        for scope in [
            HostedRoomCrypto.Context(roomID: "other", keyEpoch: 0, messageID: "message-1"),
            HostedRoomCrypto.Context(roomID: "room-test", keyEpoch: 1, messageID: "message-1"),
            HostedRoomCrypto.Context(roomID: "room-test", keyEpoch: 0, messageID: "other"),
        ] {
            XCTAssertThrowsError(try HostedRoomCrypto.decrypt(wire, key: key, context: scope))
        }
        XCTAssertThrowsError(try HostedRoomCrypto.decrypt(wire, key: Data(repeating: 0, count: 32), context: context))
        for malformed in [wire + "=", wire + "\n", "cow1:", "plain"] {
            XCTAssertThrowsError(try HostedRoomCrypto.decrypt(malformed, key: key, context: context))
        }
        let encoded = String(wire.dropFirst(5))
        var damaged = try XCTUnwrap(Data(base64Encoded: encoded + String(repeating: "=", count: (4 - encoded.count % 4) % 4)))
        damaged[damaged.count - 1] ^= 1
        let badWire = "cow1:" + damaged.base64EncodedString().replacingOccurrences(of: "=", with: "")
        XCTAssertThrowsError(try HostedRoomCrypto.decrypt(badWire, key: key, context: context))
    }

    func testFreshNoncesAndInvalidKeyOrContext() throws {
        let a = try HostedRoomCrypto.encrypt("hello 🐎", key: key, context: context)
        let b = try HostedRoomCrypto.encrypt("hello 🐎", key: key, context: context)
        XCTAssertNotEqual(a, b)
        XCTAssertThrowsError(try HostedRoomCrypto.encrypt("text", key: Data(repeating: 0, count: 31), context: context))
        XCTAssertThrowsError(try HostedRoomCrypto.encrypt("text", key: key, context: .init(roomID: "", keyEpoch: 0, messageID: "m")))
        XCTAssertThrowsError(try HostedRoomCrypto.encrypt("text", key: key, context: .init(roomID: "r", keyEpoch: 0, messageID: "")))
    }

    func testContextUsesExactUTF8NotSwiftCanonicalEquivalence() throws {
        let vector = try fixture().vectors[2]
        XCTAssertThrowsError(try HostedRoomCrypto.decrypt(vector.wire, key: key, context: .init(roomID: "room-e\u{0301}", keyEpoch: 1, messageID: vector.message_id)))
        XCTAssertThrowsError(try HostedRoomCrypto.decrypt(vector.wire, key: key, context: .init(roomID: vector.room_id, keyEpoch: 1, messageID: "message-e\u{0301}")))
    }
}
