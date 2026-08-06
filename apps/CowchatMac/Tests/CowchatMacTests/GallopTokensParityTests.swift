import AppKit
import SwiftUI
import XCTest
@testable import CowchatMac

final class GallopTokensParityTests: XCTestCase {
    /// Resolves an adaptive SwiftUI Color to its sRGB hex under the given appearance.
    private func hex(_ color: Color, appearance: NSAppearance.Name) -> String {
        var resolved = ""
        NSAppearance(named: appearance)!.performAsCurrentDrawingAppearance {
            let ns = NSColor(color).usingColorSpace(.sRGB)!
            resolved = String(
                format: "#%02X%02X%02X%02X",
                Int(round(ns.redComponent * 255)),
                Int(round(ns.greenComponent * 255)),
                Int(round(ns.blueComponent * 255)),
                Int(round(ns.alphaComponent * 255))
            )
        }
        return resolved
    }

    private func assertToken(
        _ color: Color, light: String, dark: String,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        XCTAssertEqual(hex(color, appearance: .aqua), light, "light", file: file, line: line)
        XCTAssertEqual(hex(color, appearance: .darkAqua), dark, "dark", file: file, line: line)
    }

    /// Values pinned verbatim from ~/Github/macos Gallop/Generated/GallopTokens.swift
    /// (the Dash-reconciled set — see docs/dash-design-system.md in that repo).
    func testDashReconciledSpotValues() {
        assertToken(SemanticColor.surface500, light: "#F9F7F5FF", dark: "#1D1916FF")
        assertToken(SemanticColor.textPrimary, light: "#1D1916FF", dark: "#F4F0EBFF")
        assertToken(SemanticColor.borderDefault, light: "#D9CFC4FF", dark: "#3D3530FF")
        assertToken(SemanticColor.buttonPrimaryDefault, light: "#FF9D14FF", dark: "#FF9D14FF")
        // The Dash-reconciled glass value the old bridge got wrong (was #FFFFFFCC / #2E2824CC):
        assertToken(SemanticColor.surfaceGlass500, light: "#FCFAF8B8", dark: "#2E2824B8")
        assertToken(Palette.hay50, light: "#FCFAF8FF", dark: "#FCFAF8FF")
        assertToken(Palette.nugget500, light: "#FF9D14FF", dark: "#FF9D14FF")
        // Palette colors are non-adaptive (HexColor.color, not .adaptive), so light == dark.
        assertToken(Palette.cactus500, light: "#328F58FF", dark: "#328F58FF")
        // Regression: success/warning must stay adaptive, matching the old
        // bridge's exact values — a prior fix collapsed these to a single
        // static Palette alias, losing dark-mode contrast (Task 4 fix round 1).
        assertToken(SemanticColor.success, light: "#29754AFF", dark: "#4BAA6EFF")
        assertToken(SemanticColor.warning, light: "#A85700FF", dark: "#FFAD33FF")
    }

    func testTypeRoleTableMatchesGallop() {
        XCTAssertEqual(GallopTextStyle.h4.size, 20)
        XCTAssertEqual(GallopTextStyle.h4.family, .display)
        XCTAssertEqual(GallopTextStyle.bodyL.weight, 550)
        XCTAssertEqual(GallopTextStyle.bodyS.size, 13)
        XCTAssertEqual(GallopTextStyle.caption.size, 12)
    }

    /// Regression for Task 4 fix round 1: `NSColor(token)` alone stays
    /// dynamic even inside `performAsCurrentDrawingAppearance`, so the first
    /// implementation of ThemePreview.color silently re-resolved against
    /// whichever appearance was ACTUALLY active at draw time — both Settings
    /// theme-preview swatches followed the ambient appearance instead of
    /// showing one frozen light and one frozen dark. Assert the same
    /// resolved value under BOTH ambient appearances to prove freezing.
    func testThemePreviewColorFreezesAppearance() {
        assertToken(
            ThemePreview.color(SemanticColor.surface500, dark: false),
            light: "#F9F7F5FF", dark: "#F9F7F5FF"
        )
        assertToken(
            ThemePreview.color(SemanticColor.surface500, dark: true),
            light: "#1D1916FF", dark: "#1D1916FF"
        )
    }
}
