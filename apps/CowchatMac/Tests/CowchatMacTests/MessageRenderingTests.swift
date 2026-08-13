import SwiftUI
import XCTest
@testable import CowchatMac

final class MessageRenderingTests: XCTestCase {
    @MainActor
    func testScreenshotLikeBlocksRenderWithVisibleVerticalSeparation() throws {
        let content = """
        Resolved the rebase itself.

        ## WHAT THE CONFLICT WAS

        Resolved properly.
        """
        // Keep this below the disclosure threshold: otherwise the button alone
        // can make the structured renderer taller and mask a flattened body.
        XCTAssertFalse(MessagePreview.needsDisclosure(for: content))
        let structured = ImageRenderer(
            content: ExpandableMessageText(content: content)
                .frame(width: 520, alignment: .leading)
        )
        let flattened = ImageRenderer(
            content: Text("Resolved the rebase itself.WHAT THE CONFLICT WASResolved properly.")
                .frame(width: 520, alignment: .leading)
        )

        let structuredImage = try XCTUnwrap(structured.cgImage)
        let flattenedImage = try XCTUnwrap(flattened.cgImage)

        XCTAssertGreaterThan(
            structuredImage.height,
            flattenedImage.height * 3,
            "separate block views must be laid out vertically, not overlaid at one origin"
        )
    }

    func testPathologicalMarkdownUsesOneTextBackedRenderElement() {
        let content = String(repeating: "# heading\n", count: 8_000)
        let plan = MessageRenderPlan.make(for: content)

        guard case .plainText(let rendered) = plan else {
            return XCTFail("large heading-heavy messages must bypass block view expansion")
        }
        XCTAssertEqual(rendered, content)
        XCTAssertEqual(plan.renderedElementCount, 1)
    }

    func testOrdinaryMarkdownKeepsStructuredRendering() {
        let plan = MessageRenderPlan.make(for: "Intro.\n\n## Heading\n\n- item")

        guard case .structured(let segments) = plan else {
            return XCTFail("ordinary messages should retain rich block rendering")
        }
        XCTAssertEqual(segments.map(\.kind), [.prose, .heading(level: 2), .unorderedList])
        XCTAssertEqual(plan.renderedElementCount, 3)
    }

    func testCollapsedInitializationDoesNotBuildTheFullExpandedPlan() {
        let content = String(repeating: "a", count: MessagePreview.characterLimit + 1)
        var plannedSources: [String] = []

        _ = ExpandableMessageText(
            content: content,
            makeRenderPlan: { source in
                plannedSources.append(source)
                return .plainText(source)
            }
        )

        XCTAssertEqual(plannedSources, [String(content.prefix(MessagePreview.characterLimit))])
        XCTAssertFalse(plannedSources.contains(content))
    }
}
