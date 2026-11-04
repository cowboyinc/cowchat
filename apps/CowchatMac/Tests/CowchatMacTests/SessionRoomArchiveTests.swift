import XCTest
@testable import CowchatMac

/// The archive rule's decisions for an ended session's room (navigation note, section 4.4). The simulated server's
/// tests in `Proto/ProtoSessionRoomsTests` run it through the workspace.
final class SessionRoomArchiveTests: XCTestCase {
    private let patrick = SocialAccount(id: "patrick", handle: "Patrick-Mandia", displayName: "Patrick Mandia")
    private let chad = SocialAccount(id: "chad", handle: "Chad-DePue", displayName: "Chad DePue")

    private func room(_ id: String, owner: String = "patrick", agent: String? = nil, ended: Bool = true) -> SocialRoom {
        SocialRoom(roomId: id, name: id, ownerId: owner, kind: "session",
                   session: SocialRoom.Session(agentId: agent ?? "agent-\(id)", harness: "claude-code",
                                               folder: "cowchat", branch: "main",
                                               startedAt: "2026-11-02T08:00:00.000Z",
                                               endedAt: ended ? "2026-11-02T09:00:00.000Z" : nil))
    }

    private func invitation(_ roomID: String, status: String) -> SocialRoomInvitation {
        SocialRoomInvitation(id: "i-\(roomID)", roomId: roomID, roomName: roomID, from: patrick, to: chad,
                             proposedAgents: [], members: [patrick], agents: [], historyPolicy: "full",
                             encrypted: false, slackConnected: false, status: status)
    }

    private let ended: (SocialRoom.Session) -> Bool = { $0.endedAt != nil }

    func testOnlyTheViewersEndedSessionRoomsTheRuleHasNotSettledAreChecked() {
        let rooms = [
            room("new"), room("waiting"), room("archived"), room("kept"), room("live", ended: false),
            room("chads", owner: "chad"), SocialRoom(roomId: "district", name: "District", ownerId: "patrick"),
        ]
        let outcomes: [String: SessionRoomArchive.Outcome] = ["waiting": .waiting, "archived": .archived,
                                                              "kept": .kept]
        XCTAssertEqual(SessionRoomArchive.roomsToCheck(rooms, viewerID: "patrick", hasEnded: ended, outcomes: outcomes)
                           .map(\.roomId), ["new", "waiting"],
                       "A teammate's session room is never the viewer's to archive, and an unarchived room stays")
    }

    func testASessionThatEndedWhileTheAppWasQuitIsCheckedAtLaunch() {
        // At launch nothing is settled yet, and the rooms list says the session ended.
        XCTAssertEqual(SessionRoomArchive.roomsToCheck([room("quit")], viewerID: "patrick", hasEnded: ended,
                                                       outcomes: [:]).map(\.roomId), ["quit"])
    }

    func testAResumedSessionsRoomLosesItsEntry() {
        let rooms = [room("back", ended: false), room("still", ended: true), room("fresh", ended: false)]
        XCTAssertEqual(SessionRoomArchive.resumed(rooms, viewerID: "patrick", hasEnded: ended,
                                                  outcomes: ["back": .archived, "still": .archived]),
                       ["back"])
    }

    func testSomeoneElseIsAnotherMemberOrAnOpenInvitation() {
        XCTAssertFalse(SessionRoomArchive.hasSomeoneElse(members: [patrick], viewerID: "patrick", roomID: "r",
                                                         sentInvitations: []))
        XCTAssertTrue(SessionRoomArchive.hasSomeoneElse(members: [patrick, chad], viewerID: "patrick", roomID: "r",
                                                        sentInvitations: []))
        XCTAssertTrue(SessionRoomArchive.hasSomeoneElse(members: [patrick], viewerID: "patrick", roomID: "r",
                                                        sentInvitations: [invitation("r", status: "pending")]))
        XCTAssertFalse(SessionRoomArchive.hasSomeoneElse(members: [patrick], viewerID: "patrick", roomID: "r",
                                                         sentInvitations: [invitation("r", status: "declined"),
                                                                           invitation("other", status: "pending")]))
    }

    func testAReadClosedRoomWithOnlyThePersonArchivesAtOnce() {
        XCTAssertEqual(SessionRoomArchive.decide(hasSomeoneElse: false, isUnread: false, isOpen: false), .archive)
    }

    func testAnUnreadOrOpenRoomWaits() {
        XCTAssertEqual(SessionRoomArchive.decide(hasSomeoneElse: false, isUnread: true, isOpen: false), .wait)
        XCTAssertEqual(SessionRoomArchive.decide(hasSomeoneElse: false, isUnread: false, isOpen: true), .wait)
        XCTAssertFalse(SessionRoomArchive.isReady(isUnread: true, isOpen: false))
        XCTAssertFalse(SessionRoomArchive.isReady(isUnread: false, isOpen: true))
        XCTAssertTrue(SessionRoomArchive.isReady(isUnread: false, isOpen: false), "Read, and another room is open")
    }

    func testSomeoneElseKeepsTheRoom() {
        XCTAssertEqual(SessionRoomArchive.decide(hasSomeoneElse: true, isUnread: true, isOpen: true), .keep)
        XCTAssertEqual(SessionRoomArchive.decide(hasSomeoneElse: true, isUnread: false, isOpen: false), .keep)
    }
}
