import AppKit
import SwiftUI

/// `NSCache` does not declare `Sendable`, even though this wrapper serializes
/// every access. Each entry records either an immutable decoded image or a
/// missing result, so failed resource lookups do not repeat on every render.
final class OriginalNativeImageCache: @unchecked Sendable {
    private final class Entry {
        let image: NSImage?

        init(image: NSImage?) {
            self.image = image
        }
    }

    private let cache = NSCache<NSString, Entry>()
    private let lock = NSLock()

    func image(for key: NSString, load: () -> NSImage?) -> NSImage? {
        lock.lock()
        defer { lock.unlock() }

        if let cached = cache.object(forKey: key) { return cached.image }

        let entry = Entry(image: load())
        cache.setObject(entry, forKey: key)
        return entry.image
    }
}

/// The Gallop brand icon set, shipped as SVGs in the Gallop resource bundle.
/// Raw values are the on-disk filenames (`Icons/svg/<rawValue>.svg`); the
/// parity test in `GallopIconsTests` keeps the two in step, since nothing
/// generates this enum.
public enum GallopIcon: String, CaseIterable, Sendable {
    case add
    case arrowUpRight = "arrow-up-right"
    case chevronDown = "chevron-down"
    /// Dash's 16pt Chat detail chevron, exported separately from the 24pt
    /// large variant because the paths are not scale-equivalent.
    case chevronDownExtraSmall = "chevron-down-extra-small"
    /// Dash's `Chevron` set draws each size separately rather than scaling one
    /// path, so the 20pt Small variant is not `chevron-down` rotated — it is a
    /// narrower glyph. `chevron-down` above is the 24pt Large variant; the
    /// size suffix here marks the two apart until the set is mirrored in full.
    case chevronRightSmall = "chevron-right-small"
    /// Dash's 16pt Chat detail chevron, exported separately from the 20pt
    /// small variant because the paths are not scale-equivalent.
    case chevronRightExtraSmall = "chevron-right-extra-small"
    /// Dash's 16pt Chat detail chevron, exported separately from the 24pt
    /// large variant because the paths are not scale-equivalent.
    case chevronUpExtraSmall = "chevron-up-extra-small"
    case copy
    case dismiss
    /// Dash's 16pt user-message edit glyph from Figma component `584:7533`.
    case edit
    case ellipsis
    case folder
    case lock
    case message
    case search
    case send
    case settings
    case sidebar
    /// Dash Chat's live-task indicator from Figma node `1762:71500`.
    case thinking
    case trash
    /// Dash Chat's terminal-run warning from Figma node `2086:16141`.
    case warning

    /// The icon as a tintable SwiftUI image, or nil when the resource bundle
    /// or the file inside it is absent. Callers own the fallback decision —
    /// resolution never traps, so a build that shipped without the bundle
    /// degrades instead of crashing (COW-2689).
    public var image: Image? { Self.images[self] }

    /// The original authored-colour image, for components such as Dash's
    /// status bar whose SVG encodes several semantic colours. Most callers
    /// should continue to use ``image`` so their glyph follows its context.
    /// Unlike the tintable collection, this is resolved lazily: the shell's
    /// always-visible avatar must not decode every authored-colour SVG at
    /// launch.
    public var originalImage: Image? {
        let key = rawValue as NSString
        guard let native = Self.originalNativeImageCache.image(for: key, load: {
            originalNativeImage(in: Self.resourceBundle)
        }) else {
            return nil
        }
        return Image(nsImage: native)
    }

    /// Resolved once per process. SwiftUI evaluates bodies constantly, so the
    /// SVG decode must not sit on that path.
    static let images: [GallopIcon: Image] = {
        resolvedImages(in: resourceBundle)
    }()

    private static let resourceBundle = SeasonFontProvider.resourceBundle(
        searching: SeasonFontProvider.resourceBundleCandidates
    )

    /// Kept apart from ``images`` because `NSImage.isTemplate` discards the
    /// authored colours needed by the network and gas-range status indicators.
    private static let originalNativeImageCache = OriginalNativeImageCache()

    static func resolvedImages(in bundle: Bundle?) -> [GallopIcon: Image] {
        var resolved: [GallopIcon: Image] = [:]
        for icon in allCases {
            if let native = icon.nativeImage(in: bundle) {
                resolved[icon] = Image(nsImage: native)
            }
        }
        return resolved
    }

    /// Loads the SVG as a template image so it adopts the surrounding
    /// foreground colour instead of painting its authored ink. nil — never a
    /// trap — for a missing bundle or a missing/undecodable file.
    static func nativeImage(for icon: GallopIcon, in bundle: Bundle?) -> NSImage? {
        icon.nativeImage(in: bundle)
    }

    private func nativeImage(in bundle: Bundle?) -> NSImage? {
        guard let url = bundle?.url(forResource: "Icons/svg/\(rawValue)", withExtension: "svg"),
              let image = NSImage(contentsOf: url)
        else { return nil }
        image.isTemplate = true
        return image
    }

    private func originalNativeImage(in bundle: Bundle?) -> NSImage? {
        guard let url = bundle?.url(forResource: "Icons/svg/\(rawValue)", withExtension: "svg"),
              let image = NSImage(contentsOf: url)
        else { return nil }
        return image
    }
}

/// Renders a Gallop icon at a fixed size, falling back to an SF Symbol when
/// the resource bundle is missing.
struct GallopIconView: View {
    let icon: GallopIcon
    let fallbackSystemName: String
    var size: CGFloat = 16

    var body: some View {
        if let image = icon.image {
            image.resizable().scaledToFit().frame(width: size, height: size)
        } else {
            Image(systemName: fallbackSystemName)
                .font(.system(size: size * 0.82, weight: .medium))
                .frame(width: size, height: size)
        }
    }
}
