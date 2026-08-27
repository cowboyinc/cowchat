import Foundation

enum CowchatWireProtocol {
    static let currentVersion = 2
}

struct CowchatRegistration: Equatable {
    let agentID: String
    let restoredRoomIDs: Set<String>

    init(agentID: String, restoredRoomIDs: Set<String> = []) {
        self.agentID = agentID
        self.restoredRoomIDs = restoredRoomIDs
    }
}

struct Room: Codable, Identifiable, Hashable {
    let roomID: String
    let name: String
    let description: String?
    let parentID: String?
    let createdAt: String
    let createdBy: String?
    let visibility: String
    let lastActivity: String?
    let memberCount: Int?
    let encrypted: Bool

    var id: String { roomID }

    enum CodingKeys: String, CodingKey {
        case roomID = "room_id"
        case name, description
        case parentID = "parent_id"
        case createdAt = "created_at"
        case createdBy = "created_by"
        case visibility
        case lastActivity = "last_activity"
        case memberCount = "member_count"
        case encrypted
    }

    init(
        roomID: String,
        name: String,
        description: String?,
        parentID: String?,
        createdAt: String,
        createdBy: String?,
        visibility: String,
        lastActivity: String?,
        memberCount: Int?,
        encrypted: Bool
    ) {
        self.roomID = roomID
        self.name = name
        self.description = description
        self.parentID = parentID
        self.createdAt = createdAt
        self.createdBy = createdBy
        self.visibility = visibility
        self.lastActivity = lastActivity
        self.memberCount = memberCount
        self.encrypted = encrypted
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        roomID = try values.decode(String.self, forKey: .roomID)
        name = try values.decode(String.self, forKey: .name)
        description = try values.decodeIfPresent(String.self, forKey: .description)
        parentID = try values.decodeIfPresent(String.self, forKey: .parentID)
        createdAt = try values.decodeIfPresent(String.self, forKey: .createdAt) ?? ""
        createdBy = try values.decodeIfPresent(String.self, forKey: .createdBy)
        visibility = try values.decodeIfPresent(String.self, forKey: .visibility) ?? "private"
        lastActivity = try values.decodeIfPresent(String.self, forKey: .lastActivity)
        memberCount = try values.decodeIfPresent(Int.self, forKey: .memberCount)
        encrypted = try values.decodeIfPresent(Bool.self, forKey: .encrypted) ?? false
    }

    func updating(lastActivity: String) -> Room {
        Room(
            roomID: roomID,
            name: name,
            description: description,
            parentID: parentID,
            createdAt: createdAt,
            createdBy: createdBy,
            visibility: visibility,
            lastActivity: lastActivity,
            memberCount: memberCount,
            encrypted: encrypted
        )
    }

    func updating(memberCount: Int) -> Room {
        Room(
            roomID: roomID,
            name: name,
            description: description,
            parentID: parentID,
            createdAt: createdAt,
            createdBy: createdBy,
            visibility: visibility,
            lastActivity: lastActivity,
            memberCount: memberCount,
            encrypted: encrypted
        )
    }
}

struct MessageMetadata: Codable, Equatable {
    let type: String?
    let kind: String?
    let handoff: HandoffContext?

    init(type: String? = nil, kind: String? = nil, handoff: HandoffContext? = nil) {
        self.type = type
        self.kind = kind
        self.handoff = handoff
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        type = try? values.decode(String.self, forKey: .type)
        kind = try? values.decode(String.self, forKey: .kind)
        handoff = try? values.decode(HandoffContext.self, forKey: .handoff)
    }

    private enum CodingKeys: String, CodingKey {
        case type, kind, handoff
    }
}

struct HandoffContext: Codable, Equatable {
    let version: Int
    let summary: String
    let next: String
    let risks: [String]
    let refs: [String]

    var isValid: Bool {
        version == 1
            && isRequiredText(summary)
            && isRequiredText(next)
            && risks.count <= 10
            && refs.count <= 10
            && risks.allSatisfy(isBoundedItem)
            && refs.allSatisfy(isBoundedItem)
    }

    private func isRequiredText(_ value: String) -> Bool {
        !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && value.count <= 2_000
    }

    private func isBoundedItem(_ value: String) -> Bool {
        !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && value.count <= 500
    }
}

struct ChatMessage: Codable, Identifiable, Equatable {
    let messageID: String
    let roomID: String
    let agentID: String
    let agentName: String
    let content: String
    let replyToMessage: String?
    let metadata: MessageMetadata
    let timestamp: String
    let seq: Int

