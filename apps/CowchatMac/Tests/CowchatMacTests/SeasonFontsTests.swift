import AppKit
import XCTest
@testable import CowchatMac

final class SeasonFontsTests: XCTestCase {
    /// POSITIVE PATH (Codex review requirement): under `swift test` the bundle
    /// must actually resolve and the Season families must register — a broken
    /// bundle must fail tests, not silently fall back to system fonts.
    func testResourceBundleResolvesUnderSwiftTest() {
        XCTAssertNotNil(
            SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates),
            "CowchatMac_CowchatMac.bundle not found — check Package.swift .copy entries and candidates"
        )
    }

    func testSeasonFontsRegisterAndResolve() {
        XCTAssertTrue(SeasonFontProvider.fontsRegistered)
        let font = SeasonFontProvider.variableFont(family: "Season Sans VF", size: 16, weight: 550)
        XCTAssertEqual(font?.familyName, "Season Sans VF")
        let display = SeasonFontProvider.variableFont(family: "Season Mix VF", size: 20, weight: 780)
        XCTAssertEqual(display?.familyName, "Season Mix VF")
    }

    /// Fallback stays crash-free when the bundle is absent (COW-2689 lesson).
    func testMissingBundleFallsBackWithoutTrapping() {
        XCTAssertNil(SeasonFontProvider.resourceBundle(searching: [URL(fileURLWithPath: "/nonexistent")]))
        let style = GallopTextStyle.bodyM
        _ = SystemFontProvider().font(for: style) // must not trap
    }
}
