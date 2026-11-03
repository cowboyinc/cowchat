import XCTest
@testable import CowchatMac

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

    func testBundleHasNoOrphanSVGs() throws {
        let bundle = try XCTUnwrap(
            SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates)
        )
        let urls = bundle.urls(forResourcesWithExtension: "svg", subdirectory: "Icons/svg") ?? []
        let onDisk = Set(urls.map { $0.deletingPathExtension().lastPathComponent })
        XCTAssertEqual(onDisk, Set(GallopIcon.allCases.map(\.rawValue)))
    }
}
