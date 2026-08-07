import SwiftUI
import XCTest
@testable import CowchatMac

final class RoomIconTests: XCTestCase {
    /// A realistic sidebar: cowboy rooms cluster on the same initial, which is
    /// exactly where a name-hashed icon is most likely to look duplicated.
    private let rooms = [
        "lobby",
        "harness-signing",
        "cbqs-merge-campaign",
        "cbss-dod-13",
        "canyon-deploy",
        "homestead-v1",
        "node-devnet",
        "gateway-volumes",
        "seed-raise",
        "ranchhand",
        "pvm-stack",
        "empire-swarm",
    ]

    /// Pinned digests. These are not implementation trivia: if the hash moves,
    /// every room's icon silently changes, and this is the only place that says
    /// so out loud.
    func testSeedDigestsArePinned() {
        XCTAssertEqual(RoomIconSeed("lobby").bits("hide.palette"), 7_795_330_747_600_919_999)
        XCTAssertEqual(
            RoomIconSeed("harness-signing").bits("brand.modifier"),
            4_531_811_752_026_846_529
        )
        XCTAssertEqual(
            RoomIconSeed("canyon-deploy").bits("bandana.spokes"),
            691_676_640_376_865_779
        )
    }

    func testTraitsAreStableForTheSameName() {
        for room in rooms {
            XCTAssertEqual(RoomIconTraits(name: room), RoomIconTraits(name: room))
        }
    }

    func testFieldsAreDomainSeparated() {
        let seed = RoomIconSeed("lobby")
        XCTAssertNotEqual(seed.bits("hide.palette"), seed.bits("combo.palette"))
        XCTAssertNotEqual(seed.bits("hide.0.x"), seed.bits("hide.0.y"))
    }

    func testDerivedValuesStayInRange() {
        for room in rooms {
            let traits = RoomIconTraits(name: room)
            XCTAssertTrue((3...6).contains(traits.hide.patches.count))
            XCTAssertTrue((6...9).contains(traits.bandana.spokes))
            XCTAssertTrue(traits.bandana.inner < traits.bandana.outer)
            XCTAssertTrue(RoomIconPalette.hides.indices.contains(traits.hide.paletteIndex))
            XCTAssertTrue(RoomIconPalette.brands.indices.contains(traits.brand.paletteIndex))
            XCTAssertTrue(RoomIconPalette.bandanas.indices.contains(traits.bandana.paletteIndex))
            for patch in traits.hide.patches {
                XCTAssertEqual(patch.wobble.count, HideTraits.wobblePoints)
                XCTAssertTrue((0.14...0.86).contains(patch.x))
                XCTAssertTrue((0.14...0.86).contains(patch.y))
            }
        }
    }

    func testHideAndComboDrawFromDifferentDomains() {
        for room in rooms {
            let traits = RoomIconTraits(name: room)
            XCTAssertNotEqual(traits.hide, traits.combo, "\(room) reuses one patch field")
        }
    }

    func testBrandMarkUsesTwoTokens() {
        XCTAssertEqual(BrandTraits.mark(for: "lobby"), "L")
        XCTAssertEqual(BrandTraits.mark(for: "cbqs-merge-campaign"), "CM")
        XCTAssertEqual(BrandTraits.mark(for: "Room 44"), "R4")
        XCTAssertEqual(BrandTraits.mark(for: "—"), "#")
    }

    /// The whole point of the feature: a realistic room list must not produce
    /// two identical icons.
    func testRealisticRoomListHasNoDuplicateIcons() {
        let brands = rooms.map { RoomIconTraits(name: $0).brand }
        XCTAssertEqual(Set(brands.map(\.description)).count, rooms.count)

        let combos = rooms.map { room -> String in
            let traits = RoomIconTraits(name: room)
            return "\(traits.combo.paletteIndex)|\(traits.brand.description)"
        }
        XCTAssertEqual(Set(combos).count, rooms.count)

        let bandanas = rooms.map { RoomIconTraits(name: $0).bandana }
        XCTAssertEqual(Set(bandanas.map(\.description)).count, rooms.count)
    }

    func testHidePalettesSpreadAcrossTheList() {
        let used = Set(rooms.map { RoomIconTraits(name: $0).combo.paletteIndex })
        XCTAssertGreaterThanOrEqual(used.count, 4)
    }

    func testStoredStyleFallsBackWhenUnknown() {
        XCTAssertNil(RoomIconStyle(rawValue: "tumbleweed"))
        XCTAssertEqual(RoomIconStyle.fallback, .brandOnHide)
        for style in RoomIconStyle.allCases {
            XCTAssertFalse(style.label.isEmpty)
            XCTAssertFalse(style.blurb.isEmpty)
        }
    }

    /// Guards the drawing code, not the parameters: a Canvas that clips wrong
    /// or paints one flat disc still satisfies every trait assertion above.
    @MainActor
    func testEveryProceduralStyleRendersSomething() throws {
        for style in RoomIconStyle.allCases where style != .initials {
            for room in ["lobby", "cbqs-merge-campaign"] {
                let image = try XCTUnwrap(
                    render(RoomIconView(name: room, style: style, size: 44)),
                    "\(style.rawValue) produced no bitmap"
                )
                XCTAssertGreaterThan(
                    distinctColors(in: image), 2,
                    "\(style.rawValue)/\(room) rendered as a flat fill"
                )
            }
        }
    }

    @MainActor
    private func render(_ view: some View) -> CGImage? {
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        return renderer.cgImage
    }

    private func distinctColors(in image: CGImage) -> Int {
        let width = image.width
        let height = image.height
        var pixels = [UInt32](repeating: 0, count: width * height)
        guard let context = CGContext(
            data: &pixels,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return 0 }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return Set(pixels).count
    }
}

private extension BrandTraits {
    var description: String {
        "\(mark)|\(modifier.rawValue)|\(stance.rawValue)|\(paletteIndex)"
    }
}

private extension BandanaTraits {
    var description: String {
        "\(paletteIndex)|\(motif.rawValue)|\(spokes)|\(inner)|\(outer)|\(hub)|\(dash)"
    }
}
