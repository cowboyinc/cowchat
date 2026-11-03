import SwiftUI

/// The frame of the sheets that start something, New room, New session, Join with invite code and Add person
/// (add-new-company note, section 0), drawn from the Join sheet's: 480 wide on Surface/600 with 26 padding and its
/// parts 20 apart, its height fitting its content. The title is Heading 4 on one line, truncated at the tail, with
/// nothing under it. An error is one Caption line in Text/Error 8 above the buttons.
///
/// A sheet never grows past the window's height under the toolbar (`sheetHeightLimit`). When it would, its list of
/// people gives way and scrolls, so the buttons stay in the window at its smallest size.
struct SheetFrame<Content: View, Buttons: View>: View {
    let title: String
    var error: String?
    @ViewBuilder let content: () -> Content
    @ViewBuilder let buttons: () -> Buttons
    @Environment(\.sheetHeightLimit) private var heightLimit

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text(title)
                .gallopText(.h4, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            content()
            VStack(alignment: .leading, spacing: 8) {
                if let error {
                    Text(error)
                        .gallopText(.caption, color: SemanticColor.textError)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                buttons()
            }
        }
        .padding(26)
        .frame(width: 480, alignment: .leading)
        .frame(maxHeight: heightLimit ?? .infinity, alignment: .top)
        .fixedSize(horizontal: false, vertical: true)
        .background(SemanticColor.surface600)
    }
}

private struct SheetHeightLimitKey: EnvironmentKey {
    static let defaultValue: CGFloat? = nil
}

extension EnvironmentValues {
    /// How tall a starting sheet may be: the window's height under the toolbar, less a margin.
    var sheetHeightLimit: CGFloat? {
        get { self[SheetHeightLimitKey.self] }
        set { self[SheetHeightLimitKey.self] = newValue }
    }
}

/// A part of a sheet under its label, as the Join sheet's "Add a session": Caption in Text/Tertiary, 4 above the
/// field or rows it names.
struct SheetGroup<Content: View>: View {
    let label: String
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .gallopText(.caption, color: SemanticColor.textTertiary)
                .lineLimit(1)
            content()
        }
    }
}

/// Cancel and the act at the right, 10 apart, as 36 pills with the act prominent. Escape is Cancel and Return is the
/// act. While busy, the act shows the small spinner before its label and both are disabled.
struct SheetButtons: View {
    let act: String
    var isEnabled = true
    var busy = false
    let cancel: () -> Void
    let perform: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Spacer(minLength: 0)
            // copy:
            Button("Cancel", action: cancel)
                .keyboardShortcut(.cancelAction)
                .macAccessibleAction(label: "Cancel", isEnabled: !busy, action: cancel)
            Button(action: perform) {
                HStack(spacing: 6) {
                    if busy {
                        ProgressView().controlSize(.small).frame(width: 16, height: 16)
                    }
                    Text(act)
                }
            }
            .buttonStyle(CowchatPillButtonStyle(prominent: true))
            .keyboardShortcut(.defaultAction)
            .disabled(!isEnabled)
            .macAccessibleAction(label: act, isEnabled: isEnabled && !busy, action: perform)
        }
        .buttonStyle(CowchatPillButtonStyle())
        .disabled(busy)
    }
}

/// One choice in a sheet's list, the Join sheet's session row made shared: 44 tall, 16 in, a 24 mark slot, 12, the
/// name in Body M, a 100 fact column in Body S Text/Tertiary aligned right, 12, a 16 slot that holds the tick on a
/// chosen row, then 16. The hover is the page rows'. Rows are separated by `PageRowSeam`.
///
/// Given `remove` in place of `choose`, the row is not a button and its 16 slot holds Remove (navigation round 2,
/// section 6.2), as New room's picked people have it.
struct SheetChoiceRow<Mark: View>: View {
    /// Where the names start: the 16 in, the 24 mark and the 12 after it.
    static var nameLeading: CGFloat { 16 + 24 + 12 }

