import AppKit
import Foundation
import SwiftUI

struct SlackWorkspace: Decodable, Identifiable, Equatable {
    let id: String
    let name: String
}
struct SlackChannel: Decodable, Identifiable, Equatable {
    let id: String
    let name: String
}
struct SlackRoomStatus: Decodable {
    let connected: Bool
    var channelName: String?
    var paused: Bool?
    var error: String?
    var queued: Int?
    enum CodingKeys: String, CodingKey {
        case connected, paused, error, queued
        case channelName = "channel_name"
    }
}

/// The Mac configures the bridge; delivery is owned by the global server.
private final class SlackHTTPDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil) // Never forward room secrets or bearer keys to a redirect.
    }
}

struct SlackAPI {
    private static let secureSession = URLSession(configuration: .ephemeral, delegate: SlackHTTPDelegate(), delegateQueue: nil)
    let profile: ConnectionProfile
    var session: URLSession = secureSession

    static func endpoint(profile: ConnectionProfile, path: [String]) throws -> URL {
        guard profile.kind == .cowchatCloud, let endpoint = profile.endpointURL,
              endpoint.scheme == "wss", !profile.apiKey.isEmpty,
              var components = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)
        else { throw CowchatConnectionError.transport("Slack is available only for global rooms.") }
        components.scheme = "https"
        components.path = "/integrations/slack"
        components.query = nil
        components.fragment = nil
        components.user = nil
        components.password = nil
        guard var url = components.url else { throw CowchatConnectionError.invalidResponse }
        for segment in path { url.appendPathComponent(segment) }
        return url
    }

    func request<T: Decodable>(_ path: [String], method: String = "GET", body: [String: Any]? = nil) async throws -> T {
        var request = URLRequest(url: try Self.endpoint(profile: profile, path: path))
        request.httpMethod = method
        request.timeoutInterval = 30
        request.setValue(profile.apiKey, forHTTPHeaderField: "x-cowchat-key")
        if let body {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw CowchatConnectionError.invalidResponse }
        guard (200..<300).contains(http.statusCode) else {
            let detail = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String
            throw CowchatConnectionError.server(message: detail ?? "The server could not complete the Slack operation.", code: nil)
        }
        return try JSONDecoder().decode(T.self, from: data)
    }
}

struct SlackRoomView: View {
    @EnvironmentObject private var store: ChatStore
    @Environment(\.dismiss) private var dismiss
    let room: Room
    var session: URLSession? = nil
    @State private var boundProfile: ConnectionProfile?
    @State private var status = SlackRoomStatus(connected: false)
    @State private var workspaces: [SlackWorkspace] = []
    @State private var channels: [SlackChannel] = []
    @State private var workspaceID = ""
    @State private var channelID = ""
    @State private var busy = true
    @State private var loaded = false
    @State private var errorMessage: String?

