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
    /// The gap between the header and a page's list, as production puts 32 under a page head and the house rule spaces
    /// sections (navigation round 2, section 4.2).
    static let listTop: CGFloat = 32
}

/// A page's header, drawn as a room's is: 58 tall with 10 above, the title in Heading 4 at 18 from the card's edge, an
/// optional line under it in Caption, Text/Tertiary, and an accessory at the trailing end, 20 in. The title keeps its
/// place whether or not there is a line under it, so it does not move between a room and a page. It is as wide as the
/// page's rows and their insets (`rowsWidth`), so its end stays over theirs when a legacy scroller takes the list's
/// right edge.
struct PageHeader<Accessory: View>: View {
    let title: String
    var caption: String?
    var rowsWidth: CGFloat?
    @ViewBuilder var accessory: () -> Accessory

    init(title: String, caption: String? = nil, rowsWidth: CGFloat? = nil,
         @ViewBuilder accessory: @escaping () -> Accessory) {
        self.title = title
        self.caption = caption
        self.rowsWidth = rowsWidth
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
        .frame(maxWidth: rowsWidth ?? PageLayout.headerWidth, alignment: .leading)
    }
}

/// The width of a page's rows and their insets, measured where the list draws them, which a legacy scroller narrows.
struct PageRowsWidthKey: PreferenceKey {
    static let defaultValue: CGFloat? = nil

    static func reduce(value: inout CGFloat?, nextValue: () -> CGFloat?) {
        if let next = nextValue() { value = next }
    }
}

extension View {
    /// Reports this view's width as the page's rows and their insets, for the header to take.
    func reportsPageRowsWidth() -> some View {
        background(GeometryReader { Color.clear.preference(key: PageRowsWidthKey.self, value: $0.size.width) })
    }
}

extension PageHeader where Accessory == EmptyView {
    init(title: String, caption: String? = nil) {
        self.init(title: title, caption: caption) { EmptyView() }
    }
}

/// A page row's fill, with 12 continuous corners: Surface/400 under the pointer and Button/Ghost/Pressed while pressed,
/// with no hairline or shadow, which on the page's Surface/500 would draw a box.
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
/// typed the 13 dismiss button in Icon/Tertiary, which reads as enabled in dark mode where Icon/Subtle did not. Escape clears it and gives up the focus.
/// A field that takes something other than a search, such as Friends' handle, puts its `leading` mark in Text/Tertiary
/// where the icon goes, and Return calls `onSubmit`.
///
/// A trailing accessory, such as the sidebar's 28 filter button, sits 8 after the text or the dismiss button, with 4 at
/// the end in place of 11, so a 28 circle's centre falls on the end cap's own (navigation round 2, section 3.4).
struct SearchCapsule<Accessory: View>: View {
    let placeholder: String
    @Binding var text: String
    var isFocused: FocusState<Bool>.Binding
    /// The dismiss button's accessible label.
    let clearLabel: String
    var width: CGFloat?
    var leading: String?
    var onSubmit: (() -> Void)?
    @ViewBuilder var accessory: () -> Accessory

    init(placeholder: String, text: Binding<String>, isFocused: FocusState<Bool>.Binding, clearLabel: String,
         width: CGFloat? = nil, leading: String? = nil, onSubmit: (() -> Void)? = nil,
         @ViewBuilder accessory: @escaping () -> Accessory) {
        self.placeholder = placeholder
        _text = text
        self.isFocused = isFocused
        self.clearLabel = clearLabel
        self.width = width
        self.leading = leading
        self.onSubmit = onSubmit
        self.accessory = accessory
    }

    private var hasAccessory: Bool { Accessory.self != EmptyView.self }