    let name: String
    var fact: String?
    var isChosen = false
    var isEnabled = true
    private let choose: (() -> Void)?
    private let remove: (() -> Void)?
    @ViewBuilder let mark: () -> Mark

    init(name: String, fact: String? = nil, isChosen: Bool, isEnabled: Bool = true,
         choose: @escaping () -> Void, @ViewBuilder mark: @escaping () -> Mark) {
        self.name = name
        self.fact = fact
        self.isChosen = isChosen
        self.isEnabled = isEnabled
        self.choose = choose
        remove = nil
        self.mark = mark
    }

    init(name: String, fact: String? = nil, remove: @escaping () -> Void, @ViewBuilder mark: @escaping () -> Mark) {
        self.name = name
        self.fact = fact
        choose = nil
        self.remove = remove
        self.mark = mark
    }

    var body: some View {
        if let remove {
            row { SheetRemoveButton(name: name, remove: remove) }
        } else {
            let choose = choose ?? {}
            Button(action: choose) {
                row {
                    if isChosen {
                        GallopIconView(icon: .checkmark, fallbackSystemName: "checkmark", size: 16)
                            .foregroundStyle(SemanticColor.iconPrimary)
                    }
                }
                .contentShape(Rectangle())
                .opacity(isEnabled ? 1 : 0.45)
            }
            .buttonStyle(PageRowButtonStyle())
            .disabled(!isEnabled)
            // copy:
            .macAccessibleAction(label: [name, fact].compactMap { $0 }.joined(separator: ", "),
                                 value: isChosen ? "chosen" : nil, isEnabled: isEnabled, action: choose)
        }
    }

    private func row(@ViewBuilder end: () -> some View) -> some View {
        HStack(spacing: 0) {
            ZStack { mark() }
                .frame(width: 24, height: 24)
            Spacer().frame(width: 12)
            Text(name)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let fact {
                Text(fact)
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(width: 100, alignment: .trailing)
            }
            Spacer().frame(width: 12)
            ZStack { end() }
                .frame(width: 16, height: 16)
        }
        .padding(.horizontal, 16)
        .frame(height: 44)
    }
}

extension SheetChoiceRow where Mark == EmptyView {
    /// A row with nothing in its mark slot, as the Join sheet's "No session".
    init(name: String, fact: String? = nil, isChosen: Bool, isEnabled: Bool = true, choose: @escaping () -> Void) {
        self.init(name: name, fact: fact, isChosen: isChosen, isEnabled: isEnabled, choose: choose) { EmptyView() }
    }
}

/// Remove in a picked person's tick slot: Gallop's `dismiss` at 16 in Icon/Tertiary, Icon/Primary under the pointer,
/// with a 24 by 24 press area centred on the slot.
private struct SheetRemoveButton: View {
    let name: String
    let remove: () -> Void
    @State private var isHovering = false

