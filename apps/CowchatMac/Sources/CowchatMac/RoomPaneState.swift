/// What the main pane renders for a selected room. Purely derived from live signals; persisted flags never gate
/// rendering. "Loading" means an in-flight history fetch on an established connection only.
///
enum RoomPaneState: Equatable {
    enum Unreachable: Equatable {
        case connecting
        case offline
    }

    case loading
    case chat
    /// A connected room with no messages yet.
    case empty
    /// No connection, and nothing loaded to show.
    case unreachable(Unreachable)

    static func state(
        connectionStatus: ConnectionStatus,
        isLoadingMessages: Bool,
        hasMessages: Bool
    ) -> RoomPaneState {
        // A room whose messages are already loaded keeps them, and its header caption says it is offline.
        if !connectionStatus.isConnected && !hasMessages {
            return .unreachable(connectionStatus == .connecting ? .connecting : .offline)
        }
        if isLoadingMessages { return .loading }
        if hasMessages { return .chat }
        return .empty
    }

}
