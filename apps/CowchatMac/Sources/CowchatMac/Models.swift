import Foundation

struct Room: Codable, Identifiable, Hashable {
    let roomID: String
    let name: String
    let description: String?
    let parentID: String?
    let ephemeral: Bool
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
        case ephemeral
        case createdAt = "created_at"
        case createdBy = "created_by"
        case visibility
        case lastActivity = "last_activity"
        case memberCount = "member_count"
        case encrypted
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        roomID = try values.decode(String.self, forKey: .roomID)
        name = try values.decode(String.self, forKey: .name)
        description = try values.decodeIfPresent(String.self, forKey: .description)
        parentID = try values.decodeIfPresent(String.self, forKey: .parentID)
        ephemeral = try values.decodeIfPresent(Bool.self, forKey: .ephemeral) ?? false
        createdAt = try values.decodeIfPresent(String.self, forKey: .createdAt) ?? ""
        createdBy = try values.decodeIfPresent(String.self, forKey: .createdBy)
        visibility = try values.decodeIfPresent(String.self, forKey: .visibility) ?? "private"
        lastActivity = try values.decodeIfPresent(String.self, forKey: .lastActivity)
        memberCount = try values.decodeIfPresent(Int.self, forKey: .memberCount)
        encrypted = try values.decodeIfPresent(Bool.self, forKey: .encrypted) ?? false
    }
}

struct ChatMessage: Codable, Identifiable, Equatable {
    let messageID: String
    let roomID: String
    let agentID: String
    let agentName: String
    let content: String
    let replyToMessage: String?
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
        timestamp = try values.decodeIfPresent(String.self, forKey: .timestamp) ?? ""
        seq = try values.decodeIfPresent(Int.self, forKey: .seq) ?? 0
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