    var body: some View {
        Button(action: remove) {
            GallopIconView(icon: .dismiss, fallbackSystemName: "xmark.circle.fill", size: 16)
                .foregroundStyle(isHovering ? SemanticColor.iconPrimary : SemanticColor.iconTertiary)
                .frame(width: 24, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .fixedSize()
        .onHover { isHovering = $0 }
        // copy:
        .help("Remove")
        // copy:
        .macAccessibleAction(label: "Remove \(name)", action: remove)
    }
}

/// A list's one line when it has no rows, 44 tall in Body M Text/Tertiary, 16 in, as "No friends found".
struct SheetEmptyRow: View {
    let text: String

    var body: some View {
        Text(text)
            .gallopText(.bodyM, color: SemanticColor.textTertiary)
            .lineLimit(1)
            .padding(.leading, 16)
            .frame(maxWidth: .infinity, minHeight: 44, maxHeight: 44, alignment: .leading)
    }
}

/// The 40 field as the label of a menu (add-new-company note, section 0): Body M on Textfield/Default with a 1-point
/// Border/Default edge, 13 in at the start, a leading mark 8 before the text, and Gallop's `chevronDown` at 16 in
/// Icon/Tertiary 11 from the end. The menu is the system menu. It is built as a plain button-style menu with the
/// indicator hidden, so the label draws as it is.
///
/// In a page header it is 36, as the search beside it is, with its 20 avatar 8 in so it sits centred in the end cap, a
/// title of at most `titleMaxWidth`, and Gallop's Dropdown hover: Textfield/Hover with Border/Hover under the pointer
/// (navigation round 2, section 4.3).
struct SheetMenuField<Leading: View, Items: View>: View {
    let title: String
    /// Draws the title in Text/Tertiary, as a placeholder.
    var isPlaceholder = false
    var isEnabled = true
    var help: String?
    let accessibilityLabel: String
    var height: CGFloat = 40
    var leadingInset: CGFloat = 13
    var titleMaxWidth: CGFloat?
    var hovers = false
    @ViewBuilder let leading: () -> Leading
    @ViewBuilder let items: () -> Items
    @State private var isHovering = false

    var body: some View {
        Menu(content: items) {
            HStack(spacing: 8) {
                leading()
                Text(title)
                    .gallopText(.bodyM, color: isPlaceholder ? SemanticColor.textTertiary : SemanticColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: titleMaxWidth ?? .infinity, alignment: .leading)
                GallopIconView(icon: .chevronDown, fallbackSystemName: "chevron.down", size: 16)
                    .foregroundStyle(SemanticColor.iconTertiary)
            }
            .padding(.leading, leadingInset)
            .padding(.trailing, 11)
            .frame(height: height)
            .background(isHovered ? SemanticColor.textfieldHover : SemanticColor.textfieldDefault, in: Capsule())
            .overlay {
                Capsule().strokeBorder(isHovered ? SemanticColor.borderHover : SemanticColor.borderDefault,
                                       lineWidth: 1)
            }
            .contentShape(Capsule())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .onHover { isHovering = $0 }
        .disabled(!isEnabled)
        .opacity(isEnabled ? 1 : 0.45)
        .help(help ?? "")
        .accessibilityLabel("\(accessibilityLabel), \(title)")
    }

    private var isHovered: Bool { hovers && isHovering && isEnabled }
}

extension SheetMenuField where Leading == EmptyView {
    init(title: String, isPlaceholder: Bool = false, isEnabled: Bool = true, help: String? = nil,
         accessibilityLabel: String, @ViewBuilder items: @escaping () -> Items) {
        self.init(title: title, isPlaceholder: isPlaceholder, isEnabled: isEnabled, help: help,
                  accessibilityLabel: accessibilityLabel, leading: { EmptyView() }, items: items)
    }
}

/// What a sheet's list of people holds (add-new-company note, section 1): the people picked, in the order they were
/// picked, then the rest in the server's order. A search keeps the picked rows at the top whether or not they match
/// and lists the rest that match by name or handle.
enum PeopleList {
    /// The rows a list shows with its more row closed and nothing typed.
    static let limit = 5

    static func rows(_ people: [SocialAccount], picked: [String], query: String) -> [SocialAccount] {
        let first = picked.compactMap { id in people.first { $0.id == id } }
        let rest = people.filter { !picked.contains($0.id) }
        let typed = query.trimmingCharacters(in: .whitespaces)
        return first + (typed.isEmpty ? rest : rest.filter { FriendSearch.holds($0, query: typed) })
    }

    /// The rows' area: as tall as the unfiltered list needs, up to five 44 rows with the 8 seams between them, and the
    /// 32 more row when there are more than five. It never changes while the person types.
    static func height(count: Int) -> CGFloat {
        let shown = max(1, min(count, limit))
        return CGFloat(shown) * 44 + CGFloat(shown - 1) * 8 + (count > limit ? 32 : 0)
    }

    /// New room's suggestions while the person types (navigation round 2, section 6.2): the people who match by name
    /// or handle and are not picked, in the server's order, at most `limit`.
    static func suggestions(_ people: [SocialAccount], picked: [String], query: String,
                            limit: Int = 3) -> [SocialAccount] {
        let typed = query.trimmingCharacters(in: .whitespaces)
        guard !typed.isEmpty else { return [] }
        return Array(people.filter { !picked.contains($0.id) && FriendSearch.holds($0, query: typed) }.prefix(limit))
    }

    /// The picks with `id` first, so New room shows the newest pick at the top. A person is picked once.
    static func picking(_ id: String, into picked: [String]) -> [String] {
        [id] + picked.filter { $0 != id }
    }

    /// What Return in New room's search does: pick the first suggestion, or with nothing typed, submit the sheet.
    enum Submit: Equatable {
        case pick(String)
        case submitEmpty
        case nothing
    }

    static func submit(_ people: [SocialAccount], picked: [String], query: String) -> Submit {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { return .submitEmpty }
        return suggestions(people, picked: picked, query: query, limit: 1).first.map { .pick($0.id) } ?? .nothing
    }

    // copy:
    /// The more row's words, with the list's noun: how many more it shows, or once open, how many it hides again.
    static func moreTitle(hidden: Int, showsAll: Bool, noun: (one: String, many: String)) -> String {
        guard showsAll else { return "Show \(hidden) more" }
        return "Hide \(hidden) \(hidden == 1 ? noun.one : noun.many)"
    }
}

/// A search field over a list of people that picks several (add-new-company note, sections 0 and 1): the 40 field with
/// the search icon, then 8 under it the rows, each the 24 avatar, the full name and the handle with its @, ticked once
/// picked. With nothing typed, five rows and the more row show. Return in the field picks the first match and clears
/// the field, and in an empty field it calls `submitEmpty`.
struct PeoplePicker: View {
    let people: [SocialAccount]
    @Binding var picked: [String]
    let placeholder: String
    /// The line when there are no people at all.
    let emptyLine: String
    var noun: (one: String, many: String) = ("friend", "friends")
    var submitEmpty: () -> Void = {}
    @State private var query = ""
    @State private var showsAll = false
    @FocusState private var isFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CapsuleField(placeholder: placeholder, text: $query, showsSearchIcon: true, onSubmit: submit)
                .focused($isFocused)
            ScrollView {
                VStack(spacing: 0) {
                    let rows = PeopleList.rows(people, picked: picked, query: query)
                    let isTyped = !query.trimmingCharacters(in: .whitespaces).isEmpty
                    let shown = isTyped || showsAll ? rows : Array(rows.prefix(PeopleList.limit))
                    ForEach(Array(shown.enumerated()), id: \.element.id) { index, person in
                        if index > 0 { PageRowSeam() }
                        SheetChoiceRow(name: person.displayName, fact: "@\(person.handle)",
                                       isChosen: picked.contains(person.id)) {
                            toggle(person.id)
                        } mark: {
                            SocialPersonAvatar(name: person.displayName, size: 24, letters: 1)
                        }
                    }
                    if people.isEmpty {
                        SheetEmptyRow(text: emptyLine)
                    } else if isTyped, !people.contains(where: { FriendSearch.holds($0, query: query) }) {
                        // copy:
                        SheetEmptyRow(text: "No \(noun.many) found")
                    }
                    if !isTyped, rows.count > PeopleList.limit {
                        SessionsMoreRow(title: PeopleList.moreTitle(hidden: rows.count - PeopleList.limit,
                                                                    showsAll: showsAll, noun: noun),
                                        indent: SheetChoiceRow<EmptyView>.nameLeading, surface: .page) {
                            showsAll.toggle()
                        }
                    }
                }
            }
            .scrollIndicators(.automatic)
            // As tall as the unfiltered list, or less when the window is too short for the sheet, down to one row.
            .frame(minHeight: 44, idealHeight: PeopleList.height(count: people.count),
                   maxHeight: PeopleList.height(count: people.count))
        }
        .onAppear { isFocused = true }
    }

    private func toggle(_ id: String) {
        if let index = picked.firstIndex(of: id) { picked.remove(at: index) } else { picked.append(id) }
    }

    private func submit() {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else {
            submitEmpty()
            return
        }
        guard let first = PeopleList.rows(people, picked: picked, query: query).first(where: { !picked.contains($0.id) })
        else { return }
        picked.append(first.id)
        query = ""
    }
}

/// New room's People (navigation round 2, section 6.2): the friend search, then 8 under it an area 148 tall, three 44
/// rows and their two 8 seams, whatever it shows, so the sheet keeps one height while the person types and picks.
/// When the window is too short for the sheet, the area gives up height first, down to one row.
///
/// With nothing typed the area shows the people picked, newest first, each with Remove, or "No one added". While the
/// person types it suggests up to three friends who match and are not picked. Pressing one, or Return for the first,
/// picks them and clears the field, and the focus stays in the field. Escape clears the text. Return with nothing
/// typed calls `submitEmpty`.
struct PickedPeople: View {
    let people: [SocialAccount]
    @Binding var picked: [String]
    let placeholder: String
    /// The line when there are no people at all.
    let emptyLine: String
    var submitEmpty: () -> Void = {}
    @State private var query = ""
    @FocusState private var isFocused: Bool