    var id: String { messageID }

    enum CodingKeys: String, CodingKey {
        case messageID = "message_id"
        case roomID = "room_id"
        case agentID = "agent_id"
        case agentName = "agent_name"
        case content
        case replyToMessage = "reply_to_message"
        case metadata
        case timestamp, seq
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        messageID = try values.decode(String.self, forKey: .messageID)
        roomID = try values.decode(String.self, forKey: .roomID)
        agentID = try values.decode(String.self, forKey: .agentID)
        agentName = try values.decode(String.self, forKey: .agentName)
        content = try values.decode(String.self, forKey: .content)
        replyToMessage = try values.decodeIfPresent(String.self, forKey: .replyToMessage)
        // Metadata is an arbitrary JSON value on the wire. Read the optional
        // string `type` when it has the expected object shape, but never drop
        // an otherwise valid message because another client sent a scalar,
        // array, or differently typed field.
        metadata = (try? values.decode(MessageMetadata.self, forKey: .metadata))
            ?? MessageMetadata()
        timestamp = try values.decodeIfPresent(String.self, forKey: .timestamp) ?? ""
        seq = try values.decodeIfPresent(Int.self, forKey: .seq) ?? 0
    }

    var isThinking: Bool {
        metadata.type?.localizedCaseInsensitiveCompare("thinking") == .orderedSame
    }

    var handoff: HandoffContext? {
        guard metadata.kind == "handoff.ready", let handoff = metadata.handoff, handoff.isValid else {
            return nil
        }
        return handoff
    }
}

struct MessageArrivalIdentity: Equatable {
    let messageID: String
    let sequence: Int

    static func latest(in messages: [ChatMessage]) -> MessageArrivalIdentity? {
        messages.last.map {
            MessageArrivalIdentity(messageID: $0.id, sequence: $0.seq)
        }
    }
}

struct AgentPresence: Codable, Identifiable, Equatable {
    let agentID: String
    let name: String
    let capabilities: [String]
    let connectedAt: String?
    let lastActive: String?
    let status: String?
    let statusDetail: String?
    let progress: Int?

    var id: String { agentID }

    enum CodingKeys: String, CodingKey {
        case agentID = "agent_id"
        case name, capabilities
        case connectedAt = "connected_at"
        case lastActive = "last_active"
        case status
        case statusDetail = "status_detail"
        case progress
    }

    init(
        agentID: String,
        name: String,
        capabilities: [String] = [],
        connectedAt: String? = nil,
        lastActive: String? = nil,
        status: String? = nil,
        statusDetail: String? = nil,
        progress: Int? = nil
    ) {
        self.agentID = agentID
        self.name = name
        self.capabilities = capabilities
        self.connectedAt = connectedAt
        self.lastActive = lastActive
        self.status = status
        self.statusDetail = statusDetail
        self.progress = progress
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        agentID = try values.decode(String.self, forKey: .agentID)
        name = try values.decode(String.self, forKey: .name)
        capabilities = try values.decodeIfPresent([String].self, forKey: .capabilities) ?? []
        connectedAt = try values.decodeIfPresent(String.self, forKey: .connectedAt)
        lastActive = try values.decodeIfPresent(String.self, forKey: .lastActive)
        status = try values.decodeIfPresent(String.self, forKey: .status)
        statusDetail = try values.decodeIfPresent(String.self, forKey: .statusDetail)
        progress = try values.decodeIfPresent(Int.self, forKey: .progress)
    }

    func updating(status: String?, detail: String?, progress: Int?) -> AgentPresence {
        AgentPresence(
            agentID: agentID,
            name: name,
            capabilities: capabilities,
            connectedAt: connectedAt,
            lastActive: lastActive,
            status: status,
            statusDetail: detail,
            progress: progress
        )
    }
}

enum ConnectionStatus: Equatable {
    case disconnected
    case connecting
    case connected
    case failed(String)

    var label: String {
        switch self {
        case .disconnected: return "Offline"
        case .connecting: return "Connecting…"
        case .connected: return "Connected"
        case .failed: return "Connection failed"
        }
    }

    var failureMessage: String? {
        guard case let .failed(message) = self else { return nil }
        return message
    }

    var isConnected: Bool { self == .connected }
}

extension String {
    var cowchatTime: String {
        guard !isEmpty else { return "" }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        guard let date = fractional.date(from: self) ?? plain.date(from: self) else { return "" }
        return date.formatted(date: .omitted, time: .shortened)
    }
}
