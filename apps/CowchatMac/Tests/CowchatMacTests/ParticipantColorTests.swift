import XCTest
@testable import CowchatMac

final class ParticipantColorTests: XCTestCase {
    func testEverySwatchComesFromTheCowboyPalette() {
        let palette = Set(Palette.allTokens.map { $0.hex.uppercased() })
        for swatch in ParticipantColor.swatches {
            for hex in [swatch.lightFill, swatch.lightText, swatch.darkFill, swatch.darkText] {
                XCTAssertTrue(palette.contains(hex.uppercased()), "\(hex) is not a Gallop palette colour")
            }
        }
    }

    func testTheSwatchesAreDistinct() {
        let fills = ParticipantColor.swatches.map(\.lightFill)
        XCTAssertEqual(Set(fills).count, fills.count)
        XCTAssertGreaterThanOrEqual(fills.count, 6)
    }

    func testAParticipantKeepsItsColour() {
        for name in ["claude", "codex", "Ryan Chan", "hermes"] {
            XCTAssertEqual(ParticipantColor.index(for: name), ParticipantColor.index(for: name))
        }
    }

    /// A summed-scalar hash puts anagrams and near-identical agent names on the
    /// same colour; the room's agents are often exactly that ("claude-1", "claude-2").
    func testNumberedAgentsGetDifferentColours() {
        let names = (1...6).map { "claude-\($0)" }
        XCTAssertGreaterThanOrEqual(Set(names.map(ParticipantColor.index(for:))).count, 4)
        XCTAssertNotEqual(ParticipantColor.index(for: "abc"), ParticipantColor.index(for: "cba"))
    }

    func testEveryColourIsReachable() {
        let used = Set((0..<200).map { ParticipantColor.index(for: "agent-\($0)") })
        XCTAssertEqual(used, Set(ParticipantColor.swatches.indices))
    }

    /// Pinned so a hash change, which recolours every participant, is deliberate.
    func testIndicesArePinned() {
        XCTAssertEqual(ParticipantColor.index(for: "claude"), 3)
        XCTAssertEqual(ParticipantColor.index(for: "codex"), 0)
    }
}
