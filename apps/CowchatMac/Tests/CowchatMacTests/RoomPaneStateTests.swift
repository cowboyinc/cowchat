import XCTest
@testable import CowchatMac

final class RoomPaneStateTests: XCTestCase {
    func testDisconnectedWithNothingLoadedShowsConnectPromptOfflineVariant() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .disconnected,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.offline)
        )
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .failed("boom"),
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.offline)
        )
    }

    func testConnectingWithNothingLoadedShowsConnectPromptConnectingVariant() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connecting,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.connecting)
        )
    }

    func testDisconnectionKeepsAlreadyLoadedMessagesVisible() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .disconnected,
                isLoadingMessages: false, hasMessages: true, hasOtherMembers: false
            ),
            .chat
        )
    }

    func testLoadingWinsWhileConnected() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: true, hasMessages: false, hasOtherMembers: false
            ),
            .loading
        )
    }

    func testMessagesRenderChat() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: false, hasMessages: true, hasOtherMembers: true
            ),
            .chat
        )
    }

    /// Also covers the joined-then-left case: an agent that joins and leaves
    /// before any message lands us back at exactly these inputs, so the
    /// connect prompt returns (spec §3 "consequences to implement knowingly").
    func testEmptyRoomWithNoOtherMembersShowsConnectPrompt() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.connected)
        )
    }

    func testEmptyRoomWithMembersPresentIsQuiet() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: true
            ),
            .quiet
        )
    }

    func testThinkingOnlyRecentActivityDoesNotRenderAgentlessConnectPrompt() {
        let now = Date()
        let hasCollaboratorSignal = ChatPresencePresentation.hasCollaboratorSignal(
            members: [],
            currentAgentID: "cowchat-mac",
            fallbackMemberCount: 0,
            recentActivityByAgent: ["claude": now.addingTimeInterval(-30)],
            now: now
        )

        XCTAssertTrue(hasCollaboratorSignal)
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: false,
                hasMessages: false,
                hasOtherMembers: hasCollaboratorSignal
            ),
            .quiet
        )
    }

    func testInviteNudgeOnlyFloatsOverALonelyLiveFeed() {
        XCTAssertTrue(RoomPaneState.showsInviteNudge(state: .chat, hasOtherMembers: false))
        XCTAssertFalse(RoomPaneState.showsInviteNudge(state: .chat, hasOtherMembers: true))
        // Connect and quiet states carry their own invite CTAs.
        XCTAssertFalse(
            RoomPaneState.showsInviteNudge(state: .connectPrompt(.connected), hasOtherMembers: false)
        )
        XCTAssertFalse(RoomPaneState.showsInviteNudge(state: .quiet, hasOtherMembers: true))
        XCTAssertFalse(RoomPaneState.showsInviteNudge(state: .loading, hasOtherMembers: false))
    }
}
