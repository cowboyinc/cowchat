import AppKit
import SwiftUI

/// Native Cowchat bridge for Gallop's semantic design tokens.
///
/// Source of truth: `gallop/packages/foundations/src/{colors,typography}.ts`
/// at Gallop commit `121e38465ef5696bf27ed332a573347e4ae87959`.
enum GallopTheme {
    enum Appearance: Sendable {
        case light
        case dark
    }

    /// Integer color components keep parsing and token metadata deterministic.
    struct RGBA: Equatable, Sendable {
        let red: UInt8
        let green: UInt8
        let blue: UInt8
        let alpha: UInt8

        init(red: UInt8, green: UInt8, blue: UInt8, alpha: UInt8) {
            self.red = red
            self.green = green
            self.blue = blue
            self.alpha = alpha
        }

        init?(hex: String) {
            let digits = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
            guard digits.count == 6 || digits.count == 8,
                  let value = UInt64(digits, radix: 16) else { return nil }

            if digits.count == 6 {
                red = UInt8((value >> 16) & 0xFF)
                green = UInt8((value >> 8) & 0xFF)
                blue = UInt8(value & 0xFF)
                alpha = 0xFF
            } else {
                red = UInt8((value >> 24) & 0xFF)
                green = UInt8((value >> 16) & 0xFF)
                blue = UInt8((value >> 8) & 0xFF)
                alpha = UInt8(value & 0xFF)
            }
        }

        var nsColor: NSColor {
            NSColor(
                srgbRed: CGFloat(red) / 255,
                green: CGFloat(green) / 255,
                blue: CGFloat(blue) / 255,
                alpha: CGFloat(alpha) / 255
            )
        }
    }

    /// One semantic color with values for both macOS appearances.
    struct ColorToken: Equatable, Sendable {
        let name: String
        let lightHex: String
        let darkHex: String

        fileprivate init(_ name: String, light: String, dark: String) {
            precondition(RGBA(hex: light) != nil, "Invalid light color for \(name): \(light)")
            precondition(RGBA(hex: dark) != nil, "Invalid dark color for \(name): \(dark)")
            self.name = name
            lightHex = light.uppercased()
            darkHex = dark.uppercased()
        }

        func hex(for appearance: Appearance) -> String {
            appearance == .dark ? darkHex : lightHex
        }

        func rgba(for appearance: Appearance) -> RGBA {
            // Every token is validated at construction.
            RGBA(hex: hex(for: appearance))!
        }

        /// AppKit color that follows the effective Aqua/Dark Aqua appearance.
        var nsColor: NSColor {
            NSColor(name: NSColor.Name("Gallop.\(name)")) { appearance in
                let mode: Appearance = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                    ? .dark
                    : .light
                return rgba(for: mode).nsColor
            }
        }

        /// SwiftUI color backed by the same adaptive AppKit provider.
        var color: Color {
            Color(nsColor: nsColor)
        }

        // MARK: Surface

        static let surface300 = token("surface.300", "#F1EBE5", "#000000")
        static let surface400 = token("surface.400", "#F4F0EB", "#0F0E0C")
        static let surface500 = token("surface.500", "#F9F7F5", "#1D1916")
        static let surface600 = token("surface.600", "#FDFCFB", "#26211D")
        static let surface700 = token("surface.700", "#FFFFFF", "#2E2824")

        // MARK: Glass surface

        static let surfaceGlass500 = token("surface.glass.500", "#FFFFFFCC", "#2E2824CC")
        static let surfaceGlassBorderHighlight = token("surface.glass.border.highlight", "#FFFFFFCC", "#FFFFFF14")
        static let surfaceGlassBorderShadow = token("surface.glass.border.shadow", "#2E282414", "#0F0E0CA3")
        static let surfaceGlassOffHover = token("surface.glass.off.hover", "#2E28240A", "#0F0E0C29")
        static let surfaceGlassOffPressed = token("surface.glass.off.pressed", "#2E282414", "#0F0E0C3D")
        static let surfaceGlassOnDefault = token("surface.glass.on.default", "#FFF7EB", "#291400")
        static let surfaceGlassOnHover = token("surface.glass.on.hover", "#FFF7EB", "#291400")
        static let surfaceGlassOnPressed = token("surface.glass.on.pressed", "#2E282414", "#0F0E0C3D")
        static let surfaceGlassOffTextDefault = token("surface.glass.off.text.default", "#3D3530", "#D3C7BB")
        static let surfaceGlassOffTextHover = token("surface.glass.off.text.hover", "#1D1916", "#F4F0EB")
        static let surfaceGlassOffTextPressed = token("surface.glass.off.text.pressed", "#1D1916", "#D3C7BB")
        static let surfaceGlassOffIconDefault = token("surface.glass.off.icon.default", "#4C433C", "#CDBFB1")
        static let surfaceGlassOffIconHover = token("surface.glass.off.icon.hover", "#26211D", "#F1EBE5")
        static let surfaceGlassOffIconPressed = token("surface.glass.off.icon.pressed", "#26211D", "#F1EBE5")
        static let surfaceGlassOnTextDefault = token("surface.glass.on.text.default", "#A85700", "#FFAD33")
        static let surfaceGlassOnTextHover = token("surface.glass.on.text.hover", "#A85700", "#FFAD33")

