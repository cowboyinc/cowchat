import AppKit
import SwiftUI

/// A team the account belongs to, as `GET /social/v1/teams` lists it. The server has no teams yet: Chad's plan
/// (task 12) asks him what makes one, and proposes the account's company rooms. The prototype's simulated server
/// lists the account's top-level company rooms, Cowboy and District.
struct SocialTeam: Decodable, Identifiable, Equatable {
    let id: String
    let name: String
}

/// The account's teams and the one chosen in the sidebar footer's switcher. Choosing a team only stores the choice on
/// this Mac, per account, for now. After the Inbox and Team work merges, the Team page lists the chosen team's people
/// and New room puts a room inside the chosen team's company room.
@MainActor
final class TeamChoice: ObservableObject {
    @Published private(set) var teams: [SocialTeam] = []
    @Published private(set) var chosenID: String?
    private var accountID: String?
    private let defaults: UserDefaults
    static let storageKey = "CowchatMac.chosenTeam"

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    /// The chosen team, or the first one when none was chosen or the chosen one is gone.
    var chosen: SocialTeam? { teams.first { $0.id == chosenID } ?? teams.first }

    /// Lists the signed-in account's teams and reads its stored choice. A server without the route lists none, and
    /// the footer then draws no switcher.
    func load(for session: SocialSession?) async {
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
        chosenID = team.id
        defaults.set(team.id, forKey: Self.key(for: accountID))
    }

    static func key(for accountID: String) -> String { "\(storageKey).\(accountID)" }
}

/// The sidebar footer's left side. With teams, the chosen team's room icon sits beside the switcher, which sits over
/// the footer's connection line, as the demo's footer has them; without, the connection line is alone, as before.
struct TeamFooterLead<Connection: View>: View {
    @ObservedObject var teams: TeamChoice
    @ViewBuilder let connection: () -> Connection
    @AppStorage(RoomIconStyle.storageKey) private var iconStyle = RoomIconStyle.fallback.rawValue

    var body: some View {
        if let chosen = teams.chosen {
            HStack(spacing: 10) {
                RoomAvatar(name: chosen.name, size: 28, accented: false)
                VStack(alignment: .leading, spacing: 1) {
                    TeamSwitcher(teams: teams, iconStyle: RoomIconStyle(rawValue: iconStyle) ?? .fallback)
                    connection()
                }
            }
        } else {
            connection()
        }
    }
}

/// The team switcher over the footer's connection line, as the demo draws it: the chosen team's name and a chevron,
/// opening the system menu of the account's teams, each with its room icon and a tick on the chosen one. With one
/// team there is nothing to switch, so only its name shows.
struct TeamSwitcher: View {
    @ObservedObject var teams: TeamChoice
    let iconStyle: RoomIconStyle

    var body: some View {
        if let chosen = teams.chosen {
            if teams.teams.count > 1 {
                Menu {
                    ForEach(teams.teams) { team in
                        // A toggle per team, so the menu ticks the chosen one.
                        Toggle(isOn: Binding(get: { team.id == chosen.id }, set: { _ in teams.choose(team) })) {
                            Label {
                                Text(team.name)
                            } icon: {
                                Image(nsImage: TeamIconImage.make(team, style: iconStyle))
                            }
                        }
                    }
                } label: {
                    TeamSwitcherLabel(name: chosen.name)
                }
                .menuStyle(.button)
                .buttonStyle(.plain)
                .menuIndicator(.hidden)
                .fixedSize()
                // copy:
                .help("Switch team")
                .accessibilityLabel("Team, \(chosen.name)")
            } else {
                Text(chosen.name).gallopText(.bodySStrong, color: SemanticColor.textPrimary).lineLimit(1)
            }
        }
    }
}

/// The switcher's label, as the demo draws it: the team's name in Body S Strong and the app's down chevron in
/// Icon/Tertiary, filled on hover.
private struct TeamSwitcherLabel: View {
    let name: String
    @State private var isHovering = false

    var body: some View {
        HStack(spacing: 2) {
            Text(name).gallopText(.bodySStrong, color: SemanticColor.textPrimary).lineLimit(1)
            GallopIconView(icon: .chevronDownExtraSmall, fallbackSystemName: "chevron.down", size: 16)
                .foregroundStyle(SemanticColor.iconTertiary)
        }
        .padding(.leading, 6)
        .padding(.trailing, 2)
        .frame(height: 22)
        .background(isHovering ? SemanticColor.buttonGhostHover : .clear,
                    in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .padding(.leading, -6)
        .contentShape(Rectangle())
        .onHover { isHovering = $0 }
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
