import AppKit
import SwiftUI

/// A native macOS text field for the chat composer. Using AppKit here avoids
/// SwiftUI focus regressions inside a NavigationSplitView detail column.
struct ComposerTextField: NSViewRepresentable {
    @Binding var text: String
    let placeholder: String
    let isEnabled: Bool
    let onSubmit: () -> Void
    var onCancel: (() -> Void)?

    /// The font's real line height. Framing the field shorter than this makes
    /// the field editor draw a cramped, mis-centered insertion caret.
    static let naturalHeight: CGFloat = {
        let font = SeasonFontProvider().nativeFont(for: .bodyL)
        return ceil(font.ascender - font.descender + font.leading)
    }()

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text, onSubmit: onSubmit, onCancel: onCancel)
    }

    func makeNSView(context: Context) -> NSTextField {
        let field = InitiallyFocusedTextField()
        field.isBordered = false
        field.isBezeled = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = SeasonFontProvider().nativeFont(for: .bodyL)
        field.textColor = SemanticColor.AppKitColor.textPrimary
        field.isEditable = true
        field.isSelectable = true
        field.usesSingleLineMode = true
        field.lineBreakMode = .byTruncatingTail
        field.delegate = context.coordinator
        field.setAccessibilityLabel(placeholder)
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return field
    }

    private final class InitiallyFocusedTextField: NSTextField {
        private var hasRequestedFocus = false

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard window != nil, !hasRequestedFocus else { return }
            hasRequestedFocus = true
            DispatchQueue.main.async { [weak self] in
                guard let self, let window = self.window, self.isEnabled else { return }
                window.makeFirstResponder(self)
            }
        }
    }

    func updateNSView(_ field: NSTextField, context: Context) {
        context.coordinator.text = $text
        context.coordinator.onSubmit = onSubmit
        context.coordinator.onCancel = onCancel
        field.placeholderAttributedString = NSAttributedString(
            string: placeholder,
            attributes: [
                .foregroundColor: SemanticColor.AppKitColor.textTertiary,
                .font: SeasonFontProvider().nativeFont(for: .bodyL),
            ]
        )
        field.textColor = SemanticColor.AppKitColor.textPrimary
        field.isEnabled = isEnabled
        field.setAccessibilityLabel(placeholder)
        if field.stringValue != text { field.stringValue = text }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var text: Binding<String>
        var onSubmit: () -> Void
        var onCancel: (() -> Void)?

        init(text: Binding<String>, onSubmit: @escaping () -> Void, onCancel: (() -> Void)?) {
            self.text = text
            self.onSubmit = onSubmit
            self.onCancel = onCancel
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            text.wrappedValue = field.stringValue
        }

        func control(
            _ control: NSControl,
            textView: NSTextView,
            doCommandBy commandSelector: Selector
        ) -> Bool {
            if commandSelector == #selector(NSResponder.cancelOperation(_:)), let onCancel {
                onCancel()
                return true
            }
            guard commandSelector == #selector(NSResponder.insertNewline(_:)),
                  let field = control as? NSTextField else { return false }
            text.wrappedValue = field.stringValue
            onSubmit()
            return true
        }
    }
}
