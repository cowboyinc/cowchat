import AppKit
import SwiftUI

/// A team the account belongs to, as `GET /social/v1/teams` lists it. The server has no teams yet: Chad's plan
/// (task 12) asks him what makes one, and proposes the account's company rooms. The prototype's simulated server
/// lists the account's top-level company rooms, Cowboy and District.
struct SocialTeam: Decodable, Identifiable, Equatable {
    let id: String
    let name: String
}

/// The account's teams and the one chosen in the Team page's team field. The choice is kept on this Mac, per account.
/// The Team page lists the chosen team's people. With no teams listed, the Team page does what it did before.
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

    /// Whether the Team page draws its team field: with two or more teams, since with one there is nothing to switch.
    var hasChoice: Bool { teams.count > 1 }

    /// Lists the signed-in account's teams and reads its stored choice. A server without the route lists none, and
    /// the Team page then draws no team field.
    func load(for session: SocialSession?) async {
        struct Reply: Decodable { let teams: [SocialTeam] }
        var reply: Reply?
        if let session {
            reply = try? await SocialAPI(endpoint: session.endpoint, key: session.deviceKey).social(["teams"])
            guard !Task.isCancelled else { return }
        }
        // Taken once the reply is in, so a load that answers after another one doesn't report its change again.
        let before = chosen?.id
        defer { if chosen?.id != before { onChoiceChanged?() } }
        guard let session else {
            teams = []
            chosenID = nil
            accountID = nil
            return
        }
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

/// The Team page's team field (navigation round 2, section 4.3): the menu field at 36, the search's size, with the
/// chosen team's mark at 20, its name in Body M at most 160 wide and the chevron, at its own width. Its menu is the
/// system menu of the account's teams, each with its mark and a toggle that ticks the chosen one. A team's mark is its
/// uppercase initial drawn as a person's avatar draws theirs. The field is drawn only with two or more teams, since
/// with one there is nothing to switch.
struct TeamMenuField: View {
    @ObservedObject var teams: TeamChoice

    var body: some View {
        if let chosen = teams.chosen, teams.hasChoice {
            // copy:
            SheetMenuField(title: chosen.name, help: "Switch team", accessibilityLabel: "Team", height: 36,
                           leadingInset: 8, titleMaxWidth: 160, hovers: true) {
                SocialPersonAvatar(name: chosen.name, size: 20, letters: 1)
            } items: {
                ForEach(teams.teams) { team in
                    // A toggle per team, so the menu ticks the chosen one.
                    Toggle(isOn: Binding(get: { team.id == chosen.id }, set: { _ in teams.choose(team) })) {
                        Label {
                            Text(team.name)
                        } icon: {
                            Image(nsImage: TeamIconImage.make(team))
                        }
                    }
                }
            }
            .fixedSize()
        }
    }
}

/// A team's mark as an image for the system menu, which draws images and not views: its uppercase initial, as the team
/// field draws it.
enum TeamIconImage {
    @MainActor
    static func make(_ team: SocialTeam, size: CGFloat = 16) -> NSImage {
        let renderer = ImageRenderer(content: SocialPersonAvatar(name: team.name, size: size, letters: 1))
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        return renderer.nsImage ?? NSImage()
    }
}
