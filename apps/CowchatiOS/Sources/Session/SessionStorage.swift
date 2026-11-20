import CryptoKit
import Foundation
import CowchatKit

struct ReadState: Codable, Equatable {
    var lastRead = 0
    var latest = 0

    var isUnread: Bool { latest > lastRead }
}

/// One connection's saved state. Every key carries the connection id, so connections never share a key,
/// an agent identity, read marks or room secrets.
struct SessionStorage {
    let defaults: UserDefaults
    let credentials: any CowchatCredentialStore
    let connectionID: String

    private enum Keys: String, CaseIterable {
        case agentID, agentIDKeyFingerprint, readState, secretRoomIDs, invitedRoomIDs, removedRoomIDs, archivedRoomIDs, showAllRoomIDs
    }

    private func key(_ key: Keys) -> String { "CowchatiOS.connection.\(connectionID).\(key.rawValue)" }
    private var apiKeyAccount: String { "\(connectionID).api-key" }
    private func secretAccount(_ roomID: String) -> String { "\(connectionID).room-secret.\(roomID)" }

    var apiKey: String? { try? credentials.credential(for: apiKeyAccount) }

    func save(apiKey: String) throws {
        try credentials.setCredential(apiKey, for: apiKeyAccount)
    }

    // The server binds an agent id to the key that first registered it, so each key gets its own.
    func stableAgentID(apiKey: String) -> String {
        let fingerprint = SHA256.hash(data: Data(apiKey.utf8)).prefix(8).map { String(format: "%02x", $0) }.joined()
        if defaults.string(forKey: key(.agentIDKeyFingerprint)) == fingerprint,
           let existing = defaults.string(forKey: key(.agentID)), !existing.isEmpty {
            return existing
        }
        defaults.set(fingerprint, forKey: key(.agentIDKeyFingerprint))
        return rotateAgentID()
    }

    @discardableResult
    func rotateAgentID() -> String {
        let generated = "cowchat-ios-\(UUID().uuidString.lowercased())"
        defaults.set(generated, forKey: key(.agentID))
        return generated
    }

    var readStates: [String: ReadState] {
        get {
            defaults.data(forKey: key(.readState))
                .flatMap { try? JSONDecoder().decode([String: ReadState].self, from: $0) } ?? [:]
        }
        nonmutating set {
            defaults.set(try? JSONEncoder().encode(newValue), forKey: key(.readState))
        }
    }

    /// Rooms an invite added to this connection's key.
    var invitedRoomIDs: Set<String> {
        get { Set(defaults.stringArray(forKey: key(.invitedRoomIDs)) ?? []) }
        nonmutating set { defaults.set(newValue.sorted(), forKey: key(.invitedRoomIDs)) }
    }

    /// Invited rooms the user removed. The server keeps listing them, since it has no way to give an invite back.
    var removedRoomIDs: Set<String> {
        get { Set(defaults.stringArray(forKey: key(.removedRoomIDs)) ?? []) }
        nonmutating set { defaults.set(newValue.sorted(), forKey: key(.removedRoomIDs)) }
    }

    /// Rooms hidden from the room list on this iPhone.
    var archivedRoomIDs: Set<String> {
        get { Set(defaults.stringArray(forKey: key(.archivedRoomIDs)) ?? []) }
        nonmutating set { defaults.set(newValue.sorted(), forKey: key(.archivedRoomIDs)) }
    }

    /// Rooms whose Show all messages switch is on.
    var showAllRoomIDs: Set<String> {
        get { Set(defaults.stringArray(forKey: key(.showAllRoomIDs)) ?? []) }
        nonmutating set { defaults.set(newValue.sorted(), forKey: key(.showAllRoomIDs)) }
    }

    func secret(roomID: String) -> String? {
        try? credentials.credential(for: secretAccount(roomID))
    }

    func expectsSecret(roomID: String) -> Bool {
        (defaults.stringArray(forKey: key(.secretRoomIDs)) ?? []).contains(roomID)
    }

    func setSecret(_ secret: String?, roomID: String) throws {
        try credentials.setCredential(secret, for: secretAccount(roomID))
        var ids = Set(defaults.stringArray(forKey: key(.secretRoomIDs)) ?? [])
        if secret == nil { ids.remove(roomID) } else { ids.insert(roomID) }
        defaults.set(ids.sorted(), forKey: key(.secretRoomIDs))
    }

    func eraseCredentials() {
        for roomID in defaults.stringArray(forKey: key(.secretRoomIDs)) ?? [] {
            try? credentials.setCredential(nil, for: secretAccount(roomID))
        }
        try? credentials.setCredential(nil, for: apiKeyAccount)
        defaults.removeObject(forKey: key(.secretRoomIDs))
        defaults.removeObject(forKey: key(.readState))
    }

    /// Everything, including the agent identity. For a connection that is being removed for good.
    func eraseAll() {
        eraseCredentials()
        Keys.allCases.forEach { defaults.removeObject(forKey: key($0)) }
    }
}
