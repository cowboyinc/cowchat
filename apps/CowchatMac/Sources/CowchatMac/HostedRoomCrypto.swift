import CryptoKit
import Foundation

/// Raw 256-bit keys and authenticated context for hosted rooms, distinct from local shared secrets.
enum HostedRoomCrypto {
    struct Context {
        let roomID: String
        let keyEpoch: UInt64
        let messageID: String
    }

    enum Failure: Error { case invalidMessage }
    private static let domain = "cowchat-room-message-v1"

    private static func validate(_ key: Data, _ context: Context) throws {
        guard key.count == 32, !context.roomID.isEmpty, !context.messageID.isEmpty else {
            throw Failure.invalidMessage
        }
    }

    /// Seal once and persist the result before sending; retries reuse these bytes and message ID.
    static func encrypt(_ text: String, key: Data, context: Context) throws -> String {
        try validate(key, context)
        let fields = [domain, context.roomID, String(context.keyEpoch), context.messageID, text]
        let plaintext = try JSONSerialization.data(withJSONObject: fields, options: [.withoutEscapingSlashes])
        let sealed = try ChaChaPoly.seal(plaintext, using: SymmetricKey(data: key))
        return "cow1:" + sealed.combined.base64EncodedString().replacingOccurrences(of: "=", with: "")
    }

    static func decrypt(_ content: String, key: Data, context: Context) throws -> String {
        try validate(key, context)
        do {
            guard content.hasPrefix("cow1:") else { throw Failure.invalidMessage }
            let encoded = String(content.dropFirst(5))
            let padded = encoded + String(repeating: "=", count: (4 - encoded.count % 4) % 4)
            guard let bytes = Data(base64Encoded: padded), bytes.count >= 28,
                  bytes.base64EncodedString().replacingOccurrences(of: "=", with: "") == encoded else {
                throw Failure.invalidMessage
            }
            let box = try ChaChaPoly.SealedBox(combined: bytes)
            let plaintext = try ChaChaPoly.open(box, using: SymmetricKey(data: key))
            // Foundation also accepts UTF-16/32 JSON. The wire contract is UTF-8
            // without a BOM; raw NUL cannot occur in a valid JSON document.
            guard String(data: plaintext, encoding: .utf8) != nil, !plaintext.contains(0),
                  !plaintext.starts(with: [0xEF, 0xBB, 0xBF]) else { throw Failure.invalidMessage }
            guard let fields = try JSONSerialization.jsonObject(with: plaintext) as? [String], fields.count == 5,
                  fields[0] == domain, fields[1].utf8.elementsEqual(context.roomID.utf8),
                  fields[2] == String(context.keyEpoch), fields[3].utf8.elementsEqual(context.messageID.utf8) else {
                throw Failure.invalidMessage
            }
            return fields[4]
        } catch {
            throw Failure.invalidMessage
        }
    }
}
