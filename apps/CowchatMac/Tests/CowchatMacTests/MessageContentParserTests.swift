import XCTest
@testable import CowchatMac

final class MessageContentParserTests: XCTestCase {
    func testSeparatesFencedCodeFromSurroundingProse() {
        let segments = MessageContentParser.segments(in: """
        Before **bold**.
        ```swift
        let answer = 42
        ```
        After.
        """)

        XCTAssertEqual(segments.map(\.kind), [.prose, .code, .prose])
        XCTAssertTrue(segments[0].text.contains("Before **bold**."))
        XCTAssertEqual(segments[1].text, "let answer = 42\n")
        XCTAssertTrue(segments[2].text.contains("After."))
    }

    func testUnclosedFenceTreatsRemainderAsCode() {
        let segments = MessageContentParser.segments(in: "Text\n```\ncommand")

        XCTAssertEqual(segments.map(\.kind), [.prose, .code])
        XCTAssertEqual(segments.last?.text, "command")
    }
}
