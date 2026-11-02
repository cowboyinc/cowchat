import SwiftUI

/// The measures the window's pages share. A page's list is one column capped at the message column's 760, 4 in from
/// the card's edges, and its header spans the list and its insets, so the header's end sits over the rows' end.
enum PageLayout {
    static let listWidth: CGFloat = 760
    static let listInset: CGFloat = 4
    static var headerWidth: CGFloat { listWidth + listInset * 2 }
    /// Where an empty page's line sits: 20 in from the card's edge and 40 under the header.
    static let emptyLeading: CGFloat = 20
    static let emptyTop: CGFloat = 40
}

/// A page's header, drawn as a room's is: 58 tall with 10 above, the title in Heading 4 at 18 from the card's edge, an
/// optional line under it in Caption, Text/Tertiary, and an accessory at the trailing end, 20 in. The title keeps its
/// place whether or not there is a line under it, so it does not move between a room and a page.
struct PageHeader<Accessory: View>: View {
    let title: String
    var caption: String?
    @ViewBuilder var accessory: () -> Accessory

    init(title: String, caption: String? = nil, @ViewBuilder accessory: @escaping () -> Accessory) {
        self.title = title
        self.caption = caption
        self.accessory = accessory
    }

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .gallopText(.h4, color: SemanticColor.textPrimary)
                    .lineLimit(1)
                    .accessibilityAddTraits(.isHeader)
                // A page with no line holds its place, so the title sits where a room's does.
                Text(caption ?? " ")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
                    .lineLimit(1)
                    .opacity(caption == nil ? 0 : 1)
                    .accessibilityHidden(caption == nil)
            }
            Spacer(minLength: 12)
            accessory()
        }
        .padding(.top, 10)
        .padding(.leading, 18)
        .padding(.trailing, 20)
        .frame(height: 58)
        .frame(maxWidth: PageLayout.headerWidth, alignment: .leading)
    }
}

extension PageHeader where Accessory == EmptyView {
    init(title: String, caption: String? = nil) {
        self.init(title: title, caption: caption) { EmptyView() }
    }
}

/// A page row's fill, beside the sidebar's `SidebarRowBackground` and with its 12 continuous corners: Surface/400 under
/// the pointer and Button/Ghost/Pressed while pressed, with no hairline or shadow, which on the page's Surface/500
/// would draw a box.
struct PageRowBackground: View {
    let isHovering: Bool
    var isPressed = false

    var body: some View {
        RoundedRectangle(cornerRadius: 12, style: .continuous)
            .fill(isPressed ? SemanticColor.buttonGhostPressed
                  : isHovering ? SemanticColor.surface400 : Color.clear)
    }
}

/// A page row that is a button: `PageRowBackground` under its label, with the hover and the press.
struct PageRowButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        PageRowButtonBody(configuration: configuration)
    }
}

private struct PageRowButtonBody: View {
    let configuration: ButtonStyleConfiguration
    @State private var isHovering = false

    var body: some View {
        configuration.label
            .background(PageRowBackground(isHovering: isHovering, isPressed: configuration.isPressed))
            .contentShape(Rectangle())
            .onHover { isHovering = $0 }
    }
}

/// The 1-point seam in the middle of the 8 between two page rows, inset 16 at each side, in Surface/Glass/Border/Shadow,
/// as production's record rows draw it. A row's rounded hover fill never touches it.
struct PageRowSeam: View {
    var body: some View {
        Rectangle()
            .fill(SemanticColor.surfaceGlassBorderShadow)
            .frame(height: 1)
            .padding(.horizontal, 16)
            .padding(.vertical, 3.5)
            .accessibilityHidden(true)
    }
}

/// The sidebar's search field as a view of its own: 36 tall, Textfield/Default in a capsule with a 1-point
/// Border/Default edge, 11 at the sides, the 13 search icon in Icon/Tertiary, 8, the text in Body M, and once text is
/// typed the 13 dismiss button in Icon/Subtle. Escape clears it and gives up the focus.
struct SearchCapsule: View {
    let placeholder: String
    @Binding var text: String
    var isFocused: FocusState<Bool>.Binding
    /// The dismiss button's accessible label.
    let clearLabel: String
    var width: CGFloat?

    var body: some View {
        HStack(spacing: 8) {
            GallopIconView(icon: .search, fallbackSystemName: "magnifyingglass", size: 13)
                .foregroundStyle(SemanticColor.iconTertiary)
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .focused(isFocused)
                .onExitCommand {
                    // Escape clears the filter and returns focus to the list.
                    text = ""
                    isFocused.wrappedValue = false
                }
                .accessibilityLabel(placeholder)
            if !text.isEmpty {
                Button { text = "" } label: {
                    GallopIconView(icon: .dismiss, fallbackSystemName: "xmark.circle.fill", size: 13)
                        .foregroundStyle(SemanticColor.iconSubtle)
                }
                .buttonStyle(.plain)
                .macAccessibleAction(label: clearLabel) { text = "" }
            }
        }
        .padding(.horizontal, 11)
        .frame(width: width)
        .frame(height: 36)
        .background(SemanticColor.textfieldDefault, in: Capsule())
        .overlay {
            Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
    }
}
