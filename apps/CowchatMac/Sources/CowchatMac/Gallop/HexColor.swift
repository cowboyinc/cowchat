import AppKit
import SwiftUI

/// Turns generated hex-string tokens into SwiftUI colors. Generated values are
/// validated by GallopTests, so the public entry points are non-failable; only
/// the parser itself is failable, for direct testing.
public enum HexColor {
    /// Parses `#RRGGBB` or `#RRGGBBAA` into sRGB components.
    static func components(_ hex: String) -> (r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat)? {
        guard hex.hasPrefix("#") else { return nil }
        let digits = hex.dropFirst()
        guard digits.count == 6 || digits.count == 8,
              digits.allSatisfy(\.isHexDigit),
              let value = UInt64(digits, radix: 16)
        else { return nil }
        var rgb = value
        var alpha: CGFloat = 1
        if digits.count == 8 {
            alpha = CGFloat(value & 0xFF) / 255
            rgb >>= 8
        }
        return (
            r: CGFloat((rgb >> 16) & 0xFF) / 255,
            g: CGFloat((rgb >> 8) & 0xFF) / 255,
            b: CGFloat(rgb & 0xFF) / 255,
            a: alpha
        )
    }

    static func nsColor(_ hex: String) -> NSColor {
        guard let c = components(hex) else {
            preconditionFailure("Invalid generated hex token \"\(hex)\"")
        }
        return NSColor(srgbRed: c.r, green: c.g, blue: c.b, alpha: c.a)
    }

    static func isDark(_ appearance: NSAppearance) -> Bool {
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
    }

    public static func color(_ hex: String) -> Color {
        Color(nsColor: nsColor(hex))
    }

    /// A color that resolves per system appearance. Hex strings (not NSColors)
    /// are captured so the dynamic provider closure stays Sendable-safe.
    public static func adaptive(light: String, dark: String) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            isDark(appearance) ? nsColor(dark) : nsColor(light)
        })
    }
}
