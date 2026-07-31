import Foundation
import Network

enum CowchatConnectionError: LocalizedError {
    case missingAPIKey
    case notConnected
    case invalidResponse
    case server(String)
    case transport(String)
    case timeout

    var errorDescription: String? {
        switch self {
        case .missingAPIKey:
            return "No local API key was found. Start cowchat-server once to create ~/.cowchat/auth.key."
        case .notConnected:
            return "Cowchat is not connected."
        case .invalidResponse:
            return "Cowchat returned an invalid response."
        case .server(let message), .transport(let message):
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
    func connect() async throws
    func register(name: String, agentID: String) async throws -> String
    func listRooms() async throws -> [Room]
    func listAgents(roomID: String) async throws -> [AgentPresence]
    func createRoom(name: String, description: String?, ephemeral: Bool, isPublic: Bool) async throws -> Room
    func join(roomID: String) async throws
    func leave(roomID: String) async throws
    func history(roomID: String, limit: Int) async throws -> [ChatMessage]
    func send(roomID: String, content: String) async throws -> ChatMessage
    func disconnect()
}

@MainActor
final class CowchatConnection: CowchatConnectionProtocol {
    typealias Payload = [String: Any]
    private static let maximumFrameBytes = 1_048_576

    var onEvent: ((String, Payload) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?

    private let queue = DispatchQueue(label: "inc.cowboy.cowchat.network")
    private var connection: NWConnection?
    private var buffer = Data()
    private var readyContinuation: CheckedContinuation<Void, Error>?
    private var pending: [String: CheckedContinuation<Payload, Error>] = [:]
    private var connectAttempt: UUID?

    func connect() async throws {
        disconnect()
        onStatusChange?(.connecting)

        let attempt = UUID()
        connectAttempt = attempt
        let connection = NWConnection(host: "127.0.0.1", port: 9229, using: .tcp)
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

    func register(name: String, agentID: String) async throws -> String {
        let keyURL = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".cowchat/auth.key")
        guard let key = try? String(contentsOf: keyURL, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines), !key.isEmpty else {
            throw CowchatConnectionError.missingAPIKey
        }

        let payload = try await request(type: "register", payload: [
            "key": key,
            "name": name,
            "agent_id": agentID,
            "reconnect": true,
            "capabilities": ["macos-ui"],
            "protocol_version": 1,
        ])
        guard let agentID = payload["agent_id"] as? String else {
            throw CowchatConnectionError.invalidResponse
        }
        return agentID
    }

    func listRooms() async throws -> [Room] {
        let payload = try await request(type: "list_rooms", payload: [:])
        return try decode([Room].self, from: payload["rooms"] ?? [])
    }

    func listAgents(roomID: String) async throws -> [AgentPresence] {
        let payload = try await request(type: "list_agents", payload: ["room_id": roomID])
        return try decode([AgentPresence].self, from: payload["agents"] ?? [])
    }

    func createRoom(name: String, description: String?, ephemeral: Bool, isPublic: Bool) async throws -> Room {
        var payload: Payload = [
            "name": name,
            "ephemeral": ephemeral,
            "public": isPublic,
            "encrypted": false,
        ]
        if let description, !description.isEmpty { payload["description"] = description }
        let response = try await request(type: "create_room", payload: payload)
        return try decode(Room.self, from: response)
    }

    func join(roomID: String) async throws {
        _ = try await request(type: "join_room", payload: ["room_id": roomID])
    }

    func leave(roomID: String) async throws {
        _ = try await request(type: "leave_room", payload: ["room_id": roomID])
    }

    func history(roomID: String, limit: Int = 100) async throws -> [ChatMessage] {
        let payload = try await request(type: "get_history", payload: [
            "room_id": roomID,
            "limit": limit,
        ])
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
        connection = nil
        oldConnection?.stateUpdateHandler = nil
        oldConnection?.cancel()
        buffer.removeAll(keepingCapacity: true)
        connectAttempt = nil
        readyContinuation?.resume(throwing: CowchatConnectionError.notConnected)
        readyContinuation = nil
        failPending(with: CowchatConnectionError.notConnected)
    }

    private func request(type: String, payload: Payload) async throws -> Payload {
        guard let connection else { throw CowchatConnectionError.notConnected }
        let id = UUID().uuidString
        let frame: Payload = ["id": id, "type": type, "payload": payload]
        guard JSONSerialization.isValidJSONObject(frame),
              var data = try? JSONSerialization.data(withJSONObject: frame) else {
            throw CowchatConnectionError.invalidResponse
        }
        data.append(0x0A)

        return try await withCheckedThrowingContinuation { continuation in
            pending[id] = continuation
            connection.send(content: data, completion: .contentProcessed { [weak self] error in
                guard let error else { return }
                Task { @MainActor in
                    self?.finishRequest(id: id, result: .failure(
                        CowchatConnectionError.transport(error.localizedDescription)
                    ))
                }
            })
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: 10_000_000_000)
                self?.timeoutRequest(id: id)
            }
        }
    }

    private func receiveNext(on connection: NWConnection) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65_536) { [weak self, weak connection] data, _, isComplete, error in
            Task { @MainActor in
                guard let self, let connection, connection === self.connection else { return }
                if let data { self.consume(data) }
                if let error {
                    self.failConnection(CowchatConnectionError.transport(error.localizedDescription))
                } else if isComplete {
                    self.failConnection(CowchatConnectionError.notConnected)
                } else {
                    self.receiveNext(on: connection)
                }
            }
        }
    }

    private func consume(_ data: Data) {
        buffer.append(data)
        guard buffer.count <= Self.maximumFrameBytes else {
            failConnection(CowchatConnectionError.transport("Cowchat sent a frame larger than 1 MiB."))
            return
        }
        while let newline = buffer.firstIndex(of: 0x0A) {
            let line = buffer[..<newline]
            buffer.removeSubrange(...newline)
            guard !line.isEmpty,
                  let object = try? JSONSerialization.jsonObject(with: Data(line)) as? Payload,
                  let type = object["type"] as? String else { continue }
            let payload = object["payload"] as? Payload ?? [:]

            if type == "ping", let id = object["id"] as? String {
                sendFrame(["id": UUID().uuidString, "type": "pong", "reply_to": id, "payload": [:]])
            } else if let replyTo = object["reply_to"] as? String {
                if type == "error" {
                    let message = payload["message"] as? String ?? "Unknown server error"
                    finishRequest(id: replyTo, result: .failure(CowchatConnectionError.server(message)))
                } else {
                    finishRequest(id: replyTo, result: .success(payload))
                }
            } else {
                onEvent?(type, payload)
            }
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
            failConnection(CowchatConnectionError.transport(error.localizedDescription))
        case .failed(let error):
            failConnection(CowchatConnectionError.transport(error.localizedDescription))
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
        connection = nil
        connectAttempt = nil
        failedConnection?.stateUpdateHandler = nil
        failedConnection?.cancel()
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
        guard let connection,
              JSONSerialization.isValidJSONObject(frame),
              var data = try? JSONSerialization.data(withJSONObject: frame),
              data.count <= Self.maximumFrameBytes else { return }
        data.append(0x0A)
        connection.send(content: data, completion: .idempotent)
    }

    private func decode<T: Decodable>(_ type: T.Type, from object: Any) throws -> T {
        guard JSONSerialization.isValidJSONObject(object) else {
            throw CowchatConnectionError.invalidResponse
        }
        let data = try JSONSerialization.data(withJSONObject: object)
        return try JSONDecoder().decode(type, from: data)
    }
}
