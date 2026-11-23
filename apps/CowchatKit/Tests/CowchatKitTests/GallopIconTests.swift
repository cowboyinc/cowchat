import XCTest
@testable import CowchatKit

final class GallopIconTests: XCTestCase {
    /// Every declared icon must have a decodable SVG in the bundle — parity
    /// in BOTH directions is checked so a stray/missing file fails loudly.
    func testEveryIconResolvesFromBundle() {
        for icon in GallopIcon.allCases {
            XCTAssertNotNil(icon.image, "Missing or undecodable SVG for \(icon.rawValue)")
        }
    }

    /// The sidebar's online place icon and its two down states, from Gallop's Network family (navigation note,
    /// sections 3.4 and 3.11).
    func testTheNetworkIconsResolve() {
        let icons: [GallopIcon] = [.network, .networkDown, .networkError]
        XCTAssertEqual(icons.map(\.rawValue), ["network", "network-down", "network-error"])
        for icon in icons {
            XCTAssertNotNil(icon.image, icon.rawValue)
        }
    }

    /// The toolbar's sidebar button and the search's filter button (navigation round 2, sections 2 and 3.4). The old
    /// `sidebar` crop went with the button that was its only user.
    func testTheToolbarAndFilterIconsResolve() {
        let icons: [GallopIcon] = [.sidebarExpanded, .sidebarCollapsed, .settingsSlider]
        XCTAssertEqual(icons.map(\.rawValue), ["sidebar-expanded", "sidebar-collapsed", "settings-slider"])
        for icon in icons {
            XCTAssertNotNil(icon.image, icon.rawValue)
        }
        XCTAssertNil(GallopIcon(rawValue: "sidebar"))
    }

    /// The public call other clients use, given nothing but the package's own bundle: no app-owned icon catalog.
    func testEveryIconLoadsThroughThePublicCallFromThePackageAlone() throws {
        let bundle = try XCTUnwrap(
            SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates)
        )
        for icon in GallopIcon.allCases {
            let image = try XCTUnwrap(GallopIcon.nativeImage(for: icon, in: bundle), icon.rawValue)
            XCTAssertGreaterThan(image.size.width, 0, icon.rawValue)
            XCTAssertGreaterThan(image.size.height, 0, icon.rawValue)
        }
        XCTAssertEqual(GallopIcon.resolvedImages(in: bundle).count, GallopIcon.allCases.count)
        XCTAssertNil(GallopIcon.nativeImage(for: .add, in: nil), "no bundle is no image, never a trap")
    }

    #if os(macOS)
    /// iOS loads the icons from an asset catalog generated from the SVG files. The two must hold the same icons,
    /// byte for byte, or the platforms draw different things.
    func testTheIOSCatalogMatchesTheSVGFiles() throws {
        let bundled = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/CowchatKit/Bundled")
        let svgs = try FileManager.default.contentsOfDirectory(at: bundled.appendingPathComponent("Icons/svg"), includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "svg" }
        XCTAssertEqual(Set(svgs.map { $0.deletingPathExtension().lastPathComponent }), Set(GallopIcon.allCases.map(\.rawValue)))
        let catalog = bundled.appendingPathComponent("GallopIcons.xcassets")
        for svg in svgs {
            let name = svg.deletingPathExtension().lastPathComponent
            let copy = catalog.appendingPathComponent("\(name).imageset/\(name).svg")
            XCTAssertEqual(try Data(contentsOf: copy), try Data(contentsOf: svg), "run Scripts/build-gallop-icons.py: \(name) is out of date")
        }
        let sets = try FileManager.default.contentsOfDirectory(atPath: catalog.path).filter { $0.hasSuffix(".imageset") }
        XCTAssertEqual(sets.count, svgs.count, "run Scripts/build-gallop-icons.py: the catalog has an icon with no SVG")
    }
    #endif

    func testBundleHasNoOrphanSVGs() throws {
        let bundle = try XCTUnwrap(
            SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates)
        )
        let urls = bundle.urls(forResourcesWithExtension: "svg", subdirectory: "Icons/svg") ?? []
        let onDisk = Set(urls.map { $0.deletingPathExtension().lastPathComponent })
        XCTAssertEqual(onDisk, Set(GallopIcon.allCases.map(\.rawValue)))
    }
}
