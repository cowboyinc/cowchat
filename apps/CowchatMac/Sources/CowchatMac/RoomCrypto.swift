import CryptoKit
import Foundation

/// The existing cow1 wire format: unpadded base64(nonce || ciphertext || tag).
/// The shared secret stays on the client; HKDF scopes it to one room UUID.
enum RoomCrypto {
    enum Failure: LocalizedError {
        case invalidCiphertext
        case invalidText
        var errorDescription: String? { "The room key could not decrypt this message." }
    }

    private static func key(secret: String, roomID: String) -> SymmetricKey {
        HKDF<SHA256>.deriveKey(
            inputKeyMaterial: SymmetricKey(data: Data(secret.utf8)),
            salt: Data(), info: Data("cowchat-e2e-v1:\(roomID)".utf8), outputByteCount: 32
        )
    }

    static func encrypt(_ text: String, secret: String, roomID: String) throws -> String {
        let sealed = try ChaChaPoly.seal(Data(text.utf8), using: key(secret: secret, roomID: roomID))
        return "cow1:" + sealed.combined.base64EncodedString().replacingOccurrences(of: "=", with: "")
    }

    static func decrypt(_ ciphertext: String, secret: String, roomID: String) throws -> String {
        guard ciphertext.hasPrefix("cow1:") else { throw Failure.invalidCiphertext }
        let encoded = String(ciphertext.dropFirst(5))
        let padded = encoded + String(repeating: "=", count: (4 - encoded.count % 4) % 4)
        guard let data = Data(base64Encoded: padded) else { throw Failure.invalidCiphertext }
        let box = try ChaChaPoly.SealedBox(combined: data)
        let plaintext = try ChaChaPoly.open(box, using: key(secret: secret, roomID: roomID))
        guard let text = String(data: plaintext, encoding: .utf8) else { throw Failure.invalidText }
        return text
    }

    static func credentialAccount(profile: ConnectionProfile, roomID: String) -> String {
        let scope = profile.endpointDescription + "\n" + (profile.persistentIdentityScope ?? "local")
        let digest = SHA256.hash(data: Data(scope.utf8)).map { String(format: "%02x", $0) }.joined()
        return "room-key.\(digest).\(roomID)"
    }
}
