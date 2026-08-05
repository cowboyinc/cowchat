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

    func testBundleHasNoOrphanSVGs() throws {
        let bundle = try XCTUnwrap(
            SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates)
        )
        let urls = bundle.urls(forResourcesWithExtension: "svg", subdirectory: "Icons/svg") ?? []
        let onDisk = Set(urls.map { $0.deletingPathExtension().lastPathComponent })
        XCTAssertEqual(onDisk, Set(GallopIcon.allCases.map(\.rawValue)))
    }
}