    private var api: SlackAPI {
        var api = SlackAPI(profile: boundProfile ?? store.connectionProfile)
        if let session { api.session = session }
        return api
    }
    private struct WorkspaceList: Decodable { let workspaces: [SlackWorkspace] }
    private struct ChannelList: Decodable { let channels: [SlackChannel] }
    private struct Authorization: Decodable { let url: URL }
    private struct Acknowledgement: Decodable {}

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Slack · \(room.name)").font(.headline)
                Spacer()
                Button("Done") { dismiss() }
            }
            if busy { ProgressView().controlSize(.small) }
            if let errorMessage { Text(errorMessage).foregroundStyle(.red).font(.callout) }
            if !loaded && !busy { Button("Retry") { perform { try await refresh() } } }
            if loaded && status.connected {
                Label("Connected to #\(status.channelName ?? "Slack")", systemImage: "link")
                Text(status.paused == true ? "Messages are queued while this connection is paused." : "The global server relays messages even when this Mac is offline.")
                    .font(.callout).foregroundStyle(.secondary)
                if let error = status.error { Text(error).foregroundStyle(.orange).font(.callout) }
                if let queued = status.queued, queued > 0 { Text("\(queued) Slack messages waiting").font(.caption) }
                Text("Disconnecting stops delivery and discards pending messages. Paused messages are retained for seven days.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button(status.paused == true ? "Resume" : "Pause") {
                        perform {
                            let _: Acknowledgement = try await api.request(["rooms", room.id, "pause"], method: "POST", body: ["paused": status.paused != true])
                            try await refresh()
                        }
                    }
                    Button("Disconnect", role: .destructive) {
                        perform {
                            let _: Acknowledgement = try await api.request(["rooms", room.id], method: "DELETE")
                            try await refresh()
                        }
                    }
                    Spacer()
                    Button("Refresh") { perform { try await refresh() } }
                }
            } else if loaded {
                Text("Connect only this global room to one Slack channel. Other rooms stay disconnected.").font(.callout)
                HStack {
                    Button("Connect a Slack workspace…") {
                        perform {
                            let authorization: Authorization = try await api.request(["connect"], method: "POST")
                            guard authorization.url.scheme == "https", authorization.url.host == "slack.com" else {
                                throw CowchatConnectionError.invalidResponse
                            }
                            NSWorkspace.shared.open(authorization.url)
                        }
                    }
                    Button("Refresh") { perform { try await refresh() } }
                }
                if !workspaces.isEmpty {
                    Picker("Workspace", selection: $workspaceID) {
                        Text("Select a workspace").tag("")
                        ForEach(workspaces) { Text($0.name).tag($0.id) }
                    }
                    Picker("Channel", selection: $channelID) {
                        Text("Select a channel").tag("")
                        ForEach(channels) { Text("#\($0.name)").tag($0.id) }
                    }
                    Text("Invite the Cowchat Slack app to the channel first. The Slack user who connected the workspace must belong to it.")
                        .font(.caption).foregroundStyle(.secondary)
                    Text("Connecting allows Slack channel members and the server bridge to read new room messages and address room bots. Earlier room history is not exported.")
                        .font(.callout).fixedSize(horizontal: false, vertical: true)
                    Button("Connect this room") {
                        perform {
                            var body: [String: Any] = ["installation_id": workspaceID, "channel_id": channelID]
                            guard boundProfile == store.connectionProfile else { throw CowchatConnectionError.notConnected }
                            if room.encrypted { body["room_secret"] = try store.slackRoomSecret(for: room) }
                            let _: Acknowledgement = try await api.request(["rooms", room.id], method: "POST", body: body)
                            try await refresh()
                        }
                    }
                    .disabled(workspaceID.isEmpty || channelID.isEmpty || (room.encrypted && !store.unlockedRoomIDs.contains(room.id)))
                    if room.encrypted && !store.unlockedRoomIDs.contains(room.id) {
                        Text("Unlock the room before connecting Slack.").font(.caption)
                    }
                }
            }
        }
        .padding(24).frame(width: 500)
        .foregroundStyle(SemanticColor.textPrimary)
        .background(SemanticColor.surface500)
        .disabled(busy)
        .task { boundProfile = store.connectionProfile; perform { try await refresh() } }
        .onChange(of: store.connectionProfile) { _ in dismiss() }
        .onChange(of: workspaceID) { _ in
            channels = []; channelID = ""
            guard !workspaceID.isEmpty else { return }
            perform {
                let result: ChannelList = try await api.request(["workspaces", workspaceID, "channels"])
                channels = result.channels.sorted { $0.name < $1.name }
            }
        }
    }

    @MainActor private func perform(_ operation: @escaping @MainActor () async throws -> Void) {
        busy = true; errorMessage = nil
        Task {
            defer { busy = false }
            do { try await operation() }
            catch { errorMessage = error.localizedDescription }
        }
    }
    @MainActor private func refresh() async throws {
        status = try await api.request(["rooms", room.id])
        loaded = true
        if !status.connected {
            let result: WorkspaceList = try await api.request(["workspaces"])
            workspaces = result.workspaces.sorted { $0.name < $1.name }
        }
    }
}
