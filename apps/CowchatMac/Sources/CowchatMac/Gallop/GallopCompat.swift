import AppKit
import SwiftUI

extension View {
    /// Cowchat's historical call-site shape: role + optional semantic color.
    /// Routes through the vendored Gallop modifier for font metrics.
    func gallopText(_ style: GallopTextStyle, color: Color?) -> some View {
        Group {
            if let color {
                gallopText(style).foregroundStyle(color)
            } else {
                gallopText(style)
            }
        }
    }
}

extension SemanticColor {
    /// Cowchat-local status roles (cowboy app vocabulary: cactus = live/ok, nugget700 = issues).
    /// The old bridge's warning (#A85700) IS nugget700; success moves from #29754A to cactus500.
    static let success = Palette.cactus500
    static let warning = Palette.nugget700
}

extension SemanticColor {
    /// AppKit counterparts for the tokens `ComposerTextField` needs on its
    /// `NSTextField` (NSColor, not Color) — the vendored layer only publishes
    /// SwiftUI `Color`. These are assigned once to `field.textColor` /
    /// placeholder attributes rather than read every render, so — like the
    /// old bridge's per-token `.nsColor` they replace — they must stay
    /// genuinely dynamic (re-resolved by AppKit on every appearance change),
    /// not a one-shot snapshot of whichever appearance was active at
    /// construction time. Looked up from `allTokens` so a future Dash sync
    /// updates the SwiftUI and AppKit values together.
    enum AppKitColor {
        static var textPrimary: NSColor { resolve("textPrimary") }
        static var textTertiary: NSColor { resolve("textTertiary") }

        private static func resolve(_ tokenName: String) -> NSColor {
            guard let token = SemanticColor.allTokens.first(where: { $0.name == tokenName }) else {
                preconditionFailure("Unknown SemanticColor token \"\(tokenName)\"")
            }
            return NSColor(name: nil) { appearance in
                HexColor.nsColor(HexColor.isDark(appearance) ? token.dark : token.light)
            }
        }
    }
}
