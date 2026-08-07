import AppKit
import SwiftUI
import XCTest
@testable import CowchatMac

final class ComposerTextFieldTests: XCTestCase {
    @MainActor
    private func makeComposer(
        draft: Binding<String>,
        onSubmit: @escaping () -> Void = {},
        onCancel: (() -> Void)? = nil
    ) -> ComposerTextField {
        ComposerTextField(
            text: draft,
            placeholder: "Message lobby",
            isEnabled: true,
            onSubmit: onSubmit,
            onCancel: onCancel
        )
    }

    @MainActor
    func testNativeEditEventUpdatesTheSwiftUIBinding() {
        var draft = ""
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let coordinator = makeComposer(draft: binding).makeCoordinator()
        let textView = NSTextView()
        textView.string = "hello from the composer"

        coordinator.textDidChange(Notification(name: NSText.didChangeNotification, object: textView))

        XCTAssertEqual(draft, "hello from the composer")
    }

    @MainActor
    func testPastedNewlinesAreFlattenedToSpaces() {
        var draft = ""
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let coordinator = makeComposer(draft: binding).makeCoordinator()
        let textView = NSTextView()
        textView.string = "line one\nline two\nline three"

        coordinator.textDidChange(Notification(name: NSText.didChangeNotification, object: textView))

        XCTAssertEqual(draft, "line one line two line three")
        XCTAssertEqual(textView.string, "line one line two line three")
    }

    @MainActor
    func testReturnSubmitsTheCurrentText() {
        var draft = ""
        var didSubmit = false
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let coordinator = makeComposer(draft: binding, onSubmit: { didSubmit = true }).makeCoordinator()
        let textView = NSTextView()
        textView.string = "send me"

        let handled = coordinator.textView(
            textView,
            doCommandBy: #selector(NSResponder.insertNewline(_:))
        )

        XCTAssertTrue(handled)
        XCTAssertTrue(didSubmit)
        XCTAssertEqual(draft, "send me")
    }

    @MainActor
    func testEscapeInvokesCancelAndIsConsumed() {
        var draft = ""
        var didCancel = false
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let coordinator = makeComposer(draft: binding, onCancel: { didCancel = true }).makeCoordinator()

        let handled = coordinator.textView(
            NSTextView(),
            doCommandBy: #selector(NSResponder.cancelOperation(_:))
        )

        XCTAssertTrue(handled)
        XCTAssertTrue(didCancel)
    }

    @MainActor
    func testEscapeWithoutCancelHandlerIsNotConsumed() {
        var draft = ""
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let coordinator = makeComposer(draft: binding).makeCoordinator()

        let handled = coordinator.textView(
            NSTextView(),
            doCommandBy: #selector(NSResponder.cancelOperation(_:))
        )

        XCTAssertFalse(handled)
    }
}
