import XCTest
@testable import CowchatMac

/// Which messages a room shows, how it folds the rest, and the finished work a message carries in its metadata.
final class MessagesForPeopleTests: XCTestCase {
    private func message(_ id: String, metadata: [String: Any] = [:]) throws -> ChatMessage {
        try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: [
            "message_id": id,
            "room_id": "r1",
            "agent_id": "agent-\(id)",
            "agent_name": id,
            "content": "hello",
            "metadata": metadata,
            "timestamp": "2026-11-01T18:00:00Z",
            "seq": 1,
        ]))
    }

    private func person(_ id: String, flag: Bool? = nil) throws -> ChatMessage {
        var metadata: [String: Any] = ["sender_kind": "person"]
        if let flag { metadata["display_external"] = flag }
        return try message(id, metadata: metadata)
    }

    private func agent(_ id: String, flag: Bool? = nil) throws -> ChatMessage {
        var metadata: [String: Any] = ["sender_kind": "agent"]
        if let flag { metadata["display_external"] = flag }
        return try message(id, metadata: metadata)
    }

    func testAPersonsMessageAlwaysShows() throws {
        for flag in [true, false, nil] as [Bool?] {
            XCTAssertTrue(MessageVisibility.showsByDefault(try person("p", flag: flag)), "flag \(String(describing: flag))")
        }
    }

    func testAnAgentsMessageShowsOnlyWhenMarked() throws {
        XCTAssertTrue(MessageVisibility.showsByDefault(try agent("marked", flag: true)))
        XCTAssertFalse(MessageVisibility.showsByDefault(try agent("unmarked", flag: false)))
        XCTAssertEqual(MessageVisibility.showsByDefault(try agent("missing")), MessageVisibility.missingFlagShows)
        XCTAssertFalse(MessageVisibility.missingFlagShows, "Until Chad settles task 8, a missing flag hides")
    }

    func testAMessageWithNoSenderKindShows() throws {
        for flag in [true, false, nil] as [Bool?] {
            var metadata: [String: Any] = [:]
            if let flag { metadata["display_external"] = flag }
            let old = try message("old", metadata: metadata)
            XCTAssertNil(old.metadata.senderKind)
            XCTAssertTrue(MessageVisibility.showsByDefault(old), "flag \(String(describing: flag))")
        }
        let unknown = try message("unknown", metadata: ["sender_kind": "robot", "display_external": false])
        XCTAssertNil(unknown.metadata.senderKind, "A kind this app does not know reads as none")
        XCTAssertEqual(unknown.metadata.displayExternal, false, "The rest of the metadata stays")
        XCTAssertTrue(MessageVisibility.showsByDefault(unknown))
    }

    func testHiddenMessagesFoldIntoRunsBetweenShownOnes() throws {
        let messages = [try person("a"), try agent("b", flag: false), try agent("c"), try agent("d", flag: true),
                        try agent("e", flag: false)]
        let items = MessageFeed.items(messages, showsAll: false, openRuns: [])
        XCTAssertEqual(items.map(\.id), ["a", "run/b", "d", "run/e"])
        guard case let .run(first) = items[1], case let .run(last) = items[3] else { return XCTFail("No runs") }
        XCTAssertEqual(first.messages.map(\.id), ["b", "c"])
        XCTAssertEqual(first.title, "2 agent messages")
        XCTAssertFalse(first.isOpen)
        XCTAssertEqual(last.title, "1 agent message")
    }

    func testAnOpenRunShowsItsMessagesQuietAfterItsRow() throws {
        let messages = [try person("a"), try agent("b", flag: false), try agent("c"), try person("d")]
        let items = MessageFeed.items(messages, showsAll: false, openRuns: ["b"])
        XCTAssertEqual(items.map(\.id), ["a", "run/b", "b", "c", "d"])
        guard case let .run(run) = items[1] else { return XCTFail("No run") }
        XCTAssertTrue(run.isOpen)
        XCTAssertEqual(items[2], .message(messages[1], quiet: true))
        XCTAssertEqual(items[4], .message(messages[3], quiet: false))

        // A later hidden message joins the same run, which keeps its name and stays open.
        let later = MessageFeed.items(Array(messages.dropLast()) + [try agent("e", flag: false)], showsAll: false,
                                      openRuns: ["b"])
        XCTAssertEqual(later.map(\.id), ["a", "run/b", "b", "c", "e"])
    }

    func testTheSwitchShowsEveryMessageWithTheHiddenOnesQuiet() throws {
        let messages = [try person("a"), try agent("b", flag: false), try agent("c", flag: true)]
        let items = MessageFeed.items(messages, showsAll: true, openRuns: [])
        XCTAssertEqual(items, [.message(messages[0], quiet: false), .message(messages[1], quiet: true),
                               .message(messages[2], quiet: false)])
    }

    func testPullRequestMetadataReads() throws {
        XCTAssertNil(try agent("plain", flag: true).metadata.pullRequest)
        let decoded = try message("work", metadata: [
            "sender_kind": "agent", "display_external": true,
            "pull_request": [
                "repository": "cowboyinc/cowchat", "number": 1419, "title": "Unread count clears once per person",
                "checks": "passing", "additions": 214, "deletions": 38,
                "address": "https://github.com/cowboyinc/cowchat/pull/1419",
            ],
        ])
        let pullRequest = try XCTUnwrap(decoded.metadata.pullRequest)
        XCTAssertEqual(pullRequest.repositoryName, "cowchat")
        XCTAssertEqual(pullRequest.name, "cowchat #1419")
        XCTAssertEqual(pullRequest.checks, .passing)
        XCTAssertEqual(pullRequest.checksLabel, "Checks passed")
        XCTAssertEqual(pullRequest.diff?.added, "+214")
        XCTAssertEqual(pullRequest.diff?.removed, "\u{2212}38")
        XCTAssertEqual(pullRequest.url?.absoluteString, "https://github.com/cowboyinc/cowchat/pull/1419")
    }

    func testPullRequestMetadataWithoutOptionalFields() throws {
        let decoded = try message("work", metadata: [
            "pull_request": ["repository": "cowchat", "number": 7, "title": "Docs", "checks": "queued",
                             "address": "javascript:alert(1)"],
        ])
        let pullRequest = try XCTUnwrap(decoded.metadata.pullRequest)
        XCTAssertEqual(pullRequest.name, "cowchat #7")
        XCTAssertNil(pullRequest.checks, "A state this app does not know draws no mark")
        XCTAssertNil(pullRequest.checksLabel)
        XCTAssertNil(pullRequest.diff)
        XCTAssertNil(pullRequest.url, "Only a web address opens")
    }

    func testMalformedPullRequestIsDroppedAlone() throws {
        let decoded = try message("work", metadata: [
            "sender_kind": "agent", "display_external": true, "type": "note",
            "pull_request": ["repository": "cowboyinc/cowchat", "number": "1419", "title": "Bad number",
                             "address": "https://github.com/cowboyinc/cowchat/pull/1419"],
        ])
        XCTAssertNil(decoded.metadata.pullRequest)
        XCTAssertEqual(decoded.metadata.senderKind, .agent)
        XCTAssertEqual(decoded.metadata.displayExternal, true)
        XCTAssertEqual(decoded.metadata.type, "note")

        let notAnObject = try message("work", metadata: ["pull_request": "cowchat #1419", "display_external": true])
        XCTAssertNil(notAnObject.metadata.pullRequest)
        XCTAssertEqual(notAnObject.metadata.displayExternal, true)
    }

    func testAttachmentsReadAndAMalformedOneIsDroppedAlone() throws {
        let decoded = try message("work", metadata: [
            "attachments": [
                ["blob_id": "blob-1", "name": "unread-count-sidebar.png", "size": 41_490, "sha256": "ab"],
                ["name": "no-blob.png"],
                ["blob_id": "blob-2"],
                ["blob_id": "", "name": "empty.png"],
            ],
        ])
        XCTAssertEqual(decoded.metadata.attachments.count, 3, "The entry with no blob ID is dropped")
        XCTAssertEqual(decoded.attachments, [
            FileAttachment(blobID: "blob-1", name: "unread-count-sidebar.png", size: 41_490),
            FileAttachment(blobID: "blob-2", name: "attachment", size: nil),
        ])
        XCTAssertTrue(decoded.attachments[0].isImage)
        XCTAssertFalse(decoded.attachments[1].isImage)

        let wrongShape = try message("work", metadata: ["attachments": "blob-1", "display_external": false])
        XCTAssertEqual(wrongShape.attachments, [])
        XCTAssertEqual(wrongShape.metadata.displayExternal, false)
    }

    private func tagged(_ id: String, _ tags: [[String: String]]) throws -> ChatMessage {
        try message(id, metadata: ["sender_kind": "agent", "tags": tags])
    }

    func testTagsDecodeAndAMalformedOneIsDroppedAlone() throws {
        let decoded = try message("t", metadata: ["tags": [["key": "github", "value": "core#1", "source": "extractor"],
                                                          ["value": "no key"], ["key": "urgent"]]])
        XCTAssertEqual(decoded.metadata.tags.map(\.label), ["github=core#1", "urgent"])
    }

    func testATagFilterShowsOnlyMatchingMessagesEvenWhenARoomHidesThem() throws {
        let messages = [
            try person("p"),
            try tagged("a", [["key": "github", "value": "core#1"]]),
            try tagged("b", [["key": "github", "value": "core#2"]]),
        ]
        let exact = MessageFeed.items(messages, showsAll: false, openRuns: [], tag: "github=core#1")
        XCTAssertEqual(exact.map(\.id), ["a"])
        XCTAssertEqual(exact, [.message(messages[1], quiet: false)])
        let anyValue = MessageFeed.items(messages, showsAll: false, openRuns: [], tag: "github")
        XCTAssertEqual(anyValue.map(\.id), ["a", "b"])
    }

    func testAFilterListsEachLoadedTagOnceInOrder() throws {
        let messages = [
            try tagged("a", [["key": "topic", "value": "router"], ["key": "github", "value": "core#1"]]),
            try tagged("b", [["key": "topic", "value": "router"]]),
        ]
        XCTAssertEqual(MessageFeed.tagLabels(messages), ["github=core#1", "topic=router"])
    }
}
