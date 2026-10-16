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
        VStack(spacing: 0) {
            CowchatSheetHeader(title: "Slack", subtitle: "Connect \(room.name) to your team’s conversation.") { dismiss() }
            divider
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    if let errorMessage { notice(errorMessage, error: true) }
                    if busy && !loaded {
                        ProgressView("Loading connection…")
                            .frame(maxWidth: .infinity).padding(.vertical, 50)
                    } else if loaded && status.connected {
                        connectedContent
                    } else if loaded {
                        setupContent
                    } else {
                        Text("Couldn’t load this room’s Slack connection. Try refreshing.")
                            .gallopText(.bodyM, color: SemanticColor.textSecondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(28)
                .disabled(busy)
            }
            divider
            HStack(spacing: 12) {
                if busy { ProgressView().controlSize(.small) }
                Button { perform { try await refresh() } } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .buttonStyle(CowchatPillButtonStyle())
                .disabled(busy)
                Spacer()
                if loaded && !status.connected && !workspaces.isEmpty {
                    Button("Connect this room", action: connectRoom)
                        .buttonStyle(CowchatPillButtonStyle(prominent: true))
                        .disabled(busy || workspaceID.isEmpty || channelID.isEmpty || roomLocked)
                } else {
                    Button("Done") { dismiss() }
                        .buttonStyle(CowchatPillButtonStyle(prominent: true))
                }
            }
            .padding(.horizontal, 28).padding(.vertical, 18)
        }
        .frame(width: 600, height: 660)
        .foregroundStyle(SemanticColor.textPrimary)
        .background(SemanticColor.surface600)
        .task { boundProfile = store.connectionProfile; perform { try await refresh() } }
        .onChange(of: store.connectionProfile) { _ in dismiss() }
        .onChange(of: workspaceID) { _ in
            channels = []; channelID = ""
            guard !workspaceID.isEmpty else { return }
            perform { try await loadChannels() }
        }
    }

    private var roomLocked: Bool {
        room.encrypted && !store.unlockedRoomIDs.contains(room.id)
    }

    private var divider: some View {
        Rectangle().fill(SemanticColor.borderDefault).frame(height: 0.5)
    }

    private var connectedContent: some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack(alignment: .top, spacing: 14) {
                Image(systemName: status.paused == true ? "pause.circle" : "checkmark.circle.fill")
                    .font(.system(size: 26))
                    .foregroundStyle(status.paused == true ? Color.orange : Color.green)
                VStack(alignment: .leading, spacing: 6) {
                    Text(status.paused == true ? "Connection paused" : "Connected to Slack")
                        .gallopText(.h5, color: SemanticColor.textPrimary)
                    Text("#\(status.channelName ?? "Slack")")
                        .gallopText(.bodyM, color: SemanticColor.textSecondary)
                    Text(status.paused == true ? "New messages wait until you resume." : "Messages flow both ways, even when this Mac is offline.")
                        .gallopText(.bodyS, color: SemanticColor.textTertiary)
                }
                .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .gallopCard(cornerRadius: 14)

            if let error = status.error { notice(error, error: true) }
            if let queued = status.queued, queued > 0 {
                notice("\(queued) Slack messages waiting to be delivered.")
            }
            section("In this conversation") {
                detail("arrow.left.arrow.right", title: "One shared conversation", text: "New messages in this room and its Slack channel are shared automatically.")
                detail("at", title: "Address your room bots", text: "Use a bot’s @handle in Slack to address it in this room.")
            }
            section("Manage connection") {
                Text("Paused messages are kept for seven days. Disconnecting stops delivery and discards pending messages.")
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
                HStack(spacing: 12) {
                    Button(status.paused == true ? "Resume connection" : "Pause connection") {
                        perform {
                            let _: Acknowledgement = try await api.request(["rooms", room.id, "pause"], method: "POST", body: ["paused": status.paused != true])
                            try await refresh()
                        }
                    }
                    .buttonStyle(CowchatPillButtonStyle())
                    Button("Disconnect", role: .destructive) {
                        perform {
                            let _: Acknowledgement = try await api.request(["rooms", room.id], method: "DELETE")
                            try await refresh()
                        }
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(SemanticColor.textError)
                }
            }
        }
    }

    private var setupContent: some View {
        VStack(alignment: .leading, spacing: 24) {
            SlackBridgeIllustration()
            section("1. Connect your workspace") {
                Text("Choose a Slack workspace to use with this global room.")
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
                if !workspaces.isEmpty {
                    Picker("Workspace", selection: $workspaceID) {
                        Text("Select a workspace").tag("")
                        ForEach(workspaces) { Text($0.name).tag($0.id) }
                    }
                    .controlSize(.large)
                }
                Button(workspaces.isEmpty ? "Connect a Slack workspace…" : "Add another workspace…") {
                    perform {
                        let authorization: Authorization = try await api.request(["connect"], method: "POST")
                        guard authorization.url.scheme == "https", authorization.url.host == "slack.com" else {
                            throw CowchatConnectionError.invalidResponse
                        }
                        NSWorkspace.shared.open(authorization.url)
                    }
                }
                .buttonStyle(CowchatPillButtonStyle())
            }
            if !workspaces.isEmpty {
                section("2. Choose a channel") {
                    Picker("Channel", selection: $channelID) {
                        Text("Select a channel").tag("")
                        ForEach(channels) { Text("#\($0.name)").tag($0.id) }
                    }
                    .controlSize(.large)
                    .disabled(workspaceID.isEmpty)
                    Text("Invite @Cowchat to the Slack channel first. You must belong to the channel too. Then refresh to see it here.")
                        .gallopText(.bodyS, color: SemanticColor.textTertiary)
                }
                notice("Connecting shares new room messages with Slack channel members and lets them address room bots. Earlier history is not exported. Other rooms stay disconnected.")
                if roomLocked { notice("Unlock this room before connecting Slack.") }
            }
        }
    }

    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
            content()
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func notice(_ text: String, error: Bool = false) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: error ? "exclamationmark.circle" : "info.circle")
            Text(text).fixedSize(horizontal: false, vertical: true)
        }
        .gallopText(.bodyS, color: error ? SemanticColor.textError : SemanticColor.textTertiary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(SemanticColor.surface400, in: RoundedRectangle(cornerRadius: 10))
    }

    private func detail(_ icon: String, title: String, text: String) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon).frame(width: 22).foregroundStyle(SemanticColor.iconSecondary)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).gallopText(.bodyM, color: SemanticColor.textPrimary)
                Text(text).gallopText(.bodyS, color: SemanticColor.textTertiary)
            }
        }
    }

    private func connectRoom() {
        perform {
            var body: [String: Any] = ["installation_id": workspaceID, "channel_id": channelID]
            guard boundProfile == store.connectionProfile else { throw CowchatConnectionError.notConnected }
            if room.encrypted { body["room_secret"] = try store.slackRoomSecret(for: room) }
            let _: Acknowledgement = try await api.request(["rooms", room.id], method: "POST", body: body)
            try await refresh()
        }
    }

    @MainActor private func loadChannels() async throws {
        let result: ChannelList = try await api.request(["workspaces", workspaceID, "channels"])
        channels = result.channels.sorted { $0.name < $1.name }
        if !channels.contains(where: { $0.id == channelID }) { channelID = "" }
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
            if !workspaceID.isEmpty { try await loadChannels() }
        }
    }
}
