import AppKit
import Foundation
import SwiftUI

struct RoomArtifactMetadata: Decodable {
    let title: String
    let revision: Int
    let publicID: String
    let updatedAt: String

    var updatedDate: Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: updatedAt) ?? ISO8601DateFormatter().date(from: updatedAt)
    }

    enum CodingKeys: String, CodingKey {
        case title, revision
        case publicID = "public_id", updatedAt = "updated_at"
    }
}

private final class ArtifactHTTPDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest,
                    completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

struct RoomArtifactAPI {
    private static let secureSession = URLSession(configuration: .ephemeral,
                                                  delegate: ArtifactHTTPDelegate(), delegateQueue: nil)
    let profile: ConnectionProfile
    var session: URLSession = secureSession

    static func baseURL(profile: ConnectionProfile) throws -> URL {
        guard profile.kind == .cowchatCloud, let endpoint = profile.endpointURL,
              endpoint.scheme == "wss", !profile.apiKey.isEmpty,
              var parts = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)
        else { throw CowchatConnectionError.transport("Artifacts are available for global rooms.") }
        parts.scheme = "https"
        var path = parts.path
        while path.hasSuffix("/") { path.removeLast() }
        if path.hasSuffix("/ws") { path.removeLast(3) }
        parts.path = path
        parts.query = nil
        parts.fragment = nil
        parts.user = nil
        parts.password = nil
        guard let url = parts.url else { throw CowchatConnectionError.invalidResponse }
        return url
    }

    static func shareURL(profile: ConnectionProfile, publicID: String) throws -> URL {
        guard let id = UUID(uuidString: publicID) else { throw CowchatConnectionError.invalidResponse }
        return try baseURL(profile: profile).appendingPathComponent("a")
            .appendingPathComponent(id.uuidString.lowercased())
    }

    func get(roomID: String) async throws -> RoomArtifactMetadata? {
        var url = try Self.baseURL(profile: profile)
        for segment in ["api", "rooms", roomID, "artifact"] { url.appendPathComponent(segment) }
        var request = URLRequest(url: url)
        request.timeoutInterval = 30
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.setValue(profile.apiKey, forHTTPHeaderField: "x-cowchat-key")
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw CowchatConnectionError.invalidResponse }
        if http.statusCode == 404 { return nil }
        guard http.statusCode == 200, data.count <= 1024 * 1024 else {
            throw CowchatConnectionError.server(message: "The artifact could not be loaded.", code: nil)
        }
        return try JSONDecoder().decode(RoomArtifactMetadata.self, from: data)
    }
}

struct RoomArtifactView: View {
    @EnvironmentObject private var store: ChatStore
    @Environment(\.dismiss) private var dismiss
    let room: Room
    @State private var artifact: RoomArtifactMetadata?
    @State private var boundProfile: ConnectionProfile?
    @State private var loading = true
    @State private var error: String?

    private var shareURL: URL? {
        guard let artifact, let boundProfile, boundProfile == store.connectionProfile else { return nil }
        return try? RoomArtifactAPI.shareURL(profile: boundProfile, publicID: artifact.publicID)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Room artifact").font(.title2.bold())
                    Text(room.name).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            if loading {
                ProgressView().frame(maxWidth: .infinity, minHeight: 120)
            } else if let error {
                Text(error).foregroundStyle(.red)
            } else if let artifact, let shareURL {
                VStack(alignment: .leading, spacing: 8) {
                    Text(artifact.title).font(.headline)
                    if let updated = artifact.updatedDate {
                        Text("Updated \(updated, style: .relative) ago")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    HStack {
                        Link("Open artifact", destination: shareURL).buttonStyle(.borderedProminent)
                        Button("Copy link") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(shareURL.absoluteString, forType: .string)
                        }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text("No artifact published.").foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 120)
            }
        }
        .padding(24).frame(width: 520).frame(minHeight: 230)
        .task(id: store.connectionProfile) {
            let profile = store.connectionProfile
            boundProfile = profile
            artifact = nil
            loading = true
            error = nil
            do {
                let result = try await RoomArtifactAPI(profile: profile).get(roomID: room.id)
                guard !Task.isCancelled, profile == store.connectionProfile else { return }
                artifact = result
                loading = false
            } catch is CancellationError {
                return
            } catch {
                guard !Task.isCancelled, profile == store.connectionProfile else { return }
                self.error = "The artifact could not be loaded."
                loading = false
            }
        }
    }
}
