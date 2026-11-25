import CoreText
import SwiftUI
import UIKit
import XCTest
@testable import CowchatiOS
import CowchatKit

final class GallopTests: XCTestCase {
    func testSeasonFontsRegister() {
        XCTAssertTrue(SeasonFontProvider.fontsRegistered)
    }

    func testGallopRolesResolveToSeasonFacesNotTheSystemFont() {
        let provider = SeasonFontProvider()
        let body = provider.nativeFont(for: .bodyL)
        let heading = provider.nativeFont(for: .h3)
        XCTAssertEqual(body.familyName, "Season Sans VF")
        XCTAssertEqual(heading.familyName, "Season Mix VF")
        XCTAssertNotEqual(body.familyName, SystemFontProvider().nativeFont(for: .bodyL).familyName)
    }

    func testSeasonFontTakesTheExactVariableWeight() throws {
        let font = try XCTUnwrap(SeasonFontProvider.variableFont(family: "Season Sans VF", size: 16, weight: 550))
        let variation = try XCTUnwrap(CTFontCopyVariation(font) as? [NSNumber: NSNumber])
        XCTAssertEqual(variation[NSNumber(value: 0x77676874)]?.intValue, 550)
    }

    /// The icons come from the CowchatKit package alone: the app ships no icon catalog of its own.
    func testEveryIconResolvesAsATemplateImageFromThePackage() throws {
        let package = try XCTUnwrap(SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates))
        for icon in GallopIcon.allCases {
            XCTAssertNotNil(icon.image, icon.rawValue)
            XCTAssertNotNil(icon.originalImage, icon.rawValue)
            XCTAssertEqual(GallopIcon.nativeImage(for: icon, in: package)?.renderingMode, .alwaysTemplate, icon.rawValue)
        }
        XCTAssertNil(UIImage(named: GallopIcon.add.rawValue, in: .main, with: nil), "the app has no copy of the icons")
    }

    func testHexColourParses() throws {
        let opaque = try XCTUnwrap(HexColor.components("#29754A"))
        XCTAssertEqual(opaque.r, 0x29 / 255.0, accuracy: 0.0001)
        XCTAssertEqual(opaque.g, 0x75 / 255.0, accuracy: 0.0001)
        XCTAssertEqual(opaque.b, 0x4A / 255.0, accuracy: 0.0001)
        XCTAssertEqual(opaque.a, 1)
        XCTAssertEqual(try XCTUnwrap(HexColor.components("#00000080")).a, 0x80 / 255.0, accuracy: 0.0001)
        XCTAssertNil(HexColor.components("29754A"))
        XCTAssertNil(HexColor.components("#29754"))
    }

    func testAdaptiveColourFollowsTheInterfaceStyle() {
        let color = UIColor(HexColor.adaptive(light: "#FFFFFF", dark: "#000000"))
        var white: CGFloat = -1
        color.resolvedColor(with: UITraitCollection(userInterfaceStyle: .light)).getWhite(&white, alpha: nil)
        XCTAssertEqual(white, 1, accuracy: 0.0001)
        color.resolvedColor(with: UITraitCollection(userInterfaceStyle: .dark)).getWhite(&white, alpha: nil)
        XCTAssertEqual(white, 0, accuracy: 0.0001)
    }

    func testPicturesFromOtherPeopleAreDownsampledAndOversizedOnesRefused() throws {
        let big = UIGraphicsImageRenderer(size: CGSize(width: 1200, height: 600), format: {
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            return format
        }()).pngData { context in
            UIColor.orange.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 1200, height: 600))
        }

        let thumbnail = try XCTUnwrap(SafeImage.thumbnail(from: big, maxPixelSize: 300))
        XCTAssertEqual(thumbnail.cgImage?.width, 300)
        XCTAssertEqual(thumbnail.cgImage?.height, 150)

        XCTAssertNil(SafeImage.thumbnail(from: Data("not a picture".utf8), maxPixelSize: 300))
        // A header that claims 60000 by 60000 pixels is refused before anything is decoded.
        var bomb = [UInt8](tinyPNG)
        bomb.replaceSubrange(16..<24, with: [0, 0, 0xEA, 0x60, 0, 0, 0xEA, 0x60])
        XCTAssertNil(SafeImage.thumbnail(from: Data(bomb), maxPixelSize: 300))
    }
}
