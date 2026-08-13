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
    @Binding var measuredHeight: CGFloat
    @Binding var isFocused: Bool
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

    /// Dash's overflow state shows at most five lines before the text view
    /// scrolls, keeping a long draft from taking over the conversation.
    static let maximumHeight = naturalHeight * 5

    func makeCoordinator() -> Coordinator {
        Coordinator(
            text: $text,
            measuredHeight: $measuredHeight,
            isFocused: $isFocused,
            onSubmit: onSubmit,
            onCancel: onCancel
        )
    }

    func makeNSView(context: Context) -> ComposerScrollView {
        let font = SeasonFontProvider().nativeFont(for: .bodyL)

        let textView = ComposerTextView()
        Self.configure(textView, font: font)
        textView.delegate = context.coordinator

        // Borderless clip container. Text wraps to the available width and,
        // after five lines, scrolls vertically without visible scroll chrome.
        let scrollView = ComposerScrollView()
        scrollView.documentView = textView
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasHorizontalScroller = false
        scrollView.hasVerticalScroller = false
        scrollView.horizontalScrollElasticity = .none
        scrollView.verticalScrollElasticity = .none
        scrollView.focusRingType = .none
        scrollView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        scrollView.onContentWidthChange = { [weak scrollView, weak textView, weak coordinator = context.coordinator] in
            guard let scrollView, let textView else { return }
            coordinator?.updateHeight(for: textView, in: scrollView)
        }
        return scrollView
    }

    func updateNSView(_ scrollView: ComposerScrollView, context: Context) {
        guard let textView = scrollView.documentView as? ComposerTextView else { return }
        context.coordinator.text = $text
        context.coordinator.measuredHeight = $measuredHeight
        context.coordinator.isFocused = $isFocused
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
        context.coordinator.updateHeight(for: textView, in: scrollView)
    }

    static func configure(_ textView: ComposerTextView, font: NSFont) {
        textView.font = font
        textView.textColor = SemanticColor.AppKitColor.textPrimary
        textView.insertionPointColor = SemanticColor.AppKitColor.textPrimary
        textView.focusRingType = .none
        textView.drawsBackground = false
        textView.isRichText = false
        textView.usesFontPanel = false
        textView.usesFindPanel = false
        textView.allowsUndo = true
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainerInset = .zero
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.heightTracksTextView = false
        textView.textContainer?.containerSize = NSSize(
            width: 0,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.minSize = NSSize(width: 0, height: naturalHeight)
        textView.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
    }

    static func contentHeight(for textView: NSTextView) -> CGFloat {
        guard let textContainer = textView.textContainer,
              let layoutManager = textView.layoutManager else { return naturalHeight }
        if textContainer.widthTracksTextView, textView.bounds.width > 0 {
            textContainer.containerSize = NSSize(
                width: textView.bounds.width,
                height: CGFloat.greatestFiniteMagnitude
            )
        }
        layoutManager.ensureLayout(for: textContainer)
        return max(naturalHeight, ceil(layoutManager.usedRect(for: textContainer).height))
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var text: Binding<String>
        var measuredHeight: Binding<CGFloat>
        var isFocused: Binding<Bool>
        var onSubmit: () -> Void
        var onCancel: (() -> Void)?
        var currentModifierFlags: () -> NSEvent.ModifierFlags = {
            NSApp.currentEvent?.modifierFlags ?? []
        }

        init(
            text: Binding<String>,
            measuredHeight: Binding<CGFloat>,
            isFocused: Binding<Bool>,
            onSubmit: @escaping () -> Void,
            onCancel: (() -> Void)?
        ) {
            self.text = text
            self.measuredHeight = measuredHeight
            self.isFocused = isFocused
            self.onSubmit = onSubmit
            self.onCancel = onCancel
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            text.wrappedValue = textView.string
            if let scrollView = textView.enclosingScrollView {
                updateHeight(for: textView, in: scrollView)
            }
        }

        func textDidBeginEditing(_ notification: Notification) {
            isFocused.wrappedValue = true
        }

        func textDidEndEditing(_ notification: Notification) {
            isFocused.wrappedValue = false
        }

        func textView(
            _ textView: NSTextView,
            doCommandBy commandSelector: Selector
        ) -> Bool {
            if commandSelector == #selector(NSResponder.cancelOperation(_:)), let onCancel {
                onCancel()
                return true
            }
            if commandSelector == #selector(NSResponder.insertLineBreak(_:))
                || (commandSelector == #selector(NSResponder.insertNewline(_:))
                    && currentModifierFlags().contains(.shift)) {
                textView.insertText("\n", replacementRange: textView.selectedRange())
                text.wrappedValue = textView.string
                if let scrollView = textView.enclosingScrollView {
                    updateHeight(for: textView, in: scrollView)
                }
                return true
            }
            guard commandSelector == #selector(NSResponder.insertNewline(_:)) else { return false }
            text.wrappedValue = textView.string
            onSubmit()
            return true
        }

        func updateHeight(for textView: NSTextView, in scrollView: NSScrollView) {
            let width = max(scrollView.contentSize.width, 1)
            if abs(textView.frame.width - width) > 0.5 {
                textView.setFrameSize(NSSize(width: width, height: max(textView.frame.height, ComposerTextField.naturalHeight)))
            }

            let fullHeight = ComposerTextField.contentHeight(for: textView)
            let documentHeight = max(fullHeight, scrollView.contentSize.height)
            if abs(textView.frame.height - documentHeight) > 0.5 {
                textView.setFrameSize(NSSize(width: width, height: documentHeight))
            }
            textView.scrollRangeToVisible(textView.selectedRange())

            let visibleHeight = min(fullHeight, ComposerTextField.maximumHeight)
            guard abs(measuredHeight.wrappedValue - visibleHeight) > 0.5 else { return }
            DispatchQueue.main.async { [weak self] in
                guard let self,
                      abs(self.measuredHeight.wrappedValue - visibleHeight) > 0.5 else { return }
                self.measuredHeight.wrappedValue = visibleHeight
            }
        }
    }
}

final class ComposerScrollView: NSScrollView {
    var onContentWidthChange: (() -> Void)?
    private var lastContentWidth: CGFloat = -1

    override func layout() {
        super.layout()
        let width = contentSize.width
        guard abs(width - lastContentWidth) > 0.5 else { return }
        lastContentWidth = width
        onContentWidthChange?()
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
