import XCTest
@testable import CowchatMac

final class GallopThemeTests: XCTestCase {
    func testHexParserAcceptsGallopRGBAndRGBAForms() {
        XCTAssertEqual(
            GallopTheme.RGBA(hex: "#1D1916"),
            GallopTheme.RGBA(red: 0x1D, green: 0x19, blue: 0x16, alpha: 0xFF)
        )
        XCTAssertEqual(
            GallopTheme.RGBA(hex: "FFFFFF14"),
            GallopTheme.RGBA(red: 0xFF, green: 0xFF, blue: 0xFF, alpha: 0x14)
        )
    }

    func testHexParserRejectsMalformedValues() {
        XCTAssertNil(GallopTheme.RGBA(hex: "#FFF"))
        XCTAssertNil(GallopTheme.RGBA(hex: "#GGGGGG"))
        XCTAssertNil(GallopTheme.RGBA(hex: "#1122334455"))
        XCTAssertNil(GallopTheme.RGBA(hex: " #112233"))
    }

    func testSemanticTokenMetadataMatchesGallopSource() {
        assertToken(.surface300, name: "surface.300", light: "#F1EBE5", dark: "#000000")
        assertToken(.surface700, name: "surface.700", light: "#FFFFFF", dark: "#2E2824")
        assertToken(.surfaceGlass500, name: "surface.glass.500", light: "#FFFFFFCC", dark: "#2E2824CC")
        assertToken(.textPrimary, name: "text.primary", light: "#1D1916", dark: "#F4F0EB")
        assertToken(.textError, name: "text.error", light: "#B22D10", dark: "#E34F31")
        assertToken(.iconSecondary, name: "icon.secondary", light: "#4C433C", dark: "#CDBFB1")
        assertToken(.borderFocus, name: "border.focus", light: "#FF9D14", dark: "#FF9D14")
        assertToken(.buttonPrimaryPressed, name: "button.primary.pressed", light: "#E58200", dark: "#E58200")
        assertToken(.buttonSecondaryDefault, name: "button.secondary.default", light: "#EBE5DF", dark: "#2E2824")
        assertToken(.buttonGhostIconDefault, name: "button.ghost.icon.default", light: "#72675F", dark: "#988E86")
        assertToken(.textfieldDisabled, name: "textfield.disabled", light: "#F1EBE5", dark: "#0F0E0C")
        assertToken(.success, name: "status.success", light: "#29754A", dark: "#4BAA6E")
        assertToken(.warning, name: "status.warning", light: "#A85700", dark: "#FFAD33")

        XCTAssertEqual(GallopTheme.ColorToken.all.count, 77)
        XCTAssertEqual(Set(GallopTheme.ColorToken.all.map(\.name)).count, 77)
    }

    func testNativeChatTypographyMatchesGallopDesktopRoles() {
        let expected: [GallopTheme.TypeRole: ExpectedTypeRole] = [
            .h4: .init(family: .display, weight: 780, size: 20, lineHeight: 28, letterSpacing: 0.01),
            .h5: .init(family: .sans, weight: 750, size: 17, lineHeight: 24, letterSpacing: 0.02),
            .bodyL: .init(family: .sans, weight: 550, size: 16, lineHeight: 24, letterSpacing: 0.025),
            .bodyLStrong: .init(family: .sans, weight: 750, size: 16, lineHeight: 24, letterSpacing: 0.025),
            .bodyM: .init(family: .sans, weight: 550, size: 14, lineHeight: 20, letterSpacing: 0.03),
            .bodyMStrong: .init(family: .sans, weight: 750, size: 14, lineHeight: 20, letterSpacing: 0.03),
            .bodyS: .init(family: .sans, weight: 550, size: 13, lineHeight: 20, letterSpacing: 0.035),
            .bodySStrong: .init(family: .sans, weight: 750, size: 13, lineHeight: 20, letterSpacing: 0.035),
            .caption: .init(family: .sans, weight: 550, size: 12, lineHeight: 16, letterSpacing: 0.04),
            .dataLabel: .init(family: .sans, weight: 550, size: 10, lineHeight: 16, letterSpacing: 0.05),
            .code: .init(family: .mono, weight: 500, size: 14, lineHeight: 20, letterSpacing: -0.02),
        ]

        XCTAssertEqual(Set(expected.keys), Set(GallopTheme.TypeRole.allCases))
        for role in GallopTheme.TypeRole.allCases {
            let value = expected[role]!
            XCTAssertEqual(role.fontFamily, value.family, role.rawValue)
            XCTAssertEqual(role.fontWeight, value.weight, role.rawValue)
            XCTAssertEqual(role.fontSize, value.size, accuracy: 0.0001, role.rawValue)
            XCTAssertEqual(role.lineHeight, value.lineHeight, accuracy: 0.0001, role.rawValue)
            XCTAssertEqual(role.letterSpacing, value.letterSpacing, accuracy: 0.0001, role.rawValue)
            XCTAssertEqual(role.tracking, value.size * value.letterSpacing, accuracy: 0.0001, role.rawValue)
            XCTAssertGreaterThanOrEqual(role.lineSpacing, 0, role.rawValue)
            XCTAssertEqual(role.verticalPadding * 2, role.lineSpacing, accuracy: 0.0001, role.rawValue)
        }
    }

    private func assertToken(
        _ token: GallopTheme.ColorToken,
        name: String,
        light: String,
        dark: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(token.name, name, file: file, line: line)
        XCTAssertEqual(token.lightHex, light, file: file, line: line)
        XCTAssertEqual(token.darkHex, dark, file: file, line: line)
        XCTAssertEqual(token.hex(for: .light), light, file: file, line: line)
        XCTAssertEqual(token.hex(for: .dark), dark, file: file, line: line)
    }

    private struct ExpectedTypeRole {
        let family: GallopTheme.FontFamily
        let weight: Int
        let size: CGFloat
        let lineHeight: CGFloat
        let letterSpacing: CGFloat
    }
}
