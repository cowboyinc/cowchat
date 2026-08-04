import XCTest
@testable import CowchatMac

final class MessagePreviewTests: XCTestCase {
    func testShortMultilineResponseCollapsesToFiveLines() {
        let content = (1...6).map { "Line \($0)" }.joined(separator: "\n")

        XCTAssertTrue(MessagePreview.needsDisclosure(for: content))
        XCTAssertEqual(
            MessagePreview.collapsedContent(for: content),
            "Line 1\nLine 2\nLine 3\nLine 4\nLine 5…"
        )
    }

    func testLongResponseCollapsesAtCharacterLimit() {
        let content = String(repeating: "a", count: MessagePreview.characterLimit + 1)

        XCTAssertEqual(
            MessagePreview.collapsedContent(for: content).count,
            MessagePreview.characterLimit + 1
        )
        XCTAssertTrue(MessagePreview.collapsedContent(for: content).hasSuffix("…"))
    }

    func testShortResponseDoesNotExposeDisclosureControl() {
        let content = "A short response"

        XCTAssertFalse(MessagePreview.needsDisclosure(for: content))
        XCTAssertEqual(MessagePreview.collapsedContent(for: content), content)
    }
}
