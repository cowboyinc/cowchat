import XCTest
@testable import CowchatMac

final class RoomPaneStateTests: XCTestCase {
    func testDisconnectedWithNothingLoadedIsOffline() {
        XCTAssertEqual(
            RoomPaneState.state(connectionStatus: .disconnected, isLoadingMessages: false, hasMessages: false),
            .unreachable(.offline)
        )
        XCTAssertEqual(
            RoomPaneState.state(connectionStatus: .failed("boom"), isLoadingMessages: false, hasMessages: false),
            .unreachable(.offline)
        )
    }

    func testConnectingWithNothingLoadedIsConnecting() {
        XCTAssertEqual(
            RoomPaneState.state(connectionStatus: .connecting, isLoadingMessages: false, hasMessages: false),
            .unreachable(.connecting)
        )
    }

    func testDisconnectionKeepsAlreadyLoadedMessagesVisible() {
        XCTAssertEqual(
            RoomPaneState.state(connectionStatus: .disconnected, isLoadingMessages: false, hasMessages: true),
            .chat
        )
    }

    func testLoadingWinsWhileConnected() {
        XCTAssertEqual(
            RoomPaneState.state(connectionStatus: .connected, isLoadingMessages: true, hasMessages: false),
            .loading
        )
    }

    func testMessagesRenderChat() {
        XCTAssertEqual(
            RoomPaneState.state(connectionStatus: .connected, isLoadingMessages: false, hasMessages: true),
            .chat
        )
    }

    /// A connected room with no messages draws the one empty state, whoever is in it (navigation note, section 5.1).
    func testConnectedRoomWithNoMessagesIsEmpty() {
        XCTAssertEqual(
            RoomPaneState.state(connectionStatus: .connected, isLoadingMessages: false, hasMessages: false),
            .empty
        )
    }

}
