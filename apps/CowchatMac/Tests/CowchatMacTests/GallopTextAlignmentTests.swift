import SwiftUI
import XCTest
@testable import CowchatMac

/// `gallopText` takes macOS 26's exact line height only for leading text. A centred line that wraps is drawn about 27
/// points right of its frame with it and cut off at the frame's end, so text that isn't leading takes Gallop's earlier
/// line box.
final class GallopTextAlignmentTests: XCTestCase {
    func testTheLineBoxFollowsTheAlignment() {
        XCTAssertEqual(GallopLineBox.path(for: .leading), .exactLineHeight)
        XCTAssertEqual(GallopLineBox.path(for: .center), .lineSpacing)
        XCTAssertEqual(GallopLineBox.path(for: .trailing), .lineSpacing)
    }
}