        // MARK: Text and icon

        static let textPrimary = token("text.primary", "#1D1916", "#F4F0EB")
        static let textSecondary = token("text.secondary", "#3D3530", "#D3C7BB")
        static let textTertiary = token("text.tertiary", "#72675F", "#AEA399")
        static let textError = token("text.error", "#B22D10", "#E34F31")
        static let textDisabled = token("text.disabled", "#988E86", "#4C433C")

        static let iconPrimary = token("icon.primary", "#26211D", "#F1EBE5")
        static let iconSecondary = token("icon.secondary", "#4C433C", "#CDBFB1")
        static let iconTertiary = token("icon.tertiary", "#72675F", "#988E86")
        static let iconError = token("icon.error", "#CD3214", "#CD3214")
        static let iconDisabled = token("icon.disabled", "#AEA399", "#3D3530")
        static let iconSubtle = token("icon.subtle", "#AEA399", "#3D3530")

        // MARK: Border

        static let borderDefault = token("border.default", "#D9CFC4", "#3D3530")
        static let borderHover = token("border.hover", "#CDBFB1", "#4C433C")
        static let borderPressed = token("border.pressed", "#988E86", "#26211D")
        static let borderFocus = token("border.focus", "#FF9D14", "#FF9D14")
        static let borderAccessibleDefault = token("border.accessible.default", "#988E86", "#72675F")
        static let borderAccessibleHover = token("border.accessible.hover", "#8A7F76", "#8A7F76")
        static let borderAccessiblePressed = token("border.accessible.pressed", "#72675F", "#61574F")
        static let borderError = token("border.error", "#CD3214", "#CD3214")
        static let borderDisabled = token("border.disabled", "#E6DED6", "#2E2824")

        // MARK: Button fills

        static let buttonPrimaryDefault = token("button.primary.default", "#FF9D14", "#FF9D14")
        static let buttonPrimaryHover = token("button.primary.hover", "#FFAD33", "#FFAD33")
        static let buttonPrimaryPressed = token("button.primary.pressed", "#E58200", "#E58200")
        static let buttonPrimaryDisabled = token("button.primary.disabled", "#FF9D14", "#FF9D14")
        static let buttonSecondaryDefault = token("button.secondary.default", "#EBE5DF", "#2E2824")
        static let buttonSecondaryHover = token("button.secondary.hover", "#F1EBE5", "#3D3530")
        static let buttonSecondaryPressed = token("button.secondary.pressed", "#E6DED6", "#26211D")
        static let buttonSecondaryDisabled = token("button.secondary.disabled", "#EBE5DF", "#2E2824")
        static let buttonGhostHover = token("button.ghost.hover", "#F1EBE5", "#3D3530")
        static let buttonGhostPressed = token("button.ghost.pressed", "#EBE5DF", "#2E2824")

        // MARK: Button content

        static let buttonPrimaryTextDefault = token("button.primary.text.default", "#512900", "#512900")
        static let buttonPrimaryTextHover = token("button.primary.text.hover", "#783E00", "#783E00")
        static let buttonPrimaryTextPressed = token("button.primary.text.pressed", "#291400", "#291400")
        static let buttonPrimaryTextDisabled = token("button.primary.text.disabled", "#512900", "#512900")
        static let buttonPrimaryIconDefault = token("button.primary.icon.default", "#783E00", "#783E00")
        static let buttonPrimaryIconHover = token("button.primary.icon.hover", "#A85700", "#A85700")
        static let buttonPrimaryIconPressed = token("button.primary.icon.pressed", "#512900", "#512900")
        static let buttonPrimaryIconDisabled = token("button.primary.icon.disabled", "#783E00", "#783E00")

        static let buttonSecondaryTextDefault = token("button.secondary.text.default", "#3D3530", "#D3C7BB")
        static let buttonSecondaryTextHover = token("button.secondary.text.hover", "#4C433C", "#D9CFC4")
        static let buttonSecondaryTextPressed = token("button.secondary.text.pressed", "#1D1916", "#AEA399")
        static let buttonSecondaryTextDisabled = token("button.secondary.text.disabled", "#72675F", "#AEA399")
        static let buttonSecondaryIconDefault = token("button.secondary.icon.default", "#4C433C", "#CDBFB1")
        static let buttonSecondaryIconHover = token("button.secondary.icon.hover", "#61574F", "#D3C7BB")
        static let buttonSecondaryIconPressed = token("button.secondary.icon.pressed", "#26211D", "#988E86")
        static let buttonSecondaryIconDisabled = token("button.secondary.icon.disabled", "#72675F", "#988E86")

