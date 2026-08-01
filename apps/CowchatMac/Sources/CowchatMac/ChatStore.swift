import Foundation

@MainActor
final class ChatStore: ObservableObject {
    @Published var rooms: [Room] = []
    @Published var selectedRoomID: String?
    @Published var messages: [ChatMessage] = []
    @Published var roomMembers: [AgentPresence] = []
    @Published var draft = ""
    @Published var searchText = ""
    @Published var connectionStatus: ConnectionStatus = .disconnected
    @Published var errorMessage: String?
    @Published var isLoadingMessages = false
    @Published var isCreateRoomPresented = false

    private let connection: any CowchatConnectionProtocol
    private var reconnectTask: Task<Void, Never>?
    private var roomLoadTask: Task<Void, Never>?
    private var joinedRoomID: String?
    private var roomSelectionGeneration = 0
    private(set) var agentID = ""
    let agentName = "Cowchat Mac"
    private let stableAgentID: String

    var selectedRoom: Room? {
        rooms.first { $0.roomID == selectedRoomID }
    }

    var filteredRooms: [Room] {
        guard !searchText.isEmpty else { return rooms }
        return rooms.filter {
            $0.name.localizedCaseInsensitiveContains(searchText)
                || ($0.description?.localizedCaseInsensitiveContains(searchText) ?? false)
        }
    }

    static func resolveAgentID(defaults: UserDefaults) -> String {
        let key = "CowchatMac.agentID"
        if let existing = defaults.string(forKey: key), !existing.isEmpty {
            return existing
        }
        let generated = "cowchat-mac-\(UUID().uuidString.lowercased())"
        defaults.set(generated, forKey: key)
        return generated
    }

    convenience init() {
        self.init(connection: CowchatConnection())
    }

    init(connection: any CowchatConnectionProtocol) {
        self.connection = connection
        stableAgentID = Self.resolveAgentID(defaults: .standard)
        connection.onEvent = { [weak self] type, payload in
            self?.handleEvent(type: type, payload: payload)
        }
        connection.onStatusChange = { [weak self] status in
            self?.handleConnectionStatus(status)
        }
    }

    func start() {
        guard connectionStatus != .connecting, !connectionStatus.isConnected else { return }
        reconnectTask?.cancel()
        reconnectTask = nil
        Task { await connect() }
    }

    func connect() async {
        errorMessage = nil
        do {
            try await connection.connect()
            agentID = try await connection.register(name: agentName, agentID: stableAgentID)
            joinedRoomID = nil
            connectionStatus = .connected
            reconnectTask?.cancel()
            reconnectTask = nil
            try await refreshRooms()
            if selectedRoomID == nil {
                let initial = rooms.first(where: { $0.name.lowercased() == "lobby" }) ?? rooms.first
                if let initial { await select(room: initial) }
            } else if let selectedRoom { await select(room: selectedRoom) }
        } catch {
            connectionStatus = .failed(error.localizedDescription)
            scheduleReconnect()
        }
    }

    func refreshRooms() async throws {
        rooms = try await connection.listRooms().sorted(by: roomSort)
    }

    func select(room: Room) async {
        roomSelectionGeneration += 1
        let generation = roomSelectionGeneration
        selectedRoomID = room.roomID
        messages = []
        roomMembers = []
        guard connectionStatus.isConnected else {
            isLoadingMessages = false
            return
        }
        isLoadingMessages = true
        errorMessage = nil
        let previousTransition = roomLoadTask
        let transition = Task { [weak self] in
            guard let self else { return }
            await previousTransition?.value
            guard generation == roomSelectionGeneration else { return }
            do {
                if let joinedRoomID, joinedRoomID != room.roomID {
                    try await connection.leave(roomID: joinedRoomID)
                    self.joinedRoomID = nil
                }
                if joinedRoomID != room.roomID {
                    try await connection.join(roomID: room.roomID)
                    self.joinedRoomID = room.roomID
                }
                // A newer selection may have arrived while join was in flight.
                // The next serialized transition will leave this actual joined
                // room; this stale load must not mutate the visible conversation.
                guard generation == roomSelectionGeneration else { return }
                let history = try await connection.history(roomID: room.roomID, limit: 100)
                guard generation == roomSelectionGeneration,
                      selectedRoomID == room.roomID else { return }
                messages = Self.merging(history: history, live: messages)
                let members = try await connection.listAgents(roomID: room.roomID)
                guard generation == roomSelectionGeneration,
                      selectedRoomID == room.roomID else { return }
                roomMembers = members.sorted {
                    $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
                }
            } catch {
                guard generation == roomSelectionGeneration,
                      selectedRoomID == room.roomID else { return }
                present(error)
            }
            if generation == roomSelectionGeneration,
               selectedRoomID == room.roomID { isLoadingMessages = false }
        }
        roomLoadTask = transition
        await transition.value
    }

