import XCTest
@testable import CowchatMac

final class RoomNameSuggestionTests: XCTestCase {
    func testSuggestionIsThreeKnownWordsJoinedByHyphens() {
        for _ in 0..<50 {
            let suggestion = RoomNameSuggestion.generate()
            let parts = suggestion.split(separator: "-").map(String.init)
            XCTAssertEqual(parts.count, 3, "unexpected shape: \(suggestion)")
            XCTAssertTrue(RoomNameSuggestion.animals.contains(parts[0]))
            XCTAssertTrue(RoomNameSuggestion.colors.contains(parts[1]))
            XCTAssertTrue(RoomNameSuggestion.nouns.contains(parts[2]))
        }
    }

    func testSuggestionsPassRoomNameValidation() {
        for _ in 0..<50 {
            let suggestion = RoomNameSuggestion.generate()
            XCTAssertFalse(suggestion.isEmpty)
            XCTAssertLessThanOrEqual(suggestion.unicodeScalars.count, 100)
            XCTAssertFalse(
                suggestion.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
            )
            XCTAssertNotEqual(suggestion.localizedCaseInsensitiveCompare("lobby"), .orderedSame)
        }
    }

    func testSeededGeneratorIsDeterministic() {
        var first = SeededGenerator(seed: 42)
        var second = SeededGenerator(seed: 42)
        XCTAssertEqual(
            RoomNameSuggestion.generate(using: &first),
            RoomNameSuggestion.generate(using: &second)
        )
    }
}

/// SplitMix64 — deterministic RNG for tests.
private struct SeededGenerator: RandomNumberGenerator {
    private var state: UInt64

    init(seed: UInt64) { state = seed }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}
