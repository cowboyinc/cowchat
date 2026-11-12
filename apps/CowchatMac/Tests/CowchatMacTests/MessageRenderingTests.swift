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

/// A redraw of the room rebuilds every row, so the parsing a message needs must happen once per distinct content.
final class MessageParseCacheTests: XCTestCase {
    @MainActor
    func testRebuildingAMessageReusesItsLayout() {
        let content = "Intro \(UUID()).\n\n## Heading\n\n- item"
        let misses = MessageTextLayout.cache.misses

        let first = MessageTextLayout.cached(for: content)
        for _ in 0..<50 { _ = ExpandableMessageText(content: content) }
        let again = MessageTextLayout.cached(for: content)

        XCTAssertTrue(first === again)
        XCTAssertEqual(MessageTextLayout.cache.misses, misses + 1, "the message was parsed more than once")
    }

    @MainActor
    func testLayoutMatchesTheUncachedParse() {
        let content = "Intro.\n\n## Heading\n\n- item"
        let layout = MessageTextLayout.cached(for: content)
        XCTAssertEqual(layout.collapsedPlan, MessageRenderPlan.make(for: content))
        XCTAssertNil(layout.chart)
        XCTAssertFalse(layout.showsResponseControl)
    }

    func testInlineMarkdownIsParsedOncePerText() {
        let text = "some **bold** \(UUID())"
        let misses = MessageContentParser.inlineCache.misses

        let first = MessageContentParser.attributedInline(text)
        for _ in 0..<50 { _ = MessageContentParser.attributedInline(text) }

        XCTAssertEqual(MessageContentParser.inlineCache.misses, misses + 1)
        XCTAssertEqual(MessageContentParser.attributedInline(text), first)
    }
}
