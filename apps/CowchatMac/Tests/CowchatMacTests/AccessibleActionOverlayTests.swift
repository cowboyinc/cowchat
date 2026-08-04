import AppKit
import XCTest
@testable import CowchatMac

final class AccessibleActionOverlayTests: XCTestCase {
    @MainActor
    func testNativeAccessibilityButtonPublishesLabelValueAndAction() {
        var didPress = false
        let view = AccessibleActionOverlay.ActionView()
        view.label = "Open lobby"
        view.accessibilityValueText = "selected"
        view.action = { didPress = true }

        XCTAssertTrue(view.isAccessibilityElement())
        XCTAssertEqual(view.accessibilityRole(), .button)
        XCTAssertEqual(view.accessibilityLabel(), "Open lobby")
        XCTAssertEqual(view.accessibilityValue() as? String, "selected")
        XCTAssertTrue(view.accessibilityPerformPress())
        XCTAssertTrue(didPress)
    }

    @MainActor
    func testDisabledAccessibilityButtonRejectsPress() {
        var didPress = false
        let view = AccessibleActionOverlay.ActionView()
        view.actionIsEnabled = false
        view.action = { didPress = true }

        XCTAssertFalse(view.accessibilityPerformPress())
        XCTAssertFalse(didPress)
    }
}