    static var areaHeight: CGFloat { 3 * 44 + 2 * 8 }

    private var isTyped: Bool { !query.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CapsuleField(placeholder: placeholder, text: $query, showsSearchIcon: true, clearsOnEscape: true,
                         onSubmit: submit) {
                if !query.isEmpty {
                    Button { query = "" } label: {
                        GallopIconView(icon: .dismiss, fallbackSystemName: "xmark.circle.fill", size: 13)
                            .foregroundStyle(SemanticColor.iconTertiary)
                    }
                    .buttonStyle(.plain)
                    // copy:
                    .macAccessibleAction(label: "Clear search") { query = "" }
                }
            }
            .focused($isFocused)
            ScrollView {
                VStack(spacing: 0) {
                    if people.isEmpty {
                        SheetEmptyRow(text: emptyLine)
                    } else if isTyped {
                        suggestions
                    } else {
                        pickedRows
                    }
                }
            }
            .scrollIndicators(.automatic)
            .frame(minHeight: 44, idealHeight: Self.areaHeight, maxHeight: Self.areaHeight, alignment: .top)
        }
    }

    @ViewBuilder private var suggestions: some View {
        let shown = PeopleList.suggestions(people, picked: picked, query: query)
        if shown.isEmpty {
            // copy:
            SheetEmptyRow(text: "No friends found")
        }
        ForEach(Array(shown.enumerated()), id: \.element.id) { index, person in
            if index > 0 { PageRowSeam() }
            SheetChoiceRow(name: person.displayName, fact: "@\(person.handle)", isChosen: false) {
                pick(person.id)
            } mark: {
                SocialPersonAvatar(name: person.displayName, size: 24, letters: 1)
            }
        }
    }

    @ViewBuilder private var pickedRows: some View {
        let shown = picked.compactMap { id in people.first { $0.id == id } }
        if shown.isEmpty {
            // copy:
            SheetEmptyRow(text: "No one added")
        }
        ForEach(Array(shown.enumerated()), id: \.element.id) { index, person in
            if index > 0 { PageRowSeam() }
            SheetChoiceRow(name: person.displayName, fact: "@\(person.handle)") {
                picked.removeAll { $0 == person.id }
            } mark: {
                SocialPersonAvatar(name: person.displayName, size: 24, letters: 1)
            }
        }
    }

    private func pick(_ id: String) {
        picked = PeopleList.picking(id, into: picked)
        query = ""
        isFocused = true
    }

    private func submit() {
        switch PeopleList.submit(people, picked: picked, query: query) {
        case let .pick(id): pick(id)
        case .submitEmpty: submitEmpty()
        case .nothing: break
        }
    }
}
