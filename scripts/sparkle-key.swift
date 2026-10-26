import Foundation
import CryptoKit

// Sparkle's current export format is a base64 encoded 32-byte Ed25519 seed.
let input = FileHandle.standardInput.readDataToEndOfFile()
guard let text = String(data: input, encoding: .utf8),
      let seed = Data(base64Encoded: text.trimmingCharacters(in: .whitespacesAndNewlines)),
      seed.count == 32 else {
    fputs("Expected a base64 encoded 32-byte Sparkle key.\n", stderr)
    exit(1)
}
do {
    let key = try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
    print(key.publicKey.rawRepresentation.base64EncodedString())
} catch {
    fputs("Invalid Sparkle private key.\n", stderr)
    exit(1)
}
