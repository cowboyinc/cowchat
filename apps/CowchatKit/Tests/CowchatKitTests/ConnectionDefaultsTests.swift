import XCTest
@testable import CowchatKit

/// A connection that implements only what the protocol requires, so the protocol's own defaults are what run.
@MainActor
private final class BareConnection: CowchatConnectionProtocol {
    var onEvent: ((String, [String: Any]) -> Void)?
    var onStatusChange: ((ConnectionStatus) -> Void)?
    var newestPageRequests = 0

    func connect() async throws {}
    func register(name _: String, agentID: String) async throws -> CowchatRegistration { CowchatRegistration(agentID: agentID) }
    func listRooms() async throws -> [Room] { [] }
    func listAgents(roomID _: String) async throws -> [AgentPresence] { [] }
    func createRoom(name _: String, description _: String?, parentID _: String?, isPublic _: Bool) async throws -> Room {
        throw CowchatConnectionError.invalidResponse
    }
    func rename(roomID _: String, name _: String) async throws -> Room { throw CowchatConnectionError.invalidResponse }
    func destroy(roomID _: String) async throws {}
    func createInvite(roomID _: String, singleUse _: Bool) async throws -> String { "" }
    func listInvites(roomID _: String) async throws -> [RoomInvite] { [] }
    func revokeInvite(inviteID _: String) async throws {}
    func redeemInvite(token _: String) async throws -> String { "" }
    func join(roomID _: String) async throws {}
    func leave(roomID _: String) async throws {}
    func history(roomID _: String, limit _: Int, before _: String?) async throws -> [ChatMessage] {
        newestPageRequests += 1
        return []
    }
    func send(roomID _: String, content _: String, mentions _: [String]) async throws -> ChatMessage {
        throw CowchatConnectionError.invalidResponse
    }
    func disconnect() {}
}

@MainActor
final class ConnectionDefaultsTests: XCTestCase {
    /// A connection that can't page by sequence must say so. Answering with the newest page instead would hand a
    /// caller that asked for "before 40" the same messages again, for ever.
    func testAConnectionThatCannotPageBySequenceRefusesInsteadOfReturningTheNewestPage() async {
        let connection = BareConnection()
        do {
            _ = try await connection.history(roomID: "r1", limit: 50, beforeSeq: 40, kinds: [])
            XCTFail("expected a refusal")
        } catch {
            XCTAssertTrue(error is CowchatConnectionError, "\(error)")
        }
        XCTAssertEqual(connection.newestPageRequests, 0, "the newest page was not fetched in its place")
    }
}
