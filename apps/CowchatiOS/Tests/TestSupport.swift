import Foundation
@testable import CowchatiOS
import CowchatKit

func makeMessage(
    id: String,
    room: String = "r1",
    seq: Int,
    from agentID: String = "agent-other",
    name: String = "Other",
    content: String? = nil,
    timestamp: String? = nil,
    type: String? = nil,
    senderKind: String? = nil,
    displayExternal: Bool? = nil,
    extra: [String: Any] = [:]
) -> ChatMessage {
    var object: [String: Any] = [
        "message_id": id,
        "room_id": room,
        "agent_id": agentID,
        "agent_name": name,
        "content": content ?? "message \(id)",
        "timestamp": timestamp ?? String(format: "2026-11-18T10:%02d:%02dZ", seq / 60, seq % 60),
        "seq": seq,
    ]
    var metadata: [String: Any] = extra
    if let type { metadata["type"] = type }
    if let senderKind { metadata["sender_kind"] = senderKind }
    if let displayExternal { metadata["display_external"] = displayExternal }
    if !metadata.isEmpty { object["metadata"] = metadata }
    let data = try! JSONSerialization.data(withJSONObject: object)
    return try! JSONDecoder().decode(ChatMessage.self, from: data)
}

func makeRoom(
    id: String, name: String, encrypted: Bool = false, lastActivity: String? = nil, createdBy: String? = nil
) -> Room {
    Room(
        roomID: id, name: name, description: nil, parentID: nil, createdAt: "",
        createdBy: createdBy, visibility: "private", lastActivity: lastActivity,
        memberCount: nil, encrypted: encrypted
    )
}

/// The smallest thing the chart reader accepts as a PNG: the signature and an IHDR header for a 2 by 1 image.
let tinyPNG = Data([137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0, 0, 0, 0])

func chartContent(title: String, png: Data = tinyPNG) -> String {
    let object = ["title": title, "png": png.base64EncodedString()]
    return "cowchat-chart:v1\n" + String(data: try! JSONSerialization.data(withJSONObject: object), encoding: .utf8)!
}

func entry(_ id: String, seq: Int, text: String? = nil, forPeople: Bool = true) -> TimelineEntry {
    TimelineEntry(
        id: id, seq: seq, senderID: "a", senderName: "A", isMine: false,
        date: Date(timeIntervalSince1970: TimeInterval(seq)), body: .text(text ?? id), isForPeople: forPeople
    )
}

func messagePayload(_ message: ChatMessage) -> [String: Any] {
    let data = try! JSONEncoder().encode(message)
    return try! JSONSerialization.jsonObject(with: data) as! [String: Any]
}

final class InMemoryCredentialStore: CowchatCredentialStore {
    var values: [String: String] = [:]

    func credential(for account: String) throws -> String? { values[account] }

    func setCredential(_ credential: String?, for account: String) throws {
        values[account] = credential
    }
}

struct FakeError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

