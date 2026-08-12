import XCTest
@testable import CowchatMac

final class MessagePreviewTests: XCTestCase {
    func testShortMultilineResponseCollapsesToFiveLines() {
        let content = (1...6).map { "Line \($0)" }.joined(separator: "\n")

        XCTAssertTrue(MessagePreview.needsDisclosure(for: content))
        XCTAssertEqual(
            MessagePreview.collapsedPreview(for: content),
            .init(
                source: "Line 1\nLine 2\nLine 3\nLine 4\nLine 5",
                isTruncated: true
            )
        )
    }

    func testLongResponseCollapsesAtCharacterLimit() {
        let content = String(repeating: "a", count: MessagePreview.characterLimit + 1)
        let preview = MessagePreview.collapsedPreview(for: content)

        XCTAssertEqual(preview.source.count, MessagePreview.characterLimit)
        XCTAssertTrue(preview.isTruncated)
    }

    func testShortResponseDoesNotExposeDisclosureControl() {
        let content = "A short response"

        XCTAssertFalse(MessagePreview.needsDisclosure(for: content))
        XCTAssertEqual(
            MessagePreview.collapsedPreview(for: content),
            .init(source: content, isTruncated: false)
        )
    }

    func testCRLFLinesUseTheSameDisclosureAndCutoffAsLF() {
        let content = (1...6).map { "Line \($0)" }.joined(separator: "\r\n")

        XCTAssertTrue(MessagePreview.needsDisclosure(for: content))
        XCTAssertEqual(
            MessagePreview.collapsedPreview(for: content),
            .init(
                source: "Line 1\nLine 2\nLine 3\nLine 4\nLine 5",
                isTruncated: true
            )
        )
    }

    func testFenceAtLineBoundaryCannotConsumeTruncationIndicator() {
        let content = "one\ntwo\nthree\nfour\n```\nsecret"
        let preview = MessagePreview.collapsedPreview(for: content)

        XCTAssertEqual(preview.source, "one\ntwo\nthree\nfour\n```")
        XCTAssertTrue(preview.isTruncated)
        XCTAssertFalse(preview.source.contains("…"), "indicator is rendered outside Markdown")
        XCTAssertEqual(
            MessageContentParser.segments(in: preview.source).map(\.kind),
            [.prose, .code]
        )
    }

    func testCharacterCutoffPreservesTheExactMarkdownPrefix() {
        let content = String(repeating: "a", count: MessagePreview.characterLimit - 1)
            + "**hidden**"
        let preview = MessagePreview.collapsedPreview(for: content)

        XCTAssertEqual(preview.source, String(content.prefix(MessagePreview.characterLimit)))
        XCTAssertTrue(preview.isTruncated)
    }
}
