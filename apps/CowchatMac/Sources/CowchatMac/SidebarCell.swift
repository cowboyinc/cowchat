import SwiftUI

/// What a live session's room shows in its cell's state slot.
enum SidebarMark: Equatable {
    case working
    /// The session is stopped at a prompt in the named app.
    case waiting(String)
}

/// One room's cell in the sidebar's list (navigation note, section 3.4), drawn as the Cowboy app's chat rail draws a
/// chat (`proto-site/src/components/chrome/sidebar.css:332-363`): one line, 36 tall, a capsule with the sidebar's row
/// fill, and no avatar, preview, lock or Slack badge.
///
/// Within the 264 row: 10, the 18 state slot on the nav icons' column, 6, the 16 place slot, 8, the name in Body S, 8,
/// the 40 time slot in Caption, which grows for a wider date, the 12 dot slot and 10. A room inside a company room sits 24 further in after the state
/// slot, so its place mark stands under its parent's name. A company room with rooms inside it draws its disclosure
/// chevron in the state slot, as a button of its own over the cell with a 24 by 36 press area.
///
/// This Mac's mark is SF Symbol `laptopcomputer`, since the Mac's Gallop set has no device glyph (`Bundled/Icons/svg`).
struct SidebarCell<Menu: View>: View {
    let name: String
    let placeIcon: SidebarPlaceIcon
    var connection = SidebarConnection.up
    var mark: SidebarMark?
    var depth = 0
    var disclosure: SidebarRow.Disclosure?
    let time: String
    var showsDot = false
    let isSelected: Bool
    let open: () -> Void
    var toggle: (() -> Void)?
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

            if let disclosure, let toggle {
                Button(action: toggle) {
                    GallopIconView(icon: disclosure == .open ? .chevronDownExtraSmall : .chevronRightExtraSmall,
                                   fallbackSystemName: disclosure == .open ? "chevron.down" : "chevron.right",
                                   size: 10)
                        .foregroundStyle(SemanticColor.iconTertiary)
                        .frame(width: 16, height: 16)
                        .frame(width: 24, height: 36)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                // The state slot's centre is 19 in, so the 24 press area starts 7 in.
                .padding(.leading, 7)
                .help(disclosureLabel(disclosure))
                .macAccessibleAction(label: disclosureLabel(disclosure), action: toggle)
            }
        }
        .onHover { isHovering = $0 }
    }

    private var content: some View {
        HStack(spacing: 0) {
            stateSlot
                .frame(width: 18, height: 36)
            Color.clear.frame(width: 6 + CGFloat(depth) * 24, height: 1)
            placeSlot
                .frame(width: 16, height: 16)
                .help(placeIcon.help(connection: connection))
            Text(name)
                .gallopText(.bodyS, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.leading, 8)
                .padding(.trailing, 8)
            // A date such as "Sep 30" is wider than the 40 slot in Caption, so the slot grows to its time and the
            // name gives way.
            Text(time)
                .gallopText(.caption, color: SemanticColor.textTertiary)
                .lineLimit(1)
                .fixedSize()
                .frame(minWidth: 40, alignment: .trailing)
            ZStack {
                if showsDot {
                    Circle()
                        .fill(Palette.nugget500)
                        .frame(width: 7, height: 7)
                }
            }
            .frame(width: 12)
        }
        .padding(.horizontal, 10)
        .frame(height: 36)
    }

    @ViewBuilder
    private var stateSlot: some View {
        switch mark {
        case .working?:
            SessionWorkingMark()
                // copy:
                .help("Working")
        case let .waiting(app)?:
            GallopIconView(icon: .warning, fallbackSystemName: "exclamationmark.triangle.fill", size: 16)
                .foregroundStyle(SemanticColor.notificationErrorTextDefault)
                // copy:
                .help("Waiting in \(app)")
        case nil:
            Color.clear
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
    private func disclosureLabel(_ disclosure: SidebarRow.Disclosure) -> String {
        disclosure == .open ? "Hide rooms in \(name)" : "Show rooms in \(name)"
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
        switch mark {
        case .working?: state = "working"
        case let .waiting(app)?: state = "waiting in \(app)"
        case nil: state = nil
        }
        return [name, place, state, time.isEmpty ? nil : time].compactMap { $0 }.joined(separator: ", ")
    }

    // copy:
    private var accessibilityValue: String? {
        let parts = [
            showsDot ? "Unread" : nil,
            disclosure.map { $0 == .open ? "expanded" : "collapsed" },
            isSelected ? "selected" : nil,
        ].compactMap { $0 }
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
