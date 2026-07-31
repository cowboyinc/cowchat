import AppKit
import SwiftUI
import XCTest
@testable import CowchatMac

final class ComposerTextFieldTests: XCTestCase {
    @MainActor
    func testNativeEditEventUpdatesTheSwiftUIBinding() {
        var draft = ""
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let composer = ComposerTextField(
            text: binding,
            placeholder: "Message lobby",
            isEnabled: true,
            onSubmit: {}
        )
        let coordinator = composer.makeCoordinator()
        let field = NSTextField()
        field.stringValue = "hello from the composer"

        coordinator.controlTextDidChange(Notification(name: Notification.Name("composer-edit"), object: field))

        XCTAssertEqual(draft, "hello from the composer")
    }

    @MainActor
    func testReturnSubmitsTheCurrentText() {
        var draft = ""
        var didSubmit = false
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let composer = ComposerTextField(
            text: binding,
            placeholder: "Message lobby",
            isEnabled: true,
            onSubmit: { didSubmit = true }
        )
        let coordinator = composer.makeCoordinator()
        let field = NSTextField()
        let editor = NSTextView()
        field.stringValue = "send me"

        let handled = coordinator.control(
            field,
            textView: editor,
            doCommandBy: #selector(NSResponder.insertNewline(_:))
        )

        XCTAssertTrue(handled)
        XCTAssertTrue(didSubmit)
        XCTAssertEqual(draft, "send me")
    }
}
