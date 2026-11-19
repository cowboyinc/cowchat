import Foundation

public struct RoomLease: Decodable, Identifiable {
    public let name: String
    public let generation: Int64
    public let holderName: String
    public let taskState: String
    public let detail: String?
    public let progress: String?
    public let lastHeartbeatAt: TimeInterval
    public let lastProgressAt: TimeInterval
    public let overdueAt: TimeInterval
    public let expiresAt: TimeInterval
    public let releasedAt: TimeInterval?
    public var id: String { name }
    public enum CodingKeys: String, CodingKey {
        case name, generation, detail, progress
        case holderName = "holder_name", taskState = "task_state"
        case lastHeartbeatAt = "last_heartbeat_at", lastProgressAt = "last_progress_at"
        case overdueAt = "overdue_at", expiresAt = "expires_at", releasedAt = "released_at"
    }
    public func status(at date: Date) -> String {
        if releasedAt != nil { return "Released" }
        if date.timeIntervalSince1970 >= expiresAt { return "Expired" }
        if date.timeIntervalSince1970 >= overdueAt { return "Overdue" }
        switch taskState {
        case "waiting_for_human": return "Waiting for human"
        case "waiting_for_dependency": return "Waiting for dependency"
        default: return "Working"
        }
    }
}
