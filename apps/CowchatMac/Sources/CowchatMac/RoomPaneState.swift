/// What the main pane renders for a selected, non-lobby room — §3 of the
/// onboarding spec. Purely derived from live signals; persisted flags never
/// gate rendering. "Loading" means an in-flight history fetch on an
/// established connection only.
enum RoomPaneState: Equatable {
    enum ConnectVariant: Equatable {
        case connected
        case connecting
        case offline
    }

    case connectPrompt(ConnectVariant)
    case loading
    case chat
    case quiet

    static func state(
        connectionStatus: ConnectionStatus,
        isLoadingMessages: Bool,
        hasMessages: Bool,
        hasOtherMembers: Bool
    ) -> RoomPaneState {
        // Spec §3 decision-table order, minus the lobby row (handled by
        // ContentView's outer switch before this function is reached).
        if !connectionStatus.isConnected && !hasMessages {
            return .connectPrompt(connectionStatus == .connecting ? .connecting : .offline)
        }
        if isLoadingMessages { return .loading }
        if hasMessages { return .chat }
        if hasOtherMembers { return .quiet }
        return .connectPrompt(.connected)
    }
}
