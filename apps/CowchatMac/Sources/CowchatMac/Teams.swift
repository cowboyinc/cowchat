import AppKit
import SwiftUI

/// A team the account belongs to, as `GET /social/v1/teams` lists it. The server has no teams yet: Chad's plan
/// (task 12) asks him what makes one, and proposes the account's company rooms. The prototype's simulated server
/// lists the account's top-level company rooms, Cowboy and District.
struct SocialTeam: Decodable, Identifiable, Equatable {
    let id: String
    let name: String
}

/// The account's teams and the one chosen in the sidebar's team row. The choice is kept on this Mac, per
/// account. The Team page lists the chosen team's people, and New room puts a room inside the chosen team's company
/// room. With no teams listed, both do what they did before.
@MainActor
final class TeamChoice: ObservableObject {
    @Published private(set) var teams: [SocialTeam] = []
    @Published private(set) var chosenID: String?
    /// Called when the chosen team changes, by a choice or a new list.
    var onChoiceChanged: (() -> Void)?
    private var accountID: String?
    private let defaults: UserDefaults
    static let storageKey = "CowchatMac.chosenTeam"

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    /// The chosen team, or the first one when none was chosen or the chosen one is gone.
    var chosen: SocialTeam? { teams.first { $0.id == chosenID } ?? teams.first }

    /// Lists the signed-in account's teams and reads its stored choice. A server without the route lists none, and
    /// the sidebar then draws no team row.
    func load(for session: SocialSession?) async {
        let before = chosen?.id
        defer { if chosen?.id != before { onChoiceChanged?() } }
        guard let session else {
            teams = []
            chosenID = nil
            accountID = nil
            return
        }
        struct Reply: Decodable { let teams: [SocialTeam] }
        let reply: Reply? = try? await SocialAPI(endpoint: session.endpoint, key: session.deviceKey).social(["teams"])
        guard !Task.isCancelled else { return }
        accountID = session.account.id
        teams = reply?.teams ?? []
        chosenID = defaults.string(forKey: Self.key(for: session.account.id))
    }

    func choose(_ team: SocialTeam) {
        guard let accountID, teams.contains(team) else { return }
        let before = chosen?.id
        chosenID = team.id
        defaults.set(team.id, forKey: Self.key(for: accountID))
        if chosen?.id != before { onChoiceChanged?() }
    }

    static func key(for accountID: String) -> String { "\(storageKey).\(accountID)" }
}

/// The sidebar's first row, once the account has teams (navigation note, section 3.2). It is drawn as a nav row: 36
/// tall, the chosen team's room icon at 20 centred on the nav icons' column, its name in Body S Strong, and with two or
/// more teams 2 and the 16 down chevron in Icon/Tertiary. With two or more it opens `TeamSwitcher`'s menu and takes the
/// row hover. With one there is nothing to switch, so it has no chevron, no hover and no menu.
struct SidebarTeamRow: View {
    @ObservedObject var teams: TeamChoice
    @AppStorage(RoomIconStyle.storageKey) private var iconStyle = RoomIconStyle.fallback.rawValue

    var body: some View {
        if let chosen = teams.chosen {
            if teams.teams.count > 1 {
                TeamSwitcher(teams: teams, iconStyle: RoomIconStyle(rawValue: iconStyle) ?? .fallback) { isHovering in
                    TeamSwitcherLabel(name: chosen.name, opensMenu: true, isHovering: isHovering)
                }
            } else {
                TeamSwitcherLabel(name: chosen.name, opensMenu: false, isHovering: false)
            }
        }
    }
}

/// The team switcher: the system menu of the account's teams, each with its room icon and a toggle that ticks the
/// chosen one, opened from `label`, which is told when the pointer is over it.
struct TeamSwitcher<Label: View>: View {
    @ObservedObject var teams: TeamChoice
    let iconStyle: RoomIconStyle
    @ViewBuilder let label: (Bool) -> Label
    @State private var isHovering = false

    var body: some View {
        if let chosen = teams.chosen {
            Menu {
                ForEach(teams.teams) { team in
                    // A toggle per team, so the menu ticks the chosen one.
                    Toggle(isOn: Binding(get: { team.id == chosen.id }, set: { _ in teams.choose(team) })) {
                        SwiftUI.Label {
                            Text(team.name)
                        } icon: {
                            Image(nsImage: TeamIconImage.make(team, style: iconStyle))
                        }
                    }
                }
            } label: {
                label(isHovering)
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .onHover { isHovering = $0 }
            // copy:
            .help("Switch team")
            .accessibilityLabel("Team, \(chosen.name)")
        }
    }
}

/// The team row's label, laid out as a nav row's: the team's room icon at 20 on the 18 icon column 10 in, 10, the name
/// in Body S Strong in Text/Primary, then 2 and the chevron when it opens the menu, on the sidebar's row fill.
private struct TeamSwitcherLabel: View {
    let name: String
    let opensMenu: Bool
    let isHovering: Bool

    var body: some View {
        HStack(spacing: 0) {
            RoomAvatar(name: name, size: 20, accented: false)
                .frame(width: 18)
            Text(name)
                .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .padding(.leading, 10)
            if opensMenu {
                GallopIconView(icon: .chevronDownExtraSmall, fallbackSystemName: "chevron.down", size: 16)
                    .foregroundStyle(SemanticColor.iconTertiary)
                    .padding(.leading, 2)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .frame(maxWidth: .infinity)
        .frame(height: 36)
        .background(SidebarRowBackground(state: .init(isSelected: false, isHovering: opensMenu && isHovering)))
        .contentShape(Rectangle())
    }
}

/// A team's icon as an image for the system menu, which draws images and not views: the room icon the sidebar would
/// draw for a room of that name.
enum TeamIconImage {
    @MainActor
    static func make(_ team: SocialTeam, style: RoomIconStyle, size: CGFloat = 16) -> NSImage {
        let renderer = ImageRenderer(content: RoomAvatar(name: team.name, size: size, accented: false,
                                                         styleOverride: style))
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        return renderer.nsImage ?? NSImage()
    }
}
