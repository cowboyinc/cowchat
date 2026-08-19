import Foundation
import Network

@MainActor
protocol CowchatWebSocketTaskProtocol: AnyObject {
    func resume()
    func cancel(
        with closeCode: URLSessionWebSocketTask.CloseCode,
        reason: Data?
    )
    func send(_ message: URLSessionWebSocketTask.Message) async throws
    func receive() async throws -> URLSessionWebSocketTask.Message
}

extension URLSessionWebSocketTask: CowchatWebSocketTaskProtocol {}

enum CowchatLocalAPIKey {
    static var defaultURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".cowchat", isDirectory: true)
            .appendingPathComponent("auth.key", isDirectory: false)
    }

    static func load(from url: URL = defaultURL) -> String {
        guard let key = try? String(contentsOf: url, encoding: .utf8) else { return "" }
        return key.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

enum CowchatConnectionError: LocalizedError {
    case notConnected
    case invalidResponse
    case server(message: String, code: String?)
    case transport(String)
    case timeout

    var errorDescription: String? {
        switch self {
        case .notConnected:
            return "Cowchat is not connected."
        case .invalidResponse:
            return "Cowchat returned an invalid response."
        case .server(let message, _):
            return message
        case .transport(let message):
            return message
        case .timeout:
            return "Cowchat did not respond in time."
        }
    }
}

@MainActor
protocol CowchatConnectionProtocol: AnyObject {
    var onEvent: ((String, [String: Any]) -> Void)? { get set }
    var onStatusChange: ((ConnectionStatus) -> Void)? { get set }
    func reconfigure(profile: ConnectionProfile)
    func connect() async throws
    func register(name: String, agentID: String) async throws -> CowchatRegistration
    func listRooms() async throws -> [Room]
    func listAgents(roomID: String) async throws -> [AgentPresence]
    func createRoom(
        name: String,
        description: String?,
        parentID: String?,
        isPublic: Bool
    ) async throws -> Room
    func rename(roomID: String, name: String) async throws -> Room
    func destroy(roomID: String) async throws
    /// Mints a room invite token (`cinv_…`). Single-use invites die on
    /// first redemption.
    func createInvite(roomID: String, singleUse: Bool) async throws -> String
    func listInvites(roomID: String) async throws -> [RoomInvite]
    func revokeInvite(inviteID: String) async throws
    func join(roomID: String) async throws
    func leave(roomID: String) async throws
    func history(
        roomID: String,
        limit: Int,
        before: String?
    ) async throws -> [ChatMessage]
    func send(roomID: String, content: String) async throws -> ChatMessage
    func disconnect()
}

extension CowchatConnectionProtocol {
    func reconfigure(profile _: ConnectionProfile) {}

    func history(roomID: String, limit: Int) async throws -> [ChatMessage] {
        try await history(roomID: roomID, limit: limit, before: nil)
    }
}

@MainActor
final class CowchatConnection: CowchatConnectionProtocol {
    typealias Payload = [String: Any]
    private static let maximumFrameBytes = 1_048_576

    var onEvent: ((String, Payload) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?

    private let queue = DispatchQueue(label: "inc.cowboy.cowchat.network")
    private let makeWebSocketTask: (URL) -> any CowchatWebSocketTaskProtocol
    private(set) var profile: ConnectionProfile
    private var connection: NWConnection?
    private var webSocketTask: (any CowchatWebSocketTaskProtocol)?
    private var webSocketReceiveTask: Task<Void, Never>?
    private var webSocketTransportID: UUID?
    private var buffer = Data()
    private var readyContinuation: CheckedContinuation<Void, Error>?
    private var pending: [String: CheckedContinuation<Payload, Error>] = [:]
    private var connectAttempt: UUID?

    init(host: String = "127.0.0.1", port: UInt16 = 9229) {
        profile = .local(host: host, port: port)
        makeWebSocketTask = { URLSession.shared.webSocketTask(with: $0) }
    }

    init(profile: ConnectionProfile, webSocketSession: URLSession = .shared) {
        self.profile = profile
        makeWebSocketTask = { webSocketSession.webSocketTask(with: $0) }
    }

    init(
        profile: ConnectionProfile,
        webSocketTaskFactory: @escaping (URL) -> any CowchatWebSocketTaskProtocol
    ) {
        self.profile = profile
        makeWebSocketTask = webSocketTaskFactory
    }

    func reconfigure(profile: ConnectionProfile) {
        guard self.profile != profile else { return }
        disconnect()
        self.profile = profile
    }

    func connect() async throws {
        disconnect()
        onStatusChange?(.connecting)

        switch profile.kind {
        case .local:
            try await connectTCP()
        case .cowchatCloud:
            try await connectWebSocket()
        }
    }

    private func connectTCP() async throws {
        guard let host = profile.localHost,
              let portValue = profile.localPort,
              let port = NWEndpoint.Port(rawValue: portValue) else {
            throw CowchatConnectionError.transport("The local Cowchat connection is invalid.")
        }

        let attempt = UUID()
        connectAttempt = attempt
        let connection = NWConnection(host: NWEndpoint.Host(host), port: port, using: .tcp)
        self.connection = connection
        connection.stateUpdateHandler = { [weak self, weak connection] state in
            guard let connection else { return }
            Task { @MainActor in self?.handle(state, from: connection) }
        }
        receiveNext(on: connection)

        try await withCheckedThrowingContinuation { continuation in
            readyContinuation = continuation
            connection.start(queue: queue)
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                guard !Task.isCancelled else { return }
                self?.timeoutConnect(attempt: attempt)
            }
        }
    }

    private func connectWebSocket() async throws {
        guard let endpointURL = profile.endpointURL else {
            throw CowchatConnectionError.transport("The Cowchat Cloud connection is invalid.")
        }

        let transportID = UUID()
        let task = makeWebSocketTask(endpointURL)
        webSocketTask = task
        webSocketTransportID = transportID
        startWebSocketReceiveLoop(task: task, transportID: transportID)
        task.resume()
    }

    func register(name: String, agentID: String) async throws -> CowchatRegistration {
        let registrationKey = profile.kind == .local
            ? CowchatLocalAPIKey.load()
            : profile.apiKey
        let payload = try await request(type: "register", payload: [
            "key": registrationKey,
            "name": name,
            "agent_id": agentID,
            "reconnect": true,
            "capabilities": ["macos-ui"],
            "protocol_version": CowchatWireProtocol.currentVersion,
        ])
        guard let agentID = payload["agent_id"] as? String else {
            throw CowchatConnectionError.invalidResponse
        }
        return CowchatRegistration(
            agentID: agentID,
            restoredRoomIDs: Set(payload["rooms"] as? [String] ?? [])
        )
    }

    func listRooms() async throws -> [Room] {
        let payload = try await request(type: "list_rooms", payload: [:])
        return try decode([Room].self, from: payload["rooms"] ?? [])
    }

    func listAgents(roomID: String) async throws -> [AgentPresence] {
        let payload = try await request(type: "list_agents", payload: ["room_id": roomID])
        return try decode([AgentPresence].self, from: payload["agents"] ?? [])
    }

    func createRoom(
        name: String,
        description: String?,
        parentID: String?,
        isPublic: Bool
    ) async throws -> Room {
        var payload: Payload = [
            "name": name,
            "public": isPublic,
            "encrypted": false,
        ]
        if let description, !description.isEmpty { payload["description"] = description }
        if let parentID, !parentID.isEmpty { payload["parent_id"] = parentID }
        let response = try await request(type: "create_room", payload: payload)
        return try decode(Room.self, from: response)
    }

    func destroy(roomID: String) async throws {
        _ = try await request(type: "destroy_room", payload: ["room_id": roomID])
    }

    func createInvite(roomID: String, singleUse: Bool) async throws -> String {
        let response = try await request(type: "create_invite", payload: [
            "room_id": roomID,
            "single_use": singleUse,
        ])
        guard let token = response["token"] as? String, !token.isEmpty else {
            throw CowchatConnectionError.invalidResponse
        }
        return token
    }

    func listInvites(roomID: String) async throws -> [RoomInvite] {
        let response = try await request(type: "list_invites", payload: ["room_id": roomID])
        return try decode([RoomInvite].self, from: response["invites"] ?? [])
    }

    func revokeInvite(inviteID: String) async throws {
        _ = try await request(type: "revoke_invite", payload: ["invite_id": inviteID])
    }

    func rename(roomID: String, name: String) async throws -> Room {
        let response = try await request(type: "rename_room", payload: [
            "room_id": roomID,
            "name": name,
        ])
        return try decode(Room.self, from: response)
    }

    func join(roomID: String) async throws {
        _ = try await request(type: "join_room", payload: ["room_id": roomID])
    }

    func leave(roomID: String) async throws {
        _ = try await request(type: "leave_room", payload: ["room_id": roomID])
    }

    func history(
        roomID: String,
        limit: Int = 100,
        before: String? = nil
    ) async throws -> [ChatMessage] {
        var requestPayload: Payload = [
            "room_id": roomID,
            "limit": limit,
        ]
        if let before { requestPayload["before"] = before }
        let payload = try await request(type: "get_history", payload: requestPayload)
        return try decode([ChatMessage].self, from: payload["messages"] ?? [])
    }

    func send(roomID: String, content: String) async throws -> ChatMessage {
        let payload = try await request(type: "send_message", payload: [
            "room_id": roomID,
            "content": content,
            "metadata": [:],
            "mentions": [],
        ])
        return try decode(ChatMessage.self, from: payload)
    }

    func disconnect() {
        let oldConnection = connection
        let oldWebSocket = webSocketTask
        let oldWebSocketReceiveTask = webSocketReceiveTask
        connection = nil
        webSocketTask = nil
        webSocketReceiveTask = nil
        webSocketTransportID = nil
        oldConnection?.stateUpdateHandler = nil
        oldConnection?.cancel()
        oldWebSocketReceiveTask?.cancel()
        oldWebSocket?.cancel(with: .goingAway, reason: nil)
        buffer.removeAll(keepingCapacity: true)
        connectAttempt = nil
        readyContinuation?.resume(throwing: CowchatConnectionError.notConnected)
        readyContinuation = nil
        failPending(with: CowchatConnectionError.notConnected)
    }

    private func request(
        type: String,
        payload: Payload,
        timeoutNanoseconds: UInt64 = 10_000_000_000
    ) async throws -> Payload {
        guard connection != nil || webSocketTask != nil else {
            throw CowchatConnectionError.notConnected
        }
        let id = UUID().uuidString
        let frame: Payload = ["id": id, "type": type, "payload": payload]
        guard JSONSerialization.isValidJSONObject(frame),
              let data = try? JSONSerialization.data(withJSONObject: frame) else {
            throw CowchatConnectionError.invalidResponse
        }
        guard data.count <= Self.maximumFrameBytes else {
            throw CowchatConnectionError.transport("Cowchat frames cannot exceed 1 MiB.")
        }

        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            return try await withCheckedThrowingContinuation { continuation in
                if Task.isCancelled {
                    continuation.resume(throwing: CancellationError())
                    return
                }
                pending[id] = continuation
                sendRequestFrame(data, id: id)
                Task { [weak self] in
                    try? await Task.sleep(nanoseconds: timeoutNanoseconds)
                    guard !Task.isCancelled else { return }
                    self?.timeoutRequest(id: id)
                }
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.finishRequest(id: id, result: .failure(CancellationError()))
            }
        }
    }

    private func sendRequestFrame(_ data: Data, id: String) {
        if let connection {
            var terminated = data
            terminated.append(0x0A)
            connection.send(content: terminated, completion: .contentProcessed { [weak self] error in
                guard let error else { return }
                Task { @MainActor in
                    self?.finishRequest(
                        id: id,
                        result: .failure(Self.transportError(error))
                    )
                }
            })
            return
        }

        guard let webSocketTask,
              let text = String(data: data, encoding: .utf8) else {
            finishRequest(id: id, result: .failure(CowchatConnectionError.notConnected))
            return
        }
        Task { [weak self, webSocketTask] in
            do {
                try await webSocketTask.send(.string(text))
            } catch {
                self?.finishRequest(
                    id: id,
                    result: .failure(Self.transportError(error))
                )
            }
        }
    }

    private func receiveNext(on connection: NWConnection) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65_536) { [weak self, weak connection] data, _, isComplete, error in
            Task { @MainActor in
                guard let self, let connection, connection === self.connection else { return }
                if let data { self.consume(data) }
                if let error {
                    self.failConnection(Self.transportError(error))
                } else if isComplete {
                    self.failConnection(CowchatConnectionError.notConnected)
                } else {
                    self.receiveNext(on: connection)
                }
            }
        }
    }

    private func startWebSocketReceiveLoop(
        task: any CowchatWebSocketTaskProtocol,
        transportID: UUID
    ) {
        webSocketReceiveTask = Task { [weak self, task] in
            do {
                while !Task.isCancelled {
                    let message = try await task.receive()
                    guard !Task.isCancelled else { return }
                    self?.consumeWebSocketMessage(
                        message,
                        from: task,
                        transportID: transportID
                    )
                }
            } catch {
                guard !Task.isCancelled,
                      let self,
                      self.webSocketTask === task,
                      self.webSocketTransportID == transportID else { return }
                self.failConnection(Self.transportError(error))
            }
        }
    }

    private func consumeWebSocketMessage(
        _ message: URLSessionWebSocketTask.Message,
        from task: any CowchatWebSocketTaskProtocol,
        transportID: UUID
    ) {
        guard webSocketTask === task, webSocketTransportID == transportID else { return }
        let data: Data
        switch message {
        case .string(let text):
            data = Data(text.utf8)
        case .data:
            failConnection(CowchatConnectionError.transport("Cowchat Cloud sent a non-text frame."))
            return
        @unknown default:
            failConnection(CowchatConnectionError.invalidResponse)
            return
        }
        guard data.count <= Self.maximumFrameBytes else {
            failConnection(CowchatConnectionError.transport("Cowchat sent a frame larger than 1 MiB."))
            return
        }
        if data.last == 0x0A {
            consume(data)
        } else {
            var terminated = data
            terminated.append(0x0A)
            consume(terminated)
        }
    }

    private func consume(_ data: Data) {
        buffer.append(data)
        while let newline = buffer.firstIndex(of: 0x0A) {
            let line = buffer[..<newline]
            buffer.removeSubrange(...newline)
            guard line.count <= Self.maximumFrameBytes else {
                failConnection(CowchatConnectionError.transport("Cowchat sent a frame larger than 1 MiB."))
                return
            }
            guard !line.isEmpty,
                  let object = try? JSONSerialization.jsonObject(with: Data(line)) as? Payload,
                  let type = object["type"] as? String else { continue }
            let payload = object["payload"] as? Payload ?? [:]

            if type == "ping", let id = object["id"] as? String {
                sendFrame(["id": UUID().uuidString, "type": "pong", "reply_to": id, "payload": [:]])
            } else if let replyTo = object["reply_to"] as? String {
                if type == "error" {
                    let message = payload["message"] as? String ?? "Unknown server error"
                    finishRequest(
                        id: replyTo,
                        result: .failure(CowchatConnectionError.server(
                            message: message,
                            code: payload["code"] as? String
                        ))
                    )
                } else {
                    finishRequest(id: replyTo, result: .success(payload))
                }
            } else {
                onEvent?(type, payload)
            }
        }
        guard buffer.count <= Self.maximumFrameBytes else {
            failConnection(CowchatConnectionError.transport("Cowchat sent a frame larger than 1 MiB."))
            return
        }
    }

    private func handle(_ state: NWConnection.State, from source: NWConnection) {
        guard source === connection else { return }
        switch state {
        case .ready:
            readyContinuation?.resume()
            readyContinuation = nil
            connectAttempt = nil
        case .waiting(let error):
            failConnection(Self.transportError(error))
        case .failed(let error):
            failConnection(Self.transportError(error))
        case .cancelled:
            failConnection(CowchatConnectionError.notConnected)
        default:
            break
        }
    }

    private func failConnection(_ error: Error) {
        readyContinuation?.resume(throwing: error)
        readyContinuation = nil
        failPending(with: error)
        let failedConnection = connection
        let failedWebSocket = webSocketTask
        let failedWebSocketReceiveTask = webSocketReceiveTask
        connection = nil
        webSocketTask = nil
        webSocketReceiveTask = nil
        webSocketTransportID = nil
        connectAttempt = nil
        failedConnection?.stateUpdateHandler = nil
        failedConnection?.cancel()
        failedWebSocketReceiveTask?.cancel()
        failedWebSocket?.cancel(with: .goingAway, reason: nil)
        let message = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
        onStatusChange?(.failed(message))
    }

    private func failPending(with error: Error) {
        let continuations = pending.values
        pending.removeAll()
        continuations.forEach { $0.resume(throwing: error) }
    }

    private func finishRequest(id: String, result: Result<Payload, Error>) {
        guard let continuation = pending.removeValue(forKey: id) else { return }
        continuation.resume(with: result)
    }

    private func timeoutRequest(id: String) {
        finishRequest(id: id, result: .failure(CowchatConnectionError.timeout))
    }

    private func timeoutConnect(attempt: UUID) {
        guard connectAttempt == attempt, readyContinuation != nil else { return }
        failConnection(CowchatConnectionError.timeout)
    }

    private func sendFrame(_ frame: Payload) {
        guard JSONSerialization.isValidJSONObject(frame),
              let data = try? JSONSerialization.data(withJSONObject: frame),
              data.count <= Self.maximumFrameBytes else { return }
        if let connection {
            var terminated = data
            terminated.append(0x0A)
            connection.send(content: terminated, completion: .idempotent)
        } else if let webSocketTask,
                  let text = String(data: data, encoding: .utf8) {
            Task { [weak self, webSocketTask] in
                do {
                    try await webSocketTask.send(.string(text))
                } catch {
                    guard let self, self.webSocketTask === webSocketTask else { return }
                    self.failConnection(Self.transportError(error))
                }
            }
        }
    }

    private func decode<T: Decodable>(_ type: T.Type, from object: Any) throws -> T {
        guard JSONSerialization.isValidJSONObject(object) else {
            throw CowchatConnectionError.invalidResponse
        }
        let data = try JSONSerialization.data(withJSONObject: object)
        return try JSONDecoder().decode(type, from: data)
    }

    private static func transportError(_ error: Error) -> CowchatConnectionError {
        if let connectionError = error as? CowchatConnectionError {
            return connectionError
        }
        return .transport(error.localizedDescription)
    }
}