@MainActor
final class FakeConnection: CowchatConnectionProtocol {
    var onEvent: ((String, [String: Any]) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?

    var profile: ConnectionProfile?
    var serverRooms: [Room] = []
    var serverMessages: [String: [ChatMessage]] = [:]
    var serverAgents: [String: [AgentPresence]] = [:]
    var connectFailures: [Error] = []
    var registerError: Error?
    var isConnected = false
    var registeredAgentIDs: [String] = []
    var registeredNames: [String] = []
    var joinedRooms: Set<String> = []
    private var nextSeq = 1000

    func reconfigure(profile: ConnectionProfile) { self.profile = profile }

    func connect() async throws {
        if !connectFailures.isEmpty {
            throw connectFailures.removeFirst()
        }
        isConnected = true
    }

    func register(name: String, agentID: String) async throws -> CowchatRegistration {
        try requireConnected()
        if let registerError { throw registerError }
        registeredNames.append(name)
        registeredAgentIDs.append(agentID)
        return CowchatRegistration(agentID: agentID)
    }

    func listRooms() async throws -> [Room] {
        try requireConnected()
        return serverRooms
    }

    func listAgents(roomID: String) async throws -> [AgentPresence] {
        try requireConnected()
        return serverAgents[roomID] ?? []
    }

    var joinableRooms: [Room] = []

    func listJoinableRooms() async throws -> [Room] {
        try requireConnected()
        return joinableRooms
    }

    var afterJoinForHistoryTest: ((String) -> Void)?

    func join(roomID: String) async throws {
        try requireConnected()
        joinedRooms.insert(roomID)
        afterJoinForHistoryTest?(roomID)
        if let index = joinableRooms.firstIndex(where: { $0.id == roomID }) {
            serverRooms.append(joinableRooms.remove(at: index))
        }
    }

    func emit(_ type: String, _ payload: [String: Any]) { onEvent?(type, payload) }

    func emit(_ type: String, room: Room) {
        let data = try! JSONEncoder().encode(room)
        onEvent?(type, try! JSONSerialization.jsonObject(with: data) as! [String: Any])
    }

    var historyErrors: [String: Error] = [:]
    var beforeHistory: (() -> Void)?

    /// As the server pages: only messages below the sequence, whatever their timestamps.
    /// The `beforeSeq` of each page asked for by sequence, in order.
    var sequenceRequests: [Int?] = []

    func history(roomID: String, limit: Int, beforeSeq: Int?, kinds: [String]) async throws -> [ChatMessage] {
        sequenceRequests.append(beforeSeq)
        beforeHistory?()
        try requireConnected()
        if let error = historyErrors[roomID] { throw error }
        let all = (serverMessages[roomID] ?? []).sorted { $0.seq < $1.seq }.filter { message in beforeSeq.map { message.seq < $0 } ?? true }
        return Array(all.suffix(limit))
    }

    func history(roomID: String, limit: Int, before: String?) async throws -> [ChatMessage] {
        beforeHistory?()
        try requireConnected()
        if let error = historyErrors[roomID] { throw error }
        let all = (serverMessages[roomID] ?? []).sorted { $0.seq < $1.seq }
        let eligible = before.map { cutoff in all.filter { $0.timestamp < cutoff } } ?? all
        return Array(eligible.suffix(limit))
    }

    func send(roomID: String, content: String, mentions: [String]) async throws -> ChatMessage {
        try requireConnected()
        nextSeq += 1
        let message = makeMessage(
            id: "sent-\(nextSeq)", room: roomID, seq: nextSeq,
            from: registeredAgentIDs.last ?? "", name: registeredNames.last ?? "",
            content: content
        )
        serverMessages[roomID, default: []].append(message)
        lastSentMentions = mentions
        return message
    }

    var lastSentMentions: [String] = []

    func disconnect() { isConnected = false }

    func drop() {
        isConnected = false
        onStatusChange?(.failed("The network connection was lost."))
    }

    func deliver(_ message: ChatMessage) {
        serverMessages[message.roomID, default: []].append(message)
        onEvent?("message_received", messagePayload(message))
    }

    var createdRooms: [(name: String, isPublic: Bool)] = []

    func createRoom(name: String, description: String?, parentID: String?, isPublic: Bool) async throws -> Room {
        try requireConnected()
        if serverRooms.contains(where: { $0.name == name }) {
            throw CowchatConnectionError.server(message: "Room name '\(name)' already taken", code: "room_name_taken")
        }
        createdRooms.append((name, isPublic))
        let room = makeRoom(id: "new-\(createdRooms.count)", name: name, createdBy: registeredAgentIDs.last)
        serverRooms.append(room)
        return room
    }
    var renameError: Error?

    func rename(roomID: String, name: String) async throws -> Room {
        try requireConnected()
        if let renameError { throw renameError }
        guard let index = serverRooms.firstIndex(where: { $0.id == roomID }) else {
            throw CowchatConnectionError.server(message: "Room not found", code: "room_not_found")
        }
        let old = serverRooms[index]
        serverRooms[index] = makeRoom(
            id: old.id, name: name, encrypted: old.encrypted, lastActivity: old.lastActivity, createdBy: old.createdBy
        )
        return serverRooms[index]
    }
    func destroy(roomID: String) async throws {}

    var createdInvites: [(roomID: String, singleUse: Bool)] = []

    func createInvite(roomID: String, singleUse: Bool) async throws -> String {
        try requireConnected()
        createdInvites.append((roomID, singleUse))
        return "cinv_made\(createdInvites.count)"
    }
    func listInvites(roomID: String) async throws -> [RoomInvite] { [] }
    func revokeInvite(inviteID: String) async throws {}
    var inviteRooms: [String: Room] = [:]
    var redeemedInviteTokens: [String] = []

    func redeemInvite(token: String) async throws -> String {
        try requireConnected()
        redeemedInviteTokens.append(token)
        guard let room = inviteRooms[token] else {
            throw CowchatConnectionError.server(message: "invite not found", code: "not_found")
        }
        if !serverRooms.contains(where: { $0.id == room.id }) { serverRooms.append(room) }
        return room.id
    }
    func leave(roomID: String) async throws {}

    private func requireConnected() throws {
        guard isConnected else { throw CowchatConnectionError.notConnected }
    }
}

@MainActor
func eventually(
    timeout: TimeInterval = 2,
    _ condition: () -> Bool
) async -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if condition() { return true }
        await Task.yield()
        try? await Task.sleep(nanoseconds: 1_000_000)
    }
    return condition()
}
