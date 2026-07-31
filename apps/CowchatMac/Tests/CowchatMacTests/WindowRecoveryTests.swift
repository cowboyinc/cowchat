import AppKit
import XCTest
@testable import CowchatMac

final class WindowRecoveryTests: XCTestCase {
    func testOffscreenWindowMovesToRemainingDisplay() {
        let oldExternalFrame = CGRect(x: 2_000, y: 200, width: 1_120, height: 720)
        let builtInDisplay = CGRect(x: 0, y: 0, width: 1_728, height: 1_080)

        let recovered = CowchatAppDelegate.recoveredFrame(
            oldExternalFrame,
            visibleFrames: [builtInDisplay]
        )

        XCTAssertTrue(builtInDisplay.contains(recovered))
        XCTAssertEqual(recovered.midX, builtInDisplay.midX)
        XCTAssertEqual(recovered.midY, builtInDisplay.midY)
    }

    func testPartiallyVisibleWindowIsClampedWithoutRecentering() {
        let frame = CGRect(x: -40, y: 100, width: 900, height: 700)
        let display = CGRect(x: 0, y: 0, width: 1_728, height: 1_080)

        let recovered = CowchatAppDelegate.recoveredFrame(frame, visibleFrames: [display])

        XCTAssertEqual(recovered.origin.x, 0)
        XCTAssertEqual(recovered.origin.y, 100)
    }
}
