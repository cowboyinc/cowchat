import AppKit
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

        for _ in 0..<50 { _ = ExpandableMessageText(content: content) }

        XCTAssertEqual(MessageTextLayout.cache.misses, misses + 1, "the views parsed the message more than once")
        XCTAssertTrue(MessageTextLayout.cache.contains(content), "the views did not use the shared cache")
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

final class MessageExpansionTests: XCTestCase {
    /// Forgetting a room key turns an open message's text into a placeholder; the old plaintext plan must not draw.
    func testAnOpenMessageWhoseTextChangedDrawsNoOldPlan() {
        let plaintext = "Secret.\n\n## Heading\n\n- item"
        let expansion = MessageExpansion(content: plaintext, plan: MessageRenderPlan.make(for: plaintext), isOpen: true)

        XCTAssertNotNil(expansion.openPlan(for: plaintext))
        XCTAssertNil(expansion.openPlan(for: "Encrypted message — add the room key to read it."))
        XCTAssertNil(expansion.plan(for: "Encrypted message — add the room key to read it."))
    }

    func testAClosedMessageKeepsItsPlanForReopening() {
        let content = "Long message"
        let expansion = MessageExpansion(content: content, plan: .plainText(content), isOpen: false)

        XCTAssertNil(expansion.openPlan(for: content))
        XCTAssertEqual(expansion.plan(for: content), .plainText(content))
    }

    /// A closed row keeps its plan for reopening, but forgetting the room key must release that plaintext too.
    func testForgettingTheKeyReleasesAClosedExpansion() {
        let plaintext = "Secret.\n\n## Heading\n\n- item"
        let placeholder = "Encrypted message — add the room key to read it."

        let opened = MessageExpansion.toggled(nil, content: plaintext, makePlan: MessageRenderPlan.make)
        let closed = MessageExpansion.toggled(opened, content: plaintext, makePlan: MessageRenderPlan.make)
        XCTAssertFalse(closed.isOpen)
        XCTAssertEqual(closed.content, plaintext)

        XCTAssertNil(MessageExpansion.retained(closed, for: placeholder))
        XCTAssertEqual(MessageExpansion.retained(closed, for: plaintext), closed)
    }
}

final class ExpandableMessageRowTests: XCTestCase {
    /// Drives the live row: expand, collapse, then forget the room key by redrawing the row with the placeholder.
    @MainActor
    func testForgettingTheKeyReleasesTheRowsClosedExpansion() async throws {
        let plaintext = "Secret.\n\n## Heading\n\n" + (1...40).map { "- item \($0)" }.joined(separator: "\n")
        let placeholder = "Encrypted message — add the room key to read it."
        let state = MessageExpansionState()
        let host = NSHostingView(rootView: ExpandableMessageText(content: plaintext, expansionState: state))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 600, height: 400),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentView = host
        func redraw() async throws {
            window.display(); host.layoutSubtreeIfNeeded()
            try await Task.sleep(nanoseconds: 100_000_000)
            window.display(); host.layoutSubtreeIfNeeded()
        }
        try await redraw()

        state.toggle(content: plaintext, makePlan: MessageRenderPlan.make)
        state.toggle(content: plaintext, makePlan: MessageRenderPlan.make)
        try await redraw()
        XCTAssertEqual(state.expansion?.isOpen, false)
        XCTAssertEqual(state.expansion?.content, plaintext, "a closed row keeps its plan while its text is unchanged")

        host.rootView = ExpandableMessageText(content: placeholder, expansionState: state)
        try await redraw()
        XCTAssertNil(state.expansion, "the row must drop the old plaintext once its text changes")
    }
}
