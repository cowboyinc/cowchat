#if DEBUG
import Foundation
import CowchatKit

/// Debug builds only, when the app is launched with `-demoAccount`: the real app over a made-up server and a made-up
/// account, both held in memory. It lets a test, or a person, see the tabs and rooms a signed-in account gets without
/// a Google sign-in. It makes no network calls and saves nothing outside its own settings suite.
@MainActor
enum DemoMode {
    static var isOn: Bool { ProcessInfo.processInfo.arguments.contains("-demoAccount") }

    static func makeStore() -> ConnectionsStore {
        let suite = "CowchatiOS.demo"
        UserDefaults().removePersistentDomain(forName: suite)
        return ConnectionsStore(
            makeConnection: { DemoConnection() },
            credentials: DemoCredentialStore(),
            defaults: UserDefaults(suiteName: suite)!,
            redeemInvite: { _, _ in "demo-key" },
            sleep: { try await Task.sleep(for: $0) },
            makeAccountAPI: { _, _ in DemoAccountAPI() }
        )
    }

    static func signIn(_ store: ConnectionsStore) async {
        guard store.connections.isEmpty else { return }
        _ = try? await store.add(server: .defaultGlobal, credential: .key("csoc_demo_0000"))
    }
}

private final class DemoCredentialStore: CowchatCredentialStore {
    private var values: [String: String] = [:]
    func credential(for account: String) throws -> String? { values[account] }
    func setCredential(_ credential: String?, for account: String) throws { values[account] = credential }
}

@MainActor
private final class DemoConnection: CowchatConnectionProtocol {
    var onEvent: ((String, [String: Any]) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?
    private var agentID = ""
    private var name = ""
    private var seq = 100

    private var rooms = [
        Room(roomID: "r1", name: "design", description: nil, parentID: nil, createdAt: "2026-11-18T08:00:00Z",
             createdBy: "a2", visibility: "private", lastActivity: "2026-11-18T10:05:00Z", memberCount: 3, encrypted: false),
        Room(roomID: "r2", name: "cowchat-main", description: nil, parentID: nil, createdAt: "2026-11-18T08:00:00Z",
             createdBy: "s1", visibility: "private", lastActivity: "2026-11-18T11:00:00Z", memberCount: 2, encrypted: false),
    ]

    private lazy var messages: [String: [ChatMessage]] = [
        "r1": [
            Self.message("m1", room: "r1", seq: 1, from: "a2", name: "Cal Fisher", kind: "person", text: "Can someone look at the plan?"),
            Self.message("m2", room: "r1", seq: 2, from: "s1", name: "cowchat-main", kind: "agent", external: true,
                         text: "Tests pass. Ready for review."),
            Self.message("m3", room: "r1", seq: 3, from: "s1", name: "cowchat-main", kind: "agent", text: "running the linter"),
            Self.message("m4", room: "r1", seq: 4, from: "s1", name: "cowchat-main", kind: "agent", text: "linter clean"),
            Self.message("m5", room: "r1", seq: 5, from: "a2", name: "Cal Fisher", kind: "person", text: "Thanks, merging."),
        ],
        "r2": [
            Self.message("n1", room: "r2", seq: 1, from: "s1", name: "cowchat-main", kind: "agent", external: true,
                         text: "Opened a pull request for the Inbox."),
        ],
    ]

    private static func message(_ id: String, room: String, seq: Int, from: String, name: String, kind: String?,
                                external: Bool? = nil, text: String) -> ChatMessage {
        var metadata: [String: Any] = [:]
        if let kind { metadata["sender_kind"] = kind }
        if let external { metadata["display_external"] = external }
        let object: [String: Any] = [
            "message_id": id, "room_id": room, "agent_id": from, "agent_name": name, "content": text,
            "timestamp": String(format: "2026-11-18T10:%02d:00Z", seq), "seq": seq, "metadata": metadata,
        ]
        return try! JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: object))
    }

    func reconfigure(profile: ConnectionProfile) {}
    func connect() async throws {}

    func register(name: String, agentID: String) async throws -> CowchatRegistration {
        self.name = name
        self.agentID = agentID
        return CowchatRegistration(agentID: agentID)
    }

    func listRooms() async throws -> [Room] { rooms }

    func listAgents(roomID: String) async throws -> [AgentPresence] {
        [AgentPresence(agentID: "s1", name: "cowchat-main", status: "working"), AgentPresence(agentID: agentID, name: name)]
    }

    func createRoom(name: String, description: String?, parentID: String?, isPublic: Bool) async throws -> Room {
        let room = Room(roomID: "new-\(rooms.count)", name: name, description: description, parentID: parentID,
                        createdAt: "2026-11-18T12:00:00Z", createdBy: agentID, visibility: "private",
                        lastActivity: nil, memberCount: 1, encrypted: false)
        rooms.append(room)
        return room
    }

    func rename(roomID: String, name: String) async throws -> Room {
        guard let index = rooms.firstIndex(where: { $0.id == roomID }) else { throw CowchatConnectionError.invalidResponse }
        let old = rooms[index]
        rooms[index] = Room(roomID: old.roomID, name: name, description: old.description, parentID: old.parentID,
                            createdAt: old.createdAt, createdBy: old.createdBy, visibility: old.visibility,
                            lastActivity: old.lastActivity, memberCount: old.memberCount, encrypted: old.encrypted)
        return rooms[index]
    }

    func destroy(roomID: String) async throws {}
    func createInvite(roomID: String, singleUse: Bool) async throws -> String { "cinv_demo" }
    func listInvites(roomID: String) async throws -> [RoomInvite] { [] }
    func revokeInvite(inviteID: String) async throws {}
    func redeemInvite(token: String) async throws -> String { "r1" }
    func join(roomID: String) async throws {}
    func leave(roomID: String) async throws {}

    func history(roomID: String, limit: Int, before: String?) async throws -> [ChatMessage] {
        before == nil ? Array((messages[roomID] ?? []).suffix(limit)) : []
    }

    func history(roomID: String, limit: Int, beforeSeq: Int?, kinds: [String]) async throws -> [ChatMessage] {
        let older = (messages[roomID] ?? []).filter { message in beforeSeq.map { message.seq < $0 } ?? true }
        return Array(older.suffix(limit))
    }

    func send(roomID: String, content: String, mentions: [String]) async throws -> ChatMessage {
        seq += 1
        let sent = Self.message("sent-\(seq)", room: roomID, seq: seq, from: agentID, name: name, kind: "person", text: content)
        messages[roomID, default: []].append(sent)
        return sent
    }

    func disconnect() {}
}
#endif