        static let buttonGhostIconDefault = token("button.ghost.icon.default", "#72675F", "#988E86")
        static let buttonGhostIconHover = token("button.ghost.icon.hover", "#8A7F76", "#AEA399")
        static let buttonGhostIconPressed = token("button.ghost.icon.pressed", "#26211D", "#988E86")
        static let buttonGhostIconDisabled = token("button.ghost.icon.disabled", "#72675F", "#988E86")

        // MARK: Text field and chat status

        static let textfieldDefault = token("textfield.default", "#F9F7F5", "#1D1916")
        static let textfieldHover = token("textfield.hover", "#FDFCFB", "#26211D")
        static let textfieldFocus = token("textfield.focus", "#FDFCFB", "#26211D")
        static let textfieldDisabled = token("textfield.disabled", "#F1EBE5", "#0F0E0C")

        // Gallop currently supplies status hues at the palette layer. These
        // bridge roles follow text.error's 600 (light) / 400 (dark) contrast.
        static let success = token("status.success", "#29754A", "#4BAA6E")
        static let warning = token("status.warning", "#A85700", "#FFAD33")

        /// Every bridge token, useful for metadata/parity tests.
        static let all: [ColorToken] = [
            surface300, surface400, surface500, surface600, surface700,
            surfaceGlass500, surfaceGlassBorderHighlight, surfaceGlassBorderShadow,
            surfaceGlassOffHover, surfaceGlassOffPressed, surfaceGlassOnDefault,
            surfaceGlassOnHover, surfaceGlassOnPressed, surfaceGlassOffTextDefault,
            surfaceGlassOffTextHover, surfaceGlassOffTextPressed,
            surfaceGlassOffIconDefault, surfaceGlassOffIconHover,
            surfaceGlassOffIconPressed, surfaceGlassOnTextDefault, surfaceGlassOnTextHover,
            textPrimary, textSecondary, textTertiary, textError, textDisabled,
            iconPrimary, iconSecondary, iconTertiary, iconError, iconDisabled, iconSubtle,
            borderDefault, borderHover, borderPressed, borderFocus,
            borderAccessibleDefault, borderAccessibleHover, borderAccessiblePressed,
            borderError, borderDisabled,
            buttonPrimaryDefault, buttonPrimaryHover, buttonPrimaryPressed, buttonPrimaryDisabled,
            buttonSecondaryDefault, buttonSecondaryHover, buttonSecondaryPressed, buttonSecondaryDisabled,
            buttonGhostHover, buttonGhostPressed,
            buttonPrimaryTextDefault, buttonPrimaryTextHover, buttonPrimaryTextPressed,
            buttonPrimaryTextDisabled, buttonPrimaryIconDefault, buttonPrimaryIconHover,
            buttonPrimaryIconPressed, buttonPrimaryIconDisabled,
            buttonSecondaryTextDefault, buttonSecondaryTextHover, buttonSecondaryTextPressed,
            buttonSecondaryTextDisabled, buttonSecondaryIconDefault, buttonSecondaryIconHover,
            buttonSecondaryIconPressed, buttonSecondaryIconDisabled,
            buttonGhostIconDefault, buttonGhostIconHover, buttonGhostIconPressed,
            buttonGhostIconDisabled,
            textfieldDefault, textfieldHover, textfieldFocus, textfieldDisabled,
            success, warning,
        ]

        private static func token(_ name: String, _ light: String, _ dark: String) -> ColorToken {
            ColorToken(name, light: light, dark: dark)
        }
    }

    enum FontFamily: String, Sendable {
        case display
        case sans
        case mono
    }

    struct TextStyle: Equatable, Sendable {
        let name: String
        let family: FontFamily
        let fontWeight: Int
        let fontSize: CGFloat
        let lineHeight: CGFloat
        let letterSpacing: CGFloat

        /// SwiftUI tracking is in points; Gallop stores letter spacing in em.
        var tracking: CGFloat {
            fontSize * letterSpacing
        }

        /// Extra leading needed to reach Gallop's target line height.
        var lineSpacing: CGFloat {
            max(0, lineHeight - naturalLineHeight)
        }

        /// Balances the extra leading above and below a single line. Combined
        /// with `lineSpacing`, this reproduces the target box for multiline text.
        var verticalPadding: CGFloat {
            lineSpacing / 2
        }

        var font: Font { Font(nsFont) }

