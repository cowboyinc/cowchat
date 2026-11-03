/// What the main pane renders for a selected room. Purely derived from live signals; persisted flags never gate
/// rendering. "Loading" means an in-flight history fetch on an established connection only.
///
/// An empty room draws one state whoever is in it (navigation note, section 5.1): sessions join their rooms by
/// themselves through the plugin and hooks, so nobody pastes a join prompt into an account room.
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

    /// Whether the empty state offers Copy connect prompt: in This Mac and API-key rooms, where an agent still gets in
    /// only through a pasted prompt, since the hooks join the account. Account rooms bring agents in through the
    /// person's sessions, Add person, People… and the Join sheet.
    static func emptyOffersConnectPrompt(isAccountRoom: Bool) -> Bool {
        !isAccountRoom
    }
}
