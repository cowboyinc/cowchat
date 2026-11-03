import SwiftUI

/// A live session's state, which its room's cell shows in its end slot.
enum SidebarMark: Equatable {
    case working
    /// The session is stopped at a prompt in the named app.
    case waiting(String)
}

/// What a cell's end slot holds (navigation round 2, section 3.2): one thing, in this order. A session waiting at a
/// prompt shows the warning, a working one the turning mark, an unread room the dot, and any other room its time. A mark
/// hides the dot until the session goes idle. While the account's connection is down a session's state is no longer
/// known, so its room shows what it would with no mark.
enum SidebarCellEnd: Equatable {
    case waiting(String)
    case working
    case unread
    case time(String)

    static func content(mark: SidebarMark?, isUnread: Bool, time: String, connectionDown: Bool) -> SidebarCellEnd {
        switch connectionDown ? nil : mark {
        case let .waiting(app)?: return .waiting(app)
        case .working?: return .working
        case nil: return isUnread ? .unread : .time(time)
        }
    }
}

/// One room's cell in the sidebar's list (navigation round 2, section 3.2), drawn as the Cowboy app's chat rail draws a
/// chat (`proto-site/src/components/chrome/sidebar.css:332-363`): one line, 36 tall, a capsule with the sidebar's row
/// fill, and no avatar, preview, lock or Slack badge.
///
/// Within the 264 row: 10, the 18 place slot on the Inbox and Team icons' column, 10, the name in Body S where their
/// titles start, 8, and the end slot, at least 40 and ending 10 from the row's end. The end slot holds one thing
/// (`SidebarCellEnd`): a mark or the dot centred in a 16 square at its end, so on row x 246, or the time in Caption
/// ending at 254. A date wider than 40 grows the slot leftwards and the name gives way.
///
/// This Mac's mark is SF Symbol `laptopcomputer`, since the Mac's Gallop set has no device glyph (`Bundled/Icons/svg`).
///
/// A live session's cell under the pointer hides its end slot and shows the Add person button there (navigation note,
/// section 7.4), as Cowboy's rail swaps its end slot for the row's ⋯. The button sits over the cell, outside its
/// button, so pressing it does not also open the room.
struct SidebarCell<Menu: View>: View {
    let name: String
    let placeIcon: SidebarPlaceIcon
    var connection = SidebarConnection.up
    var mark: SidebarMark?
    let time: String
    var isUnread = false
    let isSelected: Bool
    let open: () -> Void
    /// Add person, on the cell of one of the person's live sessions' rooms.
    var addPerson: (() -> Void)?
    @ViewBuilder var menu: () -> Menu
    @State private var isHovering = false

