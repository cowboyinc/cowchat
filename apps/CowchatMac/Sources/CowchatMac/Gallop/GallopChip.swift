import SwiftUI

/// Gallop's Chip as the label of a menu, as Cowboy's filter chips draw it (`@cowboyinc/gallop-components` Chip; the
/// dashboard's Type and Severity chips; navigation note, section 3.3). It is 36 tall, at its own width: a 1-point edge,
/// 7, a 20 line of Body M, 7 and the edge, with 11 at the sides. Its label is the chosen value, with no chevron.
///
/// Off, at the default value, it has a Border/Default edge, Text/Tertiary and no fill, and hover and press take the
/// Secondary button's fills, edges and text. On, at any other value, it fills with Button/Primary and has no edge.
struct GallopChipLabel<Label: View>: View {
    let isOn: Bool
    var isHovering = false
    var isPressed = false
    @ViewBuilder let label: () -> Label

    var body: some View {
        label()
            .gallopText(.bodyM, color: textColor)
            .lineLimit(1)
            .fixedSize()
            .frame(height: 20)
            .padding(.vertical, 8)
            .padding(.horizontal, 12)
            .background(fill, in: Capsule())
            .overlay {
                if let edge { Capsule().strokeBorder(edge, lineWidth: 1) }
            }
            .contentShape(Capsule())
    }

    private var fill: Color {
        if isOn {
            return isPressed ? SemanticColor.buttonPrimaryPressed
                : isHovering ? SemanticColor.buttonPrimaryHover : SemanticColor.buttonPrimaryDefault
        }
        return isPressed ? SemanticColor.buttonSecondaryPressed
            : isHovering ? SemanticColor.buttonSecondaryHover : Color.clear
    }

    private var edge: Color? {
        guard !isOn else { return nil }
        return isPressed ? SemanticColor.borderPressed
            : isHovering ? SemanticColor.borderHover : SemanticColor.borderDefault
    }

    private var textColor: Color {
        if isOn {
            return isPressed ? SemanticColor.buttonPrimaryTextPressed
                : isHovering ? SemanticColor.buttonPrimaryTextHover : SemanticColor.buttonPrimaryTextDefault
        }
        return isPressed ? SemanticColor.buttonSecondaryTextPressed
            : isHovering ? SemanticColor.buttonSecondaryTextHover : SemanticColor.textTertiary
    }
}

extension GallopChipLabel where Label == Text {
    init(title: String, isOn: Bool, isHovering: Bool = false, isPressed: Bool = false) {
        self.init(isOn: isOn, isHovering: isHovering, isPressed: isPressed) { Text(title) }
    }
}

/// A menu drawn as a Gallop Chip that chooses one value, with a ticked toggle per value in the system menu, as
/// `TeamSwitcher` builds its menu. The chip is on while a value other than `defaultValue` is chosen. A SwiftUI menu
/// does not tell its label when it is pressed, so the chip draws hover and not press; the open menu is the press's
/// answer.
struct GallopChipMenu<Value: Hashable & Identifiable>: View {
    let values: [Value]
    @Binding var selection: Value
    let defaultValue: Value
    let title: (Value) -> String
    /// The chip's help and the first part of its accessible label.
    let help: String
    @State private var isHovering = false

    var body: some View {
        Menu {
            ForEach(values) { value in
                Toggle(title(value), isOn: Binding(get: { value == selection }, set: { if $0 { selection = value } }))
            }
        } label: {
            GallopChipLabel(title: title(selection), isOn: selection != defaultValue, isHovering: isHovering)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .onHover { isHovering = $0 }
        .help(help)
        .accessibilityLabel("\(help), \(title(selection))")
    }
}
