import AppKit
import SwiftUI

/// A native macOS text field for the chat composer. Using AppKit here avoids
/// SwiftUI focus regressions inside a NavigationSplitView detail column.
///
/// Built on `NSTextView` rather than `NSTextField`: the field-editor caret
/// always spans the full line box, which reads as oversized against Season's
/// proportions — a text view lets the insertion point be drawn at text
/// height instead.
struct ComposerTextField: NSViewRepresentable {
    @Binding var text: String
    let placeholder: String
    let isEnabled: Bool
    let onSubmit: () -> Void
    var onCancel: (() -> Void)?

    /// The font's real line height. Framing the field shorter than this makes
    /// the layout draw a cramped, mis-centered insertion caret.
    static let naturalHeight: CGFloat = {
        let font = SeasonFontProvider().nativeFont(for: .bodyL)
        return ceil(font.ascender - font.descender + font.leading)
    }()

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text, onSubmit: onSubmit, onCancel: onCancel)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let font = SeasonFontProvider().nativeFont(for: .bodyL)

        let textView = ComposerTextView()
        textView.font = font
        textView.textColor = SemanticColor.AppKitColor.textPrimary
        textView.insertionPointColor = SemanticColor.AppKitColor.textPrimary
        textView.focusRingType = .none
        textView.drawsBackground = false
        textView.isRichText = false
        textView.usesFontPanel = false
        textView.usesFindPanel = false
        textView.allowsUndo = true
        textView.isVerticallyResizable = false
        textView.isHorizontallyResizable = true
        textView.textContainerInset = .zero
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.widthTracksTextView = false
        textView.textContainer?.containerSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: Self.naturalHeight
        )
        textView.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: Self.naturalHeight
        )
        textView.delegate = context.coordinator

        // Borderless clip container so long drafts scroll horizontally and
        // the caret stays in view; never shows scrollers.
        let scrollView = NSScrollView()
        scrollView.documentView = textView
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasHorizontalScroller = false
        scrollView.hasVerticalScroller = false
        scrollView.horizontalScrollElasticity = .none
        scrollView.verticalScrollElasticity = .none
        scrollView.focusRingType = .none
        scrollView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? ComposerTextView else { return }
        context.coordinator.text = $text
        context.coordinator.onSubmit = onSubmit
        context.coordinator.onCancel = onCancel
        textView.placeholderString = NSAttributedString(
            string: placeholder,
            attributes: [
                .foregroundColor: SemanticColor.AppKitColor.textTertiary,
                .font: SeasonFontProvider().nativeFont(for: .bodyL),
            ]
        )
        textView.textColor = SemanticColor.AppKitColor.textPrimary
        textView.isEditable = isEnabled
        textView.isSelectable = isEnabled
        textView.setAccessibilityLabel(placeholder)
        if textView.string != text { textView.string = text }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var text: Binding<String>
        var onSubmit: () -> Void
        var onCancel: (() -> Void)?

        init(text: Binding<String>, onSubmit: @escaping () -> Void, onCancel: (() -> Void)?) {
            self.text = text
            self.onSubmit = onSubmit
            self.onCancel = onCancel
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            // Single-line composer: flatten pasted newlines.
            if textView.string.contains(where: \.isNewline) {
                textView.string = textView.string
                    .components(separatedBy: .newlines)
                    .joined(separator: " ")
            }
            text.wrappedValue = textView.string
        }

        func textView(
            _ textView: NSTextView,
            doCommandBy commandSelector: Selector
        ) -> Bool {
            if commandSelector == #selector(NSResponder.cancelOperation(_:)), let onCancel {
                onCancel()
                return true
            }
            guard commandSelector == #selector(NSResponder.insertNewline(_:)) else { return false }
            text.wrappedValue = textView.string
            onSubmit()
            return true
        }
    }
}

/// Draws the placeholder itself (`NSTextView` has none) and trims the
/// insertion caret from the full line box down to text height — cap top to
/// just under the baseline — so it hugs the glyphs the way web inputs do.
final class ComposerTextView: NSTextView {
    var placeholderString: NSAttributedString? {
        didSet { needsDisplay = true }
    }

    private var hasRequestedFocus = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil, !hasRequestedFocus else { return }
        hasRequestedFocus = true
        DispatchQueue.main.async { [weak self] in
            guard let self, let window = self.window, self.isEditable else { return }
            window.makeFirstResponder(self)
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, let placeholderString, placeholderString.length > 0 else { return }
        // Drop the placeholder onto the layout manager's baseline (it rounds
        // ascent up) so the first typed character replaces it without a jump.
        var origin = textContainerOrigin
        if let font = placeholderString.attribute(.font, at: 0, effectiveRange: nil) as? NSFont,
           let layoutManager {
            origin.y += layoutManager.defaultBaselineOffset(for: font) - font.ascender
        }
        placeholderString.draw(at: origin)
    }

    override func drawInsertionPoint(in rect: NSRect, color: NSColor, turnedOn flag: Bool) {
        super.drawInsertionPoint(in: caretRect(for: rect), color: color, turnedOn: flag)
    }

    override func setNeedsDisplay(_ rect: NSRect, avoidAdditionalLayout flag: Bool) {
        // The system invalidates the caret using the untrimmed line-box rect;
        // widen it so blink-off erases the trimmed caret cleanly.
        super.setNeedsDisplay(rect.union(caretRect(for: rect)), avoidAdditionalLayout: flag)
    }

    private func caretRect(for rect: NSRect) -> NSRect {
        guard let font = (typingAttributes[.font] as? NSFont) ?? self.font,
              let layoutManager
        else { return rect }
        let baseline = rect.minY + layoutManager.defaultBaselineOffset(for: font)
        var caret = rect
        caret.origin.y = baseline - font.capHeight - 2
        caret.size.height = font.capHeight + 5
        return caret
    }
}
