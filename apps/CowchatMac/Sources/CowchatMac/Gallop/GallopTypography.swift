import AppKit
import SwiftUI

public enum GallopFontFamily: String, Sendable {
    case display
    case sans
    case mono
}

/// One Gallop semantic type role (desktop values).
public struct GallopTextStyle: Sendable, Equatable {
    public let name: String
    public let family: GallopFontFamily
    /// Variable-font weight axis value as authored in Figma (e.g. 550, 780).
    public let weight: Int
    /// Font size in points.
    public let size: CGFloat
    /// Target line height in points.
    public let lineHeight: CGFloat
    /// Letter spacing in em (relative to size).
    public let letterSpacing: CGFloat

    public init(
        name: String,
        family: GallopFontFamily,
        weight: Int,
        size: CGFloat,
        lineHeight: CGFloat,
        letterSpacing: CGFloat
    ) {
        self.name = name
        self.family = family
        self.weight = weight
        self.size = size
        self.lineHeight = lineHeight
        self.letterSpacing = letterSpacing
    }

    /// Nearest system font weight for the variable axis value.
    public var fontWeight: Font.Weight {
        switch weight {
        case ..<450: .regular
        case ..<600: .medium
        case ..<700: .semibold
        case ..<760: .bold
        default: .heavy
        }
    }

    /// Letter spacing in points, as SwiftUI's `.tracking` expects.
    public var tracking: CGFloat { letterSpacing * size }

    /// Extra spacing needed to stretch the system font's natural line height to
    /// the Gallop line height. Kept for source compatibility; `.gallopText(_:)`
    /// uses the active provider's exact native-font metrics instead.
    public var lineSpacing: CGFloat {
        max(0, lineHeight - SystemFontProvider().naturalLineHeight(for: self))
    }
}

struct GallopLineLayout: Sendable, Equatable {
    let lineSpacing: CGFloat
    let verticalPadding: CGFloat

    init(targetLineHeight: CGFloat, naturalLineHeight: CGFloat) {
        let leading = targetLineHeight - naturalLineHeight
        // SwiftUI clamps negative line spacing before macOS 26. Preserve exact
        // positive leading and contract the outer line box as far as the legacy
        // renderer allows; macOS 26+ uses `.lineHeight(.exact(points:))` below.
        lineSpacing = max(0, leading)
        verticalPadding = leading / 2
    }
}

extension NSFont {
    var gallopNaturalLineHeight: CGFloat {
        // SwiftUI rounds a native font's line fragment up to a whole point.
        ceil(ascender - descender + leading)
    }
}

/// Maps a text style to a concrete font. The default is `SeasonFontProvider`
/// (bundled Season Sans/Mix); `SystemFontProvider` is its degradation path
/// and remains available for tests or explicit override via the environment.
public protocol GallopFontProvider: Sendable {
    func font(for style: GallopTextStyle) -> Font
    /// Italic or oblique counterpart to the font returned by `font(for:)`.
    func italicFont(for style: GallopTextStyle) -> Font
    /// Natural line height of the same native font returned by `font(for:)`.
    /// Custom providers should override the default so Gallop can reproduce
    /// its authored line boxes exactly.
    func naturalLineHeight(for style: GallopTextStyle) -> CGFloat
}

public extension GallopFontProvider {
    func italicFont(for style: GallopTextStyle) -> Font {
        font(for: style).italic()
    }

    func naturalLineHeight(for style: GallopTextStyle) -> CGFloat {
        SystemFontProvider().naturalLineHeight(for: style)
    }
}

public struct SystemFontProvider: GallopFontProvider {
    public init() {}

    public func font(for style: GallopTextStyle) -> Font {
        Font(nativeFont(for: style))
    }

    public func naturalLineHeight(for style: GallopTextStyle) -> CGFloat {
        nativeFont(for: style).gallopNaturalLineHeight
    }

    func nativeFont(for style: GallopTextStyle) -> NSFont {
        let weight: NSFont.Weight = switch style.weight {
        case ..<450: .regular
        case ..<600: .medium
        case ..<700: .semibold
        case ..<760: .bold
        default: .heavy
        }

        switch style.family {
        case .sans:
            return NSFont.systemFont(ofSize: style.size, weight: weight)
        case .mono:
            return NSFont.monospacedSystemFont(ofSize: style.size, weight: weight)
        case .display:
            let base = NSFont.systemFont(ofSize: style.size, weight: weight)
            guard let descriptor = base.fontDescriptor.withDesign(.serif),
                  let serif = NSFont(descriptor: descriptor, size: style.size)
            else { return base }
            return serif
        }
    }
}

private struct GallopFontProviderKey: EnvironmentKey {
    /// Season fonts by default; SeasonFontProvider itself degrades to
    /// SystemFontProvider when the bundled fonts are unavailable.
    static let defaultValue: any GallopFontProvider = SeasonFontProvider()
}

public extension EnvironmentValues {
    var gallopFontProvider: any GallopFontProvider {
        get { self[GallopFontProviderKey.self] }
        set { self[GallopFontProviderKey.self] = newValue }
    }
}

private struct GallopTextModifier: ViewModifier {
    let style: GallopTextStyle
    @Environment(\.gallopFontProvider) private var provider

    @ViewBuilder
    func body(content: Content) -> some View {
        #if compiler(>=6.2)
        if #available(macOS 26.0, *) {
            content
                .font(provider.font(for: style))
                .tracking(style.tracking)
                .lineHeight(.exact(points: style.lineHeight))
        } else {
            legacyBody(content: content)
        }
        #else
        legacyBody(content: content)
        #endif
    }

    private func legacyBody(content: Content) -> some View {
        let layout = GallopLineLayout(
            targetLineHeight: style.lineHeight,
            naturalLineHeight: provider.naturalLineHeight(for: style)
        )

        return content
            .font(provider.font(for: style))
            .tracking(style.tracking)
            .lineSpacing(layout.lineSpacing)
            .padding(.vertical, layout.verticalPadding)
    }
}

public extension View {
    /// Applies a Gallop type role: font, weight, tracking, and exact line box.
    func gallopText(_ style: GallopTextStyle) -> some View {
        modifier(GallopTextModifier(style: style))
    }
}
