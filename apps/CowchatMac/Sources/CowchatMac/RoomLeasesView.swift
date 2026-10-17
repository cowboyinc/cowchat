import Foundation
import SwiftUI

struct RoomLease: Decodable, Identifiable {
    let name: String
    let generation: Int64
    let holderName: String
    let taskState: String
    let detail: String?
    let progress: String?
    let lastHeartbeatAt: TimeInterval
    let lastProgressAt: TimeInterval
    let overdueAt: TimeInterval
    let expiresAt: TimeInterval
    let releasedAt: TimeInterval?
    var id: String { name }
    enum CodingKeys: String, CodingKey {
        case name, generation, detail, progress
        case holderName = "holder_name", taskState = "task_state"
        case lastHeartbeatAt = "last_heartbeat_at", lastProgressAt = "last_progress_at"
        case overdueAt = "overdue_at", expiresAt = "expires_at", releasedAt = "released_at"
    }
    func status(at date: Date) -> String {
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

struct RoomLeasesView: View {
    @EnvironmentObject private var store: ChatStore
    @Environment(\.dismiss) private var dismiss
    let room: Room
    @State private var leases: [RoomLease] = []
    @State private var error: String?
    @State private var loading = true

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Room leases").font(.title2.bold())
                    Text(room.name).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            Text("Heartbeats every minute · Overdue after 5 minutes · Expire after 7 minutes")
                .font(.caption).foregroundStyle(.secondary)
            if let error {
                Text(error).foregroundStyle(.red)
                Text("If this server predates room leases, update it first. Status could not be refreshed.")
                    .font(.caption).foregroundStyle(.secondary)
            } else if loading {
                ProgressView()
            } else if leases.isEmpty {
                Text("No leases in this room.").foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 150)
            }
            if error == nil {
                ScrollView {
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        VStack(spacing: 10) {
                            ForEach(leases) { lease in
                                RoomLeaseRow(lease: lease, date: context.date)
                            }
                        }
                    }
                }
            }
            Text("A heartbeat proves the host is alive. Progress and prompt state must be reported by the host.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding(24).frame(width: 640, height: 480)
        .task(id: store.connectionProfile) {
            leases = []; loading = true
            while !Task.isCancelled {
                do {
                    leases = try await store.roomLeases(roomID: room.id)
                    error = nil; loading = false
                } catch is CancellationError { return }
                catch { self.error = error.localizedDescription; loading = false }
                do { try await Task.sleep(nanoseconds: 60_000_000_000) }
                catch { return }
            }
        }
    }
}

struct RoomLeaseRow: View {
    let lease: RoomLease
    let date: Date
    var body: some View {
                                VStack(alignment: .leading, spacing: 6) {
                                    HStack {
                                        Text(lease.name).font(.headline)
                                        Spacer()
                                        Text(lease.status(at: date))
                                            .foregroundStyle(statusColor(lease, at: date))
                                    }
                                    Text("\(lease.holderName) · Generation \(lease.generation)").font(.subheadline)
                                    if let detail = lease.detail, !detail.isEmpty { Text(detail) }
                                    if let progress = lease.progress, !progress.isEmpty { Text(progress).foregroundStyle(.secondary) }
                                    HStack {
                                        Text("Heartbeat \(age(since: lease.lastHeartbeatAt)) ago")
                                        Spacer()
                                        Text("Progress \(age(since: lease.lastProgressAt)) ago")
                                    }.font(.caption).foregroundStyle(.secondary)
                                }
                                .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                                .background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
    }
    private func age(since timestamp: TimeInterval) -> String {
        let seconds = max(0, Int(date.timeIntervalSince1970 - timestamp))
        if seconds < 60 { return "\(seconds)s" }
        if seconds < 3600 { return "\(seconds / 60)m" }
        return "\(seconds / 3600)h \(seconds % 3600 / 60)m"
    }
    private func statusColor(_ lease: RoomLease, at date: Date) -> Color {
        switch lease.status(at: date) {
        case "Overdue": return .orange
        case "Expired": return .red
        case "Released": return .secondary
        default: return .primary
        }
    }
}
