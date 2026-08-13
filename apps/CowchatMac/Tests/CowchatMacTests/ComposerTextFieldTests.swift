import AppKit
import SwiftUI
import XCTest
@testable import CowchatMac

final class ComposerTextFieldTests: XCTestCase {
    @MainActor
    private func makeComposer(
        draft: Binding<String>,
        measuredHeight: Binding<CGFloat>? = nil,
        isFocused: Binding<Bool> = .constant(false),
        onSubmit: @escaping () -> Void = {},
        onCancel: (() -> Void)? = nil
    ) -> ComposerTextField {
        ComposerTextField(
            text: draft,
            measuredHeight: measuredHeight ?? .constant(ComposerTextField.naturalHeight),
            isFocused: isFocused,
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
    func testPastedNewlinesArePreserved() {
        var draft = ""
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let coordinator = makeComposer(draft: binding).makeCoordinator()
        let textView = NSTextView()
        textView.string = "line one\nline two\nline three"

        coordinator.textDidChange(Notification(name: NSText.didChangeNotification, object: textView))

        XCTAssertEqual(draft, "line one\nline two\nline three")
        XCTAssertEqual(textView.string, "line one\nline two\nline three")
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
    func testShiftReturnInsertsANewlineWithoutSubmitting() {
        var draft = ""
        var didSubmit = false
        let binding = Binding<String>(get: { draft }, set: { draft = $0 })
        let coordinator = makeComposer(draft: binding, onSubmit: { didSubmit = true }).makeCoordinator()
        coordinator.currentModifierFlags = { [.shift] }
        let textView = NSTextView()
        textView.string = "line one"
        textView.setSelectedRange(NSRange(location: textView.string.utf16.count, length: 0))

        let handled = coordinator.textView(
            textView,
            doCommandBy: #selector(NSResponder.insertNewline(_:))
        )

        XCTAssertTrue(handled)
        XCTAssertFalse(didSubmit)
        XCTAssertEqual(textView.string, "line one\n")
        XCTAssertEqual(draft, "line one\n")
    }

    @MainActor
    func testEditingNotificationsExposeTheFocusState() {
        var focused = false
        let focusBinding = Binding<Bool>(get: { focused }, set: { focused = $0 })
        let coordinator = makeComposer(
            draft: .constant(""),
            isFocused: focusBinding
        ).makeCoordinator()
        let notification = Notification(name: NSText.didBeginEditingNotification)

        coordinator.textDidBeginEditing(notification)
        XCTAssertTrue(focused)

        coordinator.textDidEndEditing(notification)
        XCTAssertFalse(focused)
    }

    @MainActor
    func testTextViewWrapsAndMeasuresMoreThanOneLine() {
        let textView = ComposerTextView(frame: NSRect(x: 0, y: 0, width: 120, height: 40))
        ComposerTextField.configure(
            textView,
            font: SeasonFontProvider().nativeFont(for: .bodyL)
        )
        textView.setFrameSize(NSSize(width: 120, height: 40))
        textView.string = "This is a long composer draft that must wrap onto several lines."

        XCTAssertTrue(textView.isVerticallyResizable)
        XCTAssertFalse(textView.isHorizontallyResizable)
        XCTAssertTrue(textView.textContainer?.widthTracksTextView == true)
        XCTAssertGreaterThan(
            ComposerTextField.contentHeight(for: textView),
            ComposerTextField.naturalHeight
        )
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
