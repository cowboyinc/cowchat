import XCTest
@testable import CowchatMac

final class MessageContentParserTests: XCTestCase {
    func testSeparatesBlockMarkdownWithoutFlatteningBoundaries() {
        let segments = MessageContentParser.segments(in: """
        Intro paragraph.

        ## WHAT COMES NEXT

        - first item
        - second item

        Final paragraph.
        """)

        XCTAssertEqual(
            segments.map(\.kind),
            [.prose, .heading(level: 2), .unorderedList, .prose]
        )
        XCTAssertEqual(
            segments.map(\.text),
            [
                "Intro paragraph.",
                "WHAT COMES NEXT",
                "- first item\n- second item",
                "Final paragraph.",
            ]
        )
    }

    func testInlineMarkdownPreservesBlockWhitespaceAndInlineEmphasis() {
        let source = "First paragraph.\n\nSecond **bold** paragraph."

        let rendered = MessageContentParser.attributedInline(source)

        XCTAssertEqual(String(rendered.characters), "First paragraph.\n\nSecond bold paragraph.")
        XCTAssertTrue(rendered.runs.contains { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true })
    }

    func testRecognizesMarkdownTableAsOneReadableBlock() {
        let segments = MessageContentParser.segments(in: """
        | Gate | Result |
        | --- | --- |
        | SwiftLint | pass |
        | Validation | pending |
        """)

        XCTAssertEqual(segments.map(\.kind), [.table])
        XCTAssertEqual(segments.first?.text.components(separatedBy: "\n").count, 4)
    }

    func testRecognizesOneColumnTable() {
        let segments = MessageContentParser.segments(in: "| Status |\n| --- |\n| green |")

        XCTAssertEqual(segments.map(\.kind), [.table])
        XCTAssertEqual(segments.first?.text, "| Status |\n| --- |\n| green |")
    }

    func testPipeInProseBeforeThematicRuleIsNotATable() {
        let segments = MessageContentParser.segments(in: "A | B\n---")

        XCTAssertEqual(segments.map(\.kind), [.prose])
        XCTAssertEqual(segments.first?.text, "A | B ---")
    }

    func testInlineTripleBackticksRemainProseWithoutDroppingText() {
        let segments = MessageContentParser.segments(in: "Mention ``` in prose.\nNext line")

        XCTAssertEqual(segments.map(\.kind), [.prose])
        XCTAssertEqual(segments.first?.text, "Mention ``` in prose. Next line")
    }

    func testFenceClosesOnlyOnACompatibleFenceLine() {
        let segments = MessageContentParser.segments(in: #"""
        ````swift
        let marker = "```"
        ```
        ````
        After.
        """#)

        XCTAssertEqual(segments.map(\.kind), [.code, .prose])
        XCTAssertEqual(segments[0].text, "let marker = \"```\"\n```\n")
        XCTAssertEqual(segments[1].text, "After.")
    }

    func testTildeFenceRendersAsCode() {
        let segments = MessageContentParser.segments(in: "~~~sh\necho ok\n~~~")

        XCTAssertEqual(segments.map(\.kind), [.code])
        XCTAssertEqual(segments.first?.text, "echo ok\n")
    }

    func testIndentedCodeIsNotMisclassifiedAsMarkdownBlocks() {
        let segments = MessageContentParser.segments(in: "    # literal\n    - item")

        XCTAssertEqual(segments.map(\.kind), [.code])
        XCTAssertEqual(segments.first?.text, "# literal\n- item")
    }

    func testNestedListIndentationAndContinuationArePreserved() {
        let segments = MessageContentParser.segments(in: "- parent\n  - child\n    continuation")

        XCTAssertEqual(segments.map(\.kind), [.unorderedList])
        XCTAssertEqual(segments.first?.text, "- parent\n  - child\n    continuation")
    }

    func testMixedNestedListMarkersStayWithTheirParentList() {
        let segments = MessageContentParser.segments(
            in: "- parent\n  1. numbered child\n  2. second child\n- sibling"
        )

        XCTAssertEqual(segments.map(\.kind), [.unorderedList])
        XCTAssertEqual(
            segments.first?.text,
            "- parent\n  1. numbered child\n  2. second child\n- sibling"
        )
    }

    func testFencedCodeInsideBlockQuotePreservesCodeAndBoundaries() {
        let segments = MessageContentParser.segments(in: """
        > Before.
        > ```json
        > {"ok": true}
        >     indented
        > ```
        > After.
        """)

        XCTAssertEqual(segments.map(\.kind), [.blockQuote, .code, .blockQuote])
        XCTAssertEqual(segments.map(\.text), [
            "Before.",
            "{\"ok\": true}\n    indented\n",
            "After.",
        ])
    }

    func testFencedCodeInsideListPreservesContainerIndentationAndNewlines() {
        let segments = MessageContentParser.segments(in: """
        - Before.
          ```json
          {"ok": true}
          ```
        - After.
        """)

        XCTAssertEqual(segments.map(\.kind), [.unorderedList, .code, .unorderedList])
        XCTAssertEqual(segments[0].text, "- Before.")
        XCTAssertEqual(segments[1].text, "{\"ok\": true}\n")
        XCTAssertEqual(segments[2].text, "- After.")
    }

    func testFencedCodeUsesMultiDigitListContentIndent() {
        let segments = MessageContentParser.segments(
            in: "10. Before.\n\n    ```json\n    {\"ok\": true}\n    ```"
        )

        XCTAssertEqual(segments.map(\.kind), [.orderedList, .code])
        XCTAssertEqual(segments[0].text, "10. Before.")
        XCTAssertEqual(segments[1].text, "{\"ok\": true}\n")
    }

    func testListFenceAllowsThreeSpacesBeyondContainerIndent() {
        let segments = MessageContentParser.segments(
            in: "- Before.\n\n     ```\n     indented child\n     ```"
        )

        XCTAssertEqual(segments.map(\.kind), [.unorderedList, .code])
        XCTAssertEqual(segments[1].text, "indented child\n")
    }

    func testEmptyFenceStillProducesAVisibleCodeBlock() {
        let segments = MessageContentParser.segments(in: "```\n```")

        XCTAssertEqual(segments.map(\.kind), [.code])
        XCTAssertEqual(segments.first?.text, "")
    }

    func testCRLFContentRecognizesHeadingsAndTables() {
        let segments = MessageContentParser.segments(
            in: "Intro.\r\n\r\n## Heading\r\n\r\n| A | B |\r\n| --- | --- |\r\n| 1 | 2 |"
        )

        XCTAssertEqual(segments.map(\.kind), [.prose, .heading(level: 2), .table])
        XCTAssertEqual(segments.map(\.text), [
            "Intro.",
            "Heading",
            "| A | B |\n| --- | --- |\n| 1 | 2 |",
        ])
    }

    func testSoftProseLineBreaksRenderAsSpacesButExplicitBreaksSurvive() {
        let segments = MessageContentParser.segments(
            in: "A wrapped\nparagraph.\nA hard break.  \nNext line."
        )

        XCTAssertEqual(segments.map(\.kind), [.prose])
        XCTAssertEqual(segments.first?.text, "A wrapped paragraph. A hard break.  \nNext line.")
    }

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