        private var naturalLineHeight: CGFloat {
            // SwiftUI rounds native line fragments up to a whole point.
            ceil(nsFont.ascender - nsFont.descender + nsFont.leading)
        }

        private var nsFont: NSFont {
            switch family {
            case .display:
                let base = NSFont.systemFont(ofSize: fontSize, weight: appKitWeight)
                guard let descriptor = base.fontDescriptor.withDesign(.serif),
                      let serif = NSFont(descriptor: descriptor, size: fontSize)
                else { return base }
                return serif
            case .sans:
                return .systemFont(ofSize: fontSize, weight: appKitWeight)
            case .mono:
                return .monospacedSystemFont(ofSize: fontSize, weight: appKitWeight)
            }
        }

        /// Interpolated AppKit weights retain Gallop's variable-weight intent as
        /// closely as the installed system font permits.
        private var appKitWeight: NSFont.Weight {
            switch fontWeight {
            case 500: return .medium
            case 550: return NSFont.Weight(rawValue: 0.265)
            case 750: return NSFont.Weight(rawValue: 0.48)
            case 780: return NSFont.Weight(rawValue: 0.528)
            default: return .regular
            }
        }
    }

    enum TypeRole: String, CaseIterable, Sendable {
        case h4
        case h5
        case bodyL = "body-l"
        case bodyLStrong = "body-l-strong"
        case bodyM = "body-m"
        case bodyMStrong = "body-m-strong"
        case bodyS = "body-s"
        case bodySStrong = "body-s-strong"
        case caption
        case dataLabel = "data-label"
        case code

        var style: TextStyle {
            switch self {
            case .h4: return TextStyle(name: rawValue, family: .display, fontWeight: 780, fontSize: 20, lineHeight: 28, letterSpacing: 0.01)
            case .h5: return TextStyle(name: rawValue, family: .sans, fontWeight: 750, fontSize: 17, lineHeight: 24, letterSpacing: 0.02)
            case .bodyL: return TextStyle(name: rawValue, family: .sans, fontWeight: 550, fontSize: 16, lineHeight: 24, letterSpacing: 0.025)
            case .bodyLStrong: return TextStyle(name: rawValue, family: .sans, fontWeight: 750, fontSize: 16, lineHeight: 24, letterSpacing: 0.025)
            case .bodyM: return TextStyle(name: rawValue, family: .sans, fontWeight: 550, fontSize: 14, lineHeight: 20, letterSpacing: 0.03)
            case .bodyMStrong: return TextStyle(name: rawValue, family: .sans, fontWeight: 750, fontSize: 14, lineHeight: 20, letterSpacing: 0.03)
            case .bodyS: return TextStyle(name: rawValue, family: .sans, fontWeight: 550, fontSize: 13, lineHeight: 20, letterSpacing: 0.035)
            case .bodySStrong: return TextStyle(name: rawValue, family: .sans, fontWeight: 750, fontSize: 13, lineHeight: 20, letterSpacing: 0.035)
            case .caption: return TextStyle(name: rawValue, family: .sans, fontWeight: 550, fontSize: 12, lineHeight: 16, letterSpacing: 0.04)
            case .dataLabel: return TextStyle(name: rawValue, family: .sans, fontWeight: 550, fontSize: 10, lineHeight: 16, letterSpacing: 0.05)
            case .code: return TextStyle(name: rawValue, family: .mono, fontWeight: 500, fontSize: 14, lineHeight: 20, letterSpacing: -0.02)
            }
        }

        var fontFamily: FontFamily { style.family }
        var fontWeight: Int { style.fontWeight }
        var fontSize: CGFloat { style.fontSize }
        var lineHeight: CGFloat { style.lineHeight }
        var letterSpacing: CGFloat { style.letterSpacing }
        var tracking: CGFloat { style.tracking }
        var lineSpacing: CGFloat { style.lineSpacing }
        var verticalPadding: CGFloat { style.verticalPadding }
        var font: Font { style.font }
    }
}

private struct GallopTextModifier: ViewModifier {
    let role: GallopTheme.TypeRole
    let color: GallopTheme.ColorToken?

    @ViewBuilder
    func body(content: Content) -> some View {
        if let color {
            styled(content)
                .foregroundStyle(color.color)
        } else {
            styled(content)
        }
    }

    private func styled(_ content: Content) -> some View {
        content
            .font(role.font)
            .tracking(role.tracking)
            .lineSpacing(role.lineSpacing)
            .padding(.vertical, role.verticalPadding)
    }
}

extension View {
    /// Applies a Gallop type role and, when supplied, a semantic text color.
    func gallopText(
        _ role: GallopTheme.TypeRole,
        color: GallopTheme.ColorToken? = nil
    ) -> some View {
        modifier(GallopTextModifier(role: role, color: color))
    }
}
