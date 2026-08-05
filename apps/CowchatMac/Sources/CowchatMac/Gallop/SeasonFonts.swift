import AppKit
import CoreText
import SwiftUI

/// Anchor for `Bundle(for:)` so resource-bundle resolution tracks wherever
/// the Gallop code is actually loaded (the .xctest product under
/// `swift test`, the executable otherwise).
private final class GallopBundleFinder {}

/// Resolves Gallop text styles to the bundled Season variable fonts —
/// Season Sans VF (`sans`) and Season Mix VF (`display`), each with a
/// wght axis of 300–900 so Gallop's exact Figma weights apply without
/// snapping to system weights. `mono` intentionally stays with the system
/// provider (Gallop specifies SF Mono). Falls back to `SystemFontProvider`
/// entirely if the bundled fonts fail to register.
public struct SeasonFontProvider: GallopFontProvider {
    static let familyNames: [GallopFontFamily: String] = [
        .sans: "Season Sans VF",
        .display: "Season Mix VF",
    ]

    /// The wght axis range authored in the font files; values outside it
    /// resolve unpredictably, so requested weights are clamped.
    static let weightAxisRange = 300...900
    private static let weightAxisID = 0x77676874 // 'wght'
    /// Matches the conventional synthetic oblique used when a web font family
    /// ships only upright faces and CSS requests `font-style: italic`.
    private static let syntheticObliqueShear = CGFloat(tan(12 * Double.pi / 180))

    /// Name SwiftPM gives the Gallop target's resource bundle
    /// (`<package>_<target>.bundle` — keep in sync with `Package.swift`).
    static let resourceBundleName = "CowboyDesktopNative_Gallop.bundle"

    /// Directories that may hold the resource bundle, in lookup order: a
    /// packaged .app's Contents/Resources (both release scripts and Xcode
    /// place it there), the directory of a bare `swift build` executable,
    /// and the parent of the .xctest product (SwiftPM leaves the bundle
    /// beside it under `swift test`).
    ///
    /// This deliberately replaces the synthesized `Bundle.module`, which
    /// probes only the .app root — where nothing can live without breaking
    /// the codesign seal — plus the build machine's scratch path, and
    /// fatalErrors on a miss: v0.1.33 crashed at launch this way (COW-2689).
    static var resourceBundleCandidates: [URL] {
        let finder = Bundle(for: GallopBundleFinder.self)
        let candidates: [URL?] = [
            Bundle.main.resourceURL,
            Bundle.main.bundleURL,
            finder.resourceURL,
            finder.bundleURL.deletingLastPathComponent(),
        ]
        return candidates.compactMap { $0 }
    }

    /// First candidate directory that actually contains the resource
    /// bundle; nil — never a trap — when the bundle didn't ship, so
    /// rendering degrades to system fonts.
    static func resourceBundle(searching candidates: [URL]) -> Bundle? {
        for candidate in candidates {
            if let bundle = Bundle(url: candidate.appendingPathComponent(resourceBundleName)) {
                return bundle
            }
        }
        return nil
    }

    /// One-time, process-scoped registration of the bundled TTFs.
    /// "Already registered" outcomes count as success so tests and the app
    /// can coexist in one process.
    static let fontsRegistered: Bool = {
        guard let bundle = resourceBundle(searching: resourceBundleCandidates) else {
            return false
        }
        let names = ["SeasonSansUprightsVF", "SeasonMixUprightsVF"]
        let urls = names.compactMap {
            bundle.url(forResource: "Fonts/\($0)", withExtension: "ttf")
        }
        guard urls.count == names.count else { return false }
        return urls.allSatisfy { url in
            var error: Unmanaged<CFError>?
            if CTFontManagerRegisterFontsForURL(url as CFURL, .process, &error) {
                return true
            }
            let code = (error?.takeRetainedValue()).map(CFErrorGetCode)
            return code == CTFontManagerError.alreadyRegistered.rawValue
                || code == CTFontManagerError.duplicatedName.rawValue
        }
    }()

    private let fallback = SystemFontProvider()

    public init() {}

    public func font(for style: GallopTextStyle) -> Font {
        Font(nativeFont(for: style))
    }

    public func italicFont(for style: GallopTextStyle) -> Font {
        guard let font = seasonFont(for: style) else {
            return fallback.italicFont(for: style)
        }
        return Font(Self.syntheticObliqueFont(from: font))
    }

    public func naturalLineHeight(for style: GallopTextStyle) -> CGFloat {
        nativeFont(for: style).gallopNaturalLineHeight
    }

    private func nativeFont(for style: GallopTextStyle) -> NSFont {
        seasonFont(for: style) ?? fallback.nativeFont(for: style)
    }

    private func seasonFont(for style: GallopTextStyle) -> NSFont? {
        guard Self.fontsRegistered,
              let family = Self.familyNames[style.family],
              let font = Self.variableFont(family: family, size: style.size, weight: style.weight)
        else { return nil }
        return font
    }

    static func syntheticObliqueFont(from font: NSFont) -> NSFont {
        var transform = CGAffineTransform(
            a: 1,
            b: 0,
            c: syntheticObliqueShear,
            d: 1,
            tx: 0,
            ty: 0
        )
        return CTFontCreateCopyWithAttributes(font as CTFont, 0, &transform, nil) as NSFont
    }

    /// Instantiates the variable font at an exact wght axis value.
    static func variableFont(family: String, size: CGFloat, weight: Int) -> NSFont? {
        // This helper is also used directly by tests and future typography
        // adapters. Do not rely on another call having initialized the
        // process-scoped registration first.
        guard fontsRegistered else { return nil }

        let clamped = min(max(weight, weightAxisRange.lowerBound), weightAxisRange.upperBound)
        let descriptor = NSFontDescriptor(fontAttributes: [
            .family: family,
            .variation: [NSNumber(value: weightAxisID): NSNumber(value: clamped)],
        ])
        guard let font = NSFont(descriptor: descriptor, size: size),
              font.familyName == family
        else {
            // NSFont(descriptor:) falls back to a default font for unknown
            // families instead of returning nil — verify we got the real one.
            return nil
        }
        return font
    }
}
