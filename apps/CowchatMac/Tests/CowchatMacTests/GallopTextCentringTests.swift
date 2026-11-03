import SwiftUI
import XCTest
@testable import CowchatMac

/// macOS 26's exact line height puts all of a role's leading under the line, so a label in a capsule button sat high
/// by half its leading. `gallopText` moves the glyphs down by that half, as the earlier line box does.
@MainActor
final class GallopTextCentringTests: XCTestCase {
    private let scale: CGFloat = 2

    func testCapitalsAreCentredInAThirtySixPointFrame() throws {
        let roles: [GallopTextStyle] = [.bodySStrong, .bodyM, .caption, .h4]
        for role in roles {
            let view = Text(verbatim: "HOX")
                .gallopText(role)
                .foregroundStyle(.black)
                .frame(width: 120, height: 36)
                .background(.white)
            let ink = try XCTUnwrap(inkRows(of: view), "\(role.name) drew nothing")
            let middle = (ink.top + ink.bottom) / 2 / scale
            XCTAssertEqual(middle, 18, accuracy: 0.5, "\(role.name) sits \(18 - middle) points high")
        }
    }

    func testBaselineAlignedRolesStillShareABaseline() throws {
        let view = HStack(alignment: .firstTextBaseline, spacing: 0) {
            Text(verbatim: "HX").gallopText(.bodyS).frame(width: 60, alignment: .leading)
            Text(verbatim: "HX").gallopText(.caption).frame(width: 60, alignment: .leading)
        }
        .foregroundStyle(.black)
        .frame(width: 120, height: 36)
        .background(.white)
        let left = try XCTUnwrap(inkRows(of: view, columns: 0..<Int(60 * scale)))
        let right = try XCTUnwrap(inkRows(of: view, columns: Int(60 * scale)..<Int(120 * scale)))
        XCTAssertEqual(left.bottom / scale, right.bottom / scale, accuracy: 0.5)
    }

    /// The first and last pixel rows with ink, counted from the top, in pixels. The bottom row's lower edge is the
    /// baseline, since H and X sit on it and O's overshoot above and below is the same.
    private func inkRows(of view: some View, columns: Range<Int>? = nil) -> (top: CGFloat, bottom: CGFloat)? {
        let renderer = ImageRenderer(content: view)
        renderer.scale = scale
        guard let image = renderer.cgImage else { return nil }
        let width = image.width
        let height = image.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        guard let context = CGContext(
            data: &pixels,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        let columns = columns ?? 0..<width
        var rows: [Int] = []
        for row in 0..<height {
            let hasInk = columns.contains { column in
                pixels[(row * width + column) * 4] < 128
            }
            if hasInk { rows.append(row) }
        }
        guard let first = rows.first, let last = rows.last else { return nil }
        return (CGFloat(first), CGFloat(last + 1))
    }
}