    var body: some View {
        ZStack(alignment: .leading) {
            Button(action: open) {
                content
                    .background(SidebarRowBackground(state: .init(isSelected: isSelected, isHovering: isHovering)))
                    .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .contextMenu { menu() }
            .macAccessibleAction(label: accessibilityLabel, value: accessibilityValue, action: open)

            if let addPerson {
                // Its 28 press area ends 4 from the row's end, so its glyph is centred on the marks' 246.
                AddPersonButton(name: name, isShown: isHovering, action: addPerson)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .padding(.trailing, 4)
            }
        }
        .onHover { isHovering = $0 }
    }

    private var end: SidebarCellEnd {
        SidebarCellEnd.content(mark: mark, isUnread: isUnread, time: time, connectionDown: connection != .up)
    }

    private var content: some View {
        HStack(spacing: 0) {
            placeSlot
                .frame(width: 18, height: 36)
                .help(placeIcon.help(connection: connection))
            Text(name)
                .gallopText(.bodyS, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.leading, 10)
                .padding(.trailing, 8)
            endSlot
                .frame(minWidth: 40, alignment: .trailing)
                .opacity(addPerson != nil && isHovering ? 0 : 1)
        }
        .padding(.horizontal, 10)
        .frame(height: 36)
    }

    @ViewBuilder
    private var endSlot: some View {
        switch end {
        case let .waiting(app):
            GallopIconView(icon: .warning, fallbackSystemName: "exclamationmark.triangle.fill", size: 16)
                .foregroundStyle(SemanticColor.notificationErrorTextDefault)
                // copy:
                .help("Waiting in \(app)")
        case .working:
            SessionWorkingMark()
                .frame(width: 16, height: 16)
                // copy:
                .help("Working")
        case .unread:
            Circle()
                .fill(Palette.nugget500)
                .frame(width: 7, height: 7)
                .frame(width: 16, height: 16)
        case let .time(time):
            Text(time)
                .gallopText(.caption, color: SemanticColor.textTertiary)
                .lineLimit(1)
                .fixedSize()
        }
    }

    @ViewBuilder
    private var placeSlot: some View {
        switch placeIcon {
        case .thisMac:
            Image(systemName: "laptopcomputer")
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(placeTint)
        case .thisMacFailed:
            Image(systemName: "laptopcomputer.trianglebadge.exclamationmark")
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(SemanticColor.notificationErrorTextDefault)
        case .online:
            GallopIconView(icon: .network, fallbackSystemName: "globe", size: 16)
                .foregroundStyle(placeTint)
        case .onlineDown:
            GallopIconView(icon: .networkDown, fallbackSystemName: "wifi.slash", size: 16)
                .foregroundStyle(SemanticColor.iconTertiary)
        case .onlineFailed:
            GallopIconView(icon: .networkError, fallbackSystemName: "wifi.exclamationmark", size: 16)
                .foregroundStyle(SemanticColor.notificationErrorTextDefault)
        }
    }

    /// The place mark's tint, which a selected cell turns as a selected nav row turns its icon.
    private var placeTint: Color {
        isSelected ? SemanticColor.surfaceGlassOnIconDefault : SemanticColor.iconTertiary
    }

    // copy:
    /// The name, the place, the state and the time, as "parity-sign-in, on this Mac, waiting in Codex, 1h".
    private var accessibilityLabel: String {
        let place: String
        switch placeIcon {
        case .thisMac: place = "on this Mac"
        case .online: place = "online"
        case .thisMacFailed, .onlineDown, .onlineFailed: place = placeIcon.help(connection: connection).lowercased()
        }
        let state: String?
        switch end {
        case .working: state = "working"
        case let .waiting(app): state = "waiting in \(app)"
        case .unread, .time: state = nil
        }
        return [name, place, state, time.isEmpty ? nil : time].compactMap { $0 }.joined(separator: ", ")
    }

    // copy:
    private var accessibilityValue: String? {
        let parts = [isUnread ? "Unread" : nil, isSelected ? "selected" : nil].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }
}

extension SidebarCell where Menu == EmptyView {
    /// A cell with no right-click menu, as a To join result.
    init(name: String, placeIcon: SidebarPlaceIcon, connection: SidebarConnection = .up, time: String,
         isSelected: Bool, open: @escaping () -> Void) {
        self.init(name: name, placeIcon: placeIcon, connection: connection, time: time, isSelected: isSelected,
                  open: open, menu: { EmptyView() })
    }
}

/// The Add person button at the end of a live session's cell (add-new-company note, section 1): SF Symbol
/// `person.badge.plus` at 13 medium, since the Mac's Gallop set has no person-add glyph (`Bundled/Icons/svg`), in
/// Icon/Tertiary and Icon/Primary under the pointer, centred in a 28 by 36 press area. It stays laid out while hidden,
/// with no hit testing, so its accessible action and the tour can reach it.
private struct AddPersonButton: View {
    let name: String
    let isShown: Bool
    let action: () -> Void
    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: "person.badge.plus")
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(isHovering ? SemanticColor.iconPrimary : SemanticColor.iconTertiary)
                .opacity(isShown ? 1 : 0)
                .frame(width: 28, height: 36)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .allowsHitTesting(isShown)
        .onHover { isHovering = $0 }
        // copy:
        .help("Add person")
        // copy:
        .macAccessibleAction(label: "Add person to \(name)", action: action)
    }
}
