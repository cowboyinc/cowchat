import AppKit
import SwiftUI

/// Gives custom-drawn SwiftUI controls a native macOS accessibility element.
/// SwiftUI's plain buttons in this app do not currently publish their labels
/// through AX, so the visual button remains the pointer target while this
/// transparent sibling provides the label and press action to assistive tech.
struct AccessibleActionOverlay: NSViewRepresentable {
    let label: String
    var value: String?
    var isEnabled = true
    /// Actions beside the press, which a screen reader lists by name.
    var namedActions: [MacNamedAction] = []
    let action: () -> Void

    func makeNSView(context: Context) -> ActionView {
        let view = ActionView()
        update(view)
        return view
    }

    func updateNSView(_ view: ActionView, context: Context) {
        update(view)
    }

    private func update(_ view: ActionView) {
        view.label = label
        view.accessibilityValueText = value
        view.actionIsEnabled = isEnabled
        view.action = action
        view.setAccessibilityCustomActions(namedActions.map { named in
            NSAccessibilityCustomAction(name: named.name) {
                named.action()
                return true
            }
        })
        view.setAccessibilityEnabled(isEnabled)
    }

    final class ActionView: NSView {
        var label = ""
        var accessibilityValueText: String?
        var actionIsEnabled = true
        var action: () -> Void = {}

        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func isAccessibilityElement() -> Bool { true }
        override func accessibilityRole() -> NSAccessibility.Role? { .button }
        override func accessibilityLabel() -> String? { label }
        override func accessibilityValue() -> Any? { accessibilityValueText }
        override func accessibilityPerformPress() -> Bool {
            guard actionIsEnabled else { return false }
            action()
            return true
        }
    }
}

/// An action an accessible element offers by name beside its press, as a Team row's "Open on GitHub".
struct MacNamedAction {
    let name: String
    let action: () -> Void
}

extension View {
    func macAccessibleAction(
        label: String,
        value: String? = nil,
        isEnabled: Bool = true,
        namedActions: [MacNamedAction] = [],
        action: @escaping () -> Void
    ) -> some View {
        accessibilityHidden(true)
            .overlay {
                AccessibleActionOverlay(
                    label: label,
                    value: value,
                    isEnabled: isEnabled,
                    namedActions: namedActions,
                    action: action
                )
                .allowsHitTesting(false)
            }
    }
}
