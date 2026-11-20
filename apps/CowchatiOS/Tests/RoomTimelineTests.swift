import XCTest
@testable import CowchatiOS
import CowchatKit

final class RoomTimelineTests: XCTestCase {
    func testMergeOrdersBySeqAndDeduplicatesByID() {
        let timeline = RoomTimeline(entries: [entry("b", seq: 2), entry("d", seq: 4)])
        let merged = timeline.merging([entry("c", seq: 3), entry("a", seq: 1), entry("b", seq: 2)])
        XCTAssertEqual(merged.entries.map(\.id), ["a", "b", "c", "d"])
    }

    func testMergeReplacesAnExistingEntryWithTheIncomingOne() {
        let timeline = RoomTimeline(entries: [entry("a", seq: 1, text: "old")])
        let merged = timeline.merging([entry("a", seq: 1, text: "new")])
        XCTAssertEqual(merged.entries.map(\.body), [.text("new")])
    }

    func testLiveMessageAppendsAndItsEchoDoesNotDuplicate() {
        let timeline = RoomTimeline(entries: [entry("a", seq: 1)])
            .merging([entry("b", seq: 2)])
            .merging([entry("b", seq: 2)])
        XCTAssertEqual(timeline.entries.map(\.id), ["a", "b"])
    }

    func testMergeKeepsPagingFlags() {
        let timeline = RoomTimeline(entries: [], hasEarlier: false, isLoading: true)
        let merged = timeline.merging([entry("a", seq: 1)])
        XCTAssertEqual(merged.hasEarlier, false)
        XCTAssertEqual(merged.isLoading, true)
    }

    func testReconnectBackoffDoublesAndCapsAtThirtySeconds() {
        XCTAssertEqual(
            (0..<8).map(ReconnectBackoff.delay(afterFailures:)),
            [1, 2, 4, 8, 16, 30, 30, 30].map { Duration.seconds($0) }
        )
    }

    // MARK: Feed

    func testHiddenAgentMessagesFoldIntoOneRowBetweenShownMessages() {
        let entries = [
            entry("a", seq: 1), entry("b", seq: 2, forPeople: false), entry("c", seq: 3, forPeople: false),
            entry("d", seq: 4), entry("e", seq: 5, forPeople: false),
        ]
        let items = TimelineFeed.items(entries, showsAll: false, openRuns: [])
        XCTAssertEqual(items.map(\.id), ["a", "run/b", "d", "run/e"])
        XCTAssertEqual(items[1], .run(id: "b", count: 2, isOpen: false))
        XCTAssertEqual(TimelineFeed.runTitle(count: 2), "2 agent messages")
        XCTAssertEqual(TimelineFeed.runTitle(count: 1), "1 agent message")
    }

    func testAnOpenRunListsItsMessagesQuietlyAfterItsRow() {
        let entries = [entry("a", seq: 1), entry("b", seq: 2, forPeople: false), entry("c", seq: 3, forPeople: false)]
        let items = TimelineFeed.items(entries, showsAll: false, openRuns: ["b"])
        XCTAssertEqual(items.map(\.id), ["a", "run/b", "b", "c"])
        XCTAssertEqual(items[2], .entry(entries[1], quiet: true))
    }

    func testShowAllListsEveryMessageWithNoRunRows() {
        let entries = [entry("a", seq: 1), entry("b", seq: 2, forPeople: false)]
        let items = TimelineFeed.items(entries, showsAll: true, openRuns: [])
        XCTAssertEqual(items, [.entry(entries[0], quiet: false), .entry(entries[1], quiet: true)])
    }

    // MARK: Presence line

    func testPresenceLineNamesActiveAgentsThenCountsConnectedOnes() {
        let idle = RoomMember(id: "a", name: "Ada")
        let working = RoomMember(id: "b", name: "Bo", status: "working")
        let me = RoomMember(id: "me", name: "iPhone", status: "working")
        XCTAssertEqual(PresenceLine.summary(members: [idle, working, me], myAgentID: "me", phase: .connected), "Bo active")
        XCTAssertEqual(PresenceLine.summary(members: [idle, me], myAgentID: "me", phase: .connected), "1 agent connected")
        XCTAssertEqual(PresenceLine.summary(members: [idle, RoomMember(id: "c", name: "Cy"), me], myAgentID: "me", phase: .connected), "2 agents connected")
        XCTAssertEqual(PresenceLine.summary(members: [me], myAgentID: "me", phase: .connected), "No agents connected")
        XCTAssertEqual(PresenceLine.summary(members: [idle], myAgentID: "me", phase: .reconnecting), "Offline")
    }
}
