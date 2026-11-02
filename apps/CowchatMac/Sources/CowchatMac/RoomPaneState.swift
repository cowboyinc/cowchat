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
        // Spec §3 decision-table order. Its lobby row is gone: the lobby is an
        // ordinary room (Patrick removed the Lobby page, 2 November).
        if !connectionStatus.isConnected && !hasMessages {
            return .connectPrompt(connectionStatus == .connecting ? .connecting : .offline)
        }
        if isLoadingMessages { return .loading }
        if hasMessages { return .chat }
        if hasOtherMembers { return .quiet }
        return .connectPrompt(.connected)
    }

    /// The floating "Invite a collaborator" nudge: live chat, but nobody
    /// else here. The connect and quiet states carry their own invite CTAs,
    /// so the nudge only floats over an active feed.
    static func showsInviteNudge(state: RoomPaneState, hasOtherMembers: Bool) -> Bool {
        state == .chat && !hasOtherMembers
    }

    /// Whether the quiet state offers Copy connect prompt: everywhere but Home while one of the person's sessions is
    /// live there, since an agent is already in.
    static func quietOffersConnectPrompt(isHome: Bool, liveSessions: Int) -> Bool {
        !(isHome && liveSessions > 0)
    }
}