    var body: some View {
        HStack(spacing: 8) {
            if let leading {
                Text(leading)
                    .gallopText(.bodyM, color: SemanticColor.textTertiary)
                    .accessibilityHidden(true)
            } else {
                GallopIconView(icon: .search, fallbackSystemName: "magnifyingglass", size: 13)
                    .foregroundStyle(SemanticColor.iconTertiary)
            }
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .focused(isFocused)
                .onSubmit { onSubmit?() }
                .onExitCommand {
                    // Escape clears the filter and returns focus to the list.
                    text = ""
                    isFocused.wrappedValue = false
                }
                .accessibilityLabel(placeholder)
            if !text.isEmpty {
                Button { text = "" } label: {
                    GallopIconView(icon: .dismiss, fallbackSystemName: "xmark.circle.fill", size: 13)
                        .foregroundStyle(SemanticColor.iconTertiary)
                }
                .buttonStyle(.plain)
                .macAccessibleAction(label: clearLabel) { text = "" }
            }
            accessory()
        }
        .padding(.leading, 11)
        .padding(.trailing, hasAccessory ? 4 : 11)
        .frame(width: width)
        .frame(height: 36)
        .background(SemanticColor.textfieldDefault, in: Capsule())
        .overlay {
            Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
    }
}

extension SearchCapsule where Accessory == EmptyView {
    init(placeholder: String, text: Binding<String>, isFocused: FocusState<Bool>.Binding, clearLabel: String,
         width: CGFloat? = nil, leading: String? = nil, onSubmit: (() -> Void)? = nil) {
        self.init(placeholder: placeholder, text: text, isFocused: isFocused, clearLabel: clearLabel, width: width,
                  leading: leading, onSubmit: onSubmit) { EmptyView() }
    }
}

/// The app's 40 text field, as Settings and first run draw theirs: Body M on a Textfield/Default capsule with a 1-point
/// Border/Default edge drawn inside it, and 13 at the sides. A leading mark, such as a handle's "@", sits in
/// Text/Tertiary, and a trailing view 11 from the end. A sheet's search field puts the 13 search icon in Icon/Tertiary
/// 8 before the text, as `SearchCapsule` draws it. Return calls `onSubmit`.
struct CapsuleField<Trailing: View>: View {
    let placeholder: String
    @Binding var text: String
    var leading: String?
    var showsSearchIcon = false
    var isSecure = false
    /// Escape clears the text, as `SearchCapsule`'s does, while there is text to clear (navigation round 2, section
    /// 6.2). With none, Escape is left to the sheet's Cancel.
    var clearsOnEscape = false
    var onSubmit: (() -> Void)?
    @ViewBuilder let trailing: Trailing

    init(placeholder: String, text: Binding<String>, leading: String? = nil, showsSearchIcon: Bool = false,
         isSecure: Bool = false, clearsOnEscape: Bool = false, onSubmit: (() -> Void)? = nil,
         @ViewBuilder trailing: () -> Trailing = { EmptyView() }) {
        self.placeholder = placeholder
        _text = text
        self.leading = leading
        self.showsSearchIcon = showsSearchIcon
        self.isSecure = isSecure
        self.clearsOnEscape = clearsOnEscape
        self.onSubmit = onSubmit
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: showsSearchIcon ? 8 : 2) {
            if showsSearchIcon {
                GallopIconView(icon: .search, fallbackSystemName: "magnifyingglass", size: 13)
                    .foregroundStyle(SemanticColor.iconTertiary)
            } else if let leading {
                Text(leading).gallopText(.bodyM, color: SemanticColor.textTertiary)
            }
            Group {
                if isSecure {
                    SecureField(placeholder, text: $text)
                } else {
                    TextField(placeholder, text: $text)
                }
            }
            .textFieldStyle(.plain)
            .gallopText(.bodyM, color: SemanticColor.textPrimary)
            .onSubmit { onSubmit?() }
            .onExitCommand(perform: clearsOnEscape && !text.isEmpty ? { text = "" } : nil)
            .accessibilityLabel(placeholder)
            trailing
        }
        .padding(.leading, 13)
        .padding(.trailing, Trailing.self == EmptyView.self ? 13 : 11)
        .frame(height: 40)
        .background(SemanticColor.textfieldDefault, in: Capsule())
        .overlay { Capsule().strokeBorder(SemanticColor.borderDefault, lineWidth: 1) }
    }
}

/// A page row's actions behind a 24 ⋯, drawn as the Lobby's room cards draw theirs: the 14 ellipsis in Icon/Tertiary.
/// It shows while the row is hovered and keeps its place when hidden, so the row's columns don't move.
struct PageRowMoreMenu<Items: View>: View {
    let isVisible: Bool
    let accessibilityLabel: String
    @ViewBuilder let items: () -> Items

    var body: some View {
        Menu(content: items) {
            GallopIconView(icon: .ellipsis, fallbackSystemName: "ellipsis", size: 14)
                .foregroundStyle(SemanticColor.iconTertiary)
                .frame(width: 24, height: 24)
                .contentShape(Rectangle())
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        // A borderless menu tints its label with the accent colour, so the ⋯ takes Icon/Tertiary as its tint.
        .tint(SemanticColor.iconTertiary)
        .fixedSize()
        .frame(width: 24, height: 24)
        .opacity(isVisible ? 1 : 0)
        .accessibilityLabel(accessibilityLabel)
    }
}

/// Text that is a link, as first run's `SetupLink` draws one: in its colour now, and underlined under the pointer,
/// where it takes `hoverColor` when one is given. Its press area is the text's own. The room header's caption opens
/// People… with it, and a Team row's name opens GitHub (navigation note, sections 8 and 9.3).
struct TextLink: View {
    let title: String
    let style: GallopTextStyle
    let color: Color
    var hoverColor: Color?
    let action: () -> Void
    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            Text(title)
                .underline(isHovering)
                .gallopText(style, color: isHovering ? hoverColor ?? color : color)
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
    }
}