    func createRoom(name: String, description: String, ephemeral: Bool, isPublic: Bool) async -> Bool {
        do {
            let room = try await connection.createRoom(
                name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                description: description.trimmingCharacters(in: .whitespacesAndNewlines),
                ephemeral: ephemeral,
                isPublic: isPublic
            )
            if !rooms.contains(where: { $0.id == room.id }) { rooms.append(room) }
            rooms.sort(by: roomSort)
            isCreateRoomPresented = false
            await select(room: room)
            return true
        } catch {
            present(error)
            return false
        }
    }

    func sendDraft() {
        guard let room = selectedRoom, !room.encrypted else { return }
        let content = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !content.isEmpty else { return }
        guard connectionStatus.isConnected else {
            start()
            return
        }
        draft = ""
        Task {
            do {
                let message = try await connection.send(roomID: room.roomID, content: content)
                if selectedRoomID == room.roomID { append(message) }
            } catch {
                if selectedRoomID == room.roomID, draft.isEmpty { draft = content }
                present(error)
            }
        }
    }

    private func handleEvent(type: String, payload: [String: Any]) {
        switch type {
        case "message_received":
            if let message = try? decode(ChatMessage.self, payload), message.roomID == selectedRoomID {
                append(message)
            }
        case "room_created":
            if let room = try? decode(Room.self, payload), !rooms.contains(where: { $0.id == room.id }) {
                rooms.append(room)
                rooms.sort(by: roomSort)
            }
        case "room_destroyed":
            if let id = payload["room_id"] as? String {
                rooms.removeAll { $0.roomID == id }
                if selectedRoomID == id { roomMembers = [] }
            }
        case "agent_joined", "agent_left":
            Task { await refreshMembers() }
        case "presence_update":
            updatePresence(from: payload)
        default:
            break
        }
    }

    private func handleConnectionStatus(_ status: ConnectionStatus) {
        connectionStatus = status
        if case .failed = status {
            joinedRoomID = nil
            roomMembers = []
            scheduleReconnect()
        }
    }

    private func refreshMembers() async {
        guard connectionStatus.isConnected, let roomID = selectedRoomID else { return }
        guard let members = try? await connection.listAgents(roomID: roomID),
              selectedRoomID == roomID else { return }
        roomMembers = members.sorted {
            $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
    }

    private func updatePresence(from payload: [String: Any]) {
        guard let agentID = payload["agent_id"] as? String,
              let index = roomMembers.firstIndex(where: { $0.agentID == agentID }) else { return }
        roomMembers[index] = roomMembers[index].updating(
            status: payload["status"] as? String,
            detail: payload["status_detail"] as? String,
            progress: payload["progress"] as? Int
        )
    }

    private func scheduleReconnect() {
        guard reconnectTask == nil else { return }
        reconnectTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            guard !Task.isCancelled, let self else { return }
            reconnectTask = nil
            await connect()
        }
    }

    private func append(_ message: ChatMessage) {
        guard !messages.contains(where: { $0.id == message.id }) else { return }
        messages.append(message)
        messages.sort { $0.seq < $1.seq }
        if messages.count > 200 { messages.removeFirst(messages.count - 200) }
    }

    static func merging(history: [ChatMessage], live: [ChatMessage]) -> [ChatMessage] {
        var byID: [String: ChatMessage] = [:]
        for message in history + live { byID[message.id] = message }
        return byID.values.sorted { $0.seq < $1.seq }
    }

    private func decode<T: Decodable>(_ type: T.Type, _ object: [String: Any]) throws -> T {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try JSONDecoder().decode(type, from: data)
    }

    private func roomSort(_ lhs: Room, _ rhs: Room) -> Bool {
        if lhs.name.lowercased() == "lobby" { return true }
        if rhs.name.lowercased() == "lobby" { return false }
        return (lhs.lastActivity ?? lhs.createdAt) > (rhs.lastActivity ?? rhs.createdAt)
    }

    private func present(_ error: Error) {
        errorMessage = error.localizedDescription
    }
}
