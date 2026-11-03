import SwiftUI

/// The Team page: the header with the count under the title and the team field and the search at its end, then 32 under
/// it the column labels and one group per person who shares their work, 32 apart, each with a head, up to five rows and
/// a row for the rest, in one column capped at 760.
struct TeamPage: View {
    @EnvironmentObject private var workspace: WorkspaceStore
    @ObservedObject var team: TeamStore
    /// The account's rooms, which say which rows open a room.
    @ObservedObject var account: ChatStore
    @FocusState private var isSearchFocused: Bool
    /// The rows' width with their insets, which the header takes.
    @State private var rowsWidth: CGFloat?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            PageHeader(title: "Team", caption: caption, rowsWidth: rowsWidth) {
                HStack(spacing: 8) {
                    // Switching keeps the typed search, so the caption's count is the new team's.
                    TeamMenuField(teams: workspace.teams)
                    // copy: the spec's "Search people, branches or repos" does not fit the 240 capsule in Body M.
                    SearchCapsule(placeholder: "Search people or branches", text: $team.query,
                                  isFocused: $isSearchFocused, clearLabel: "Clear team search", width: 240)
                }
            }
            if team.isLoaded {
                if team.groups.isEmpty {
                    // copy:
                    emptyLine("No one on your team is sharing live sessions.")
                } else if team.shownGroups.isEmpty {
                    // copy:
                    emptyLine("Nothing matches.")
                } else {
                    list
                }
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onPreferenceChange(PageRowsWidthKey.self) { rowsWidth = $0 }
    }

    /// The count once the server has answered and someone shares their work; an empty page says so in its line.
    private var caption: String? {
        team.isLoaded && !team.groups.isEmpty ? team.countLine : nil
    }

    private func emptyLine(_ text: String) -> some View {
        Text(text)
            .gallopText(.bodyM, color: SemanticColor.textTertiary)
            .padding(.leading, PageLayout.emptyLeading)
            .padding(.top, PageLayout.emptyTop)
    }

    private var list: some View {
        let roomIDs = Set(account.rooms.map(\.id))
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: 32) {
                ForEach(team.shownGroups) { group in
                    VStack(alignment: .leading, spacing: 0) {
                        // The labels are the list's column heads, on their own line 4 over the first shown head.
                        if group.id == team.shownGroups.first?.id {
                            TeamColumnLabels()
                                .padding(.bottom, 4)
                        }
                        TeamGroupHead(person: group.person)
                        ForEach(team.rows(of: group)) { branch in
                            TeamRow(branch: branch,
                                    opens: TeamList.opensRoom(branch, accountRoomIDs: roomIDs)) {
                                Task { await workspace.open(branch) }
                            }
                        }
                        if team.hiddenCount(of: group) > 0 {
                            SessionsMoreRow(title: team.moreTitle(of: group), indent: TeamRow.leading,
                                            surface: .page) {
                                team.toggleMore(of: group)
                            }
                        }
                    }
                }
            }
            .frame(maxWidth: PageLayout.listWidth, alignment: .leading)
            // The labels' line has no inset above its text.
            .padding(.top, PageLayout.listTop(firstTextInset: 0))
            .padding(.bottom, 16)
            .padding(.horizontal, PageLayout.listInset)
            .reportsPageRowsWidth()
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.automatic)
    }
}

/// A person's head on the Team page, the message row's sender line: 16 in, their 24 avatar, 11, and their name in
/// Body M Strong, 32 tall. It is not a button. Friends heads its groups with the same line without the avatar.
struct TeamGroupHead: View {
    let title: String
    let person: SocialAccount?

    static let height: CGFloat = 32
    /// How far under the head's top its title's line of 20 starts, centred in the head.
    static var textInset: CGFloat { (height - 20) / 2 }

    init(person: SocialAccount) {
        title = person.displayName
        self.person = person
    }

    init(title: String) {
        self.title = title
        person = nil
    }

    var body: some View {
        HStack(spacing: 11) {
            if let person { SocialPersonAvatar(name: person.displayName, size: 24, letters: 1) }
            Text(title)
                .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
        }
        .padding(.leading, 16)
        .frame(height: Self.height)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// The Team page's column labels (navigation round 2, section 4.3), once, on their own 20 line over the first shown
/// person's head: "Branch" over the branch names, then "Repo", "Pull request" and "State" in Body S, Text/Tertiary,
/// laid out with the rows' own widths. Repo ends where its column ends. Pull request and State end where their words
/// end, 22 before their columns end, since a 16 mark slot and its 6 follow the words. A screen reader hears the
/// columns in each row's label instead.
struct TeamColumnLabels: View {
    /// The 6 and the 16 mark slot after a pull request's number and a state's word.
    private static let markSlot: CGFloat = 6 + 16

    var body: some View {
        HStack(spacing: 0) {
            // copy:
            Text("Branch")
                .gallopText(.bodyS, color: SemanticColor.textTertiary)
                .lineLimit(1)
                .frame(minWidth: TeamRow.nameMinWidth, maxWidth: .infinity, alignment: .leading)
            Spacer().frame(width: TeamRow.gap)
            // copy:
            label("Repo", width: TeamRow.repoWidth)
            Spacer().frame(width: TeamRow.gap)
            // copy:
            label("Pull request", width: TeamRow.pullRequestWidth - Self.markSlot)
            Spacer().frame(width: Self.markSlot + TeamRow.gap)
            // copy:
            label("State", width: TeamRow.stateWidth - Self.markSlot)
            Spacer().frame(width: Self.markSlot + TeamRow.endWidth)
        }
        .padding(.leading, TeamRow.leading)
        .frame(height: 20)
        .frame(maxWidth: .infinity)
        .accessibilityHidden(true)
    }

    private func label(_ text: String, width: CGFloat) -> some View {
        Text(text)
            .gallopText(.bodyS, color: SemanticColor.textTertiary)
            .lineLimit(1)
            .fixedSize()
            .frame(width: width, alignment: .trailing)
    }
}

/// One branch on the Team page (navigation note, section 9.1): 40 tall and one line, the name starting under the
/// person's name at 51, then Repo (120), Pull request (88, the number and a 16 slot for its checks), State (80, the
/// word and a 16 slot for its mark), all ending on the right, and the chevron on a row that opens a room. Only those
/// rows are buttons. The name links to GitHub when the branch has somewhere to go there, and a press on the rest of
/// the row still opens the room.
struct TeamRow: View {
    let branch: TeamBranch
    let opens: Bool
    let press: () -> Void

    /// The head's 16, its 24 avatar and the 11 after it.
    static let leading: CGFloat = 16 + 24 + 11
    static let height: CGFloat = 40
    static let nameMinWidth: CGFloat = 160
    static let gap: CGFloat = 16
    static let repoWidth: CGFloat = 120
    static let pullRequestWidth: CGFloat = 88
    static let stateWidth: CGFloat = 80
    /// The 12 before the chevron, its 16 and the 12 after it.
    static let endWidth: CGFloat = 12 + 16 + 12

    var body: some View {
        if opens, let room = branch.room {
            Button(action: press) { label }
                .buttonStyle(PageRowButtonStyle())
                // copy:
                .help("Open \(room.name)")
                .overlay(alignment: .leading) { nameLink }
                .macAccessibleAction(label: TeamList.accessibilityLabel(for: branch), namedActions: gitHubAction,
                                     action: press)
        } else {
            label
                .overlay(alignment: .leading) { nameLink }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(TeamList.accessibilityLabel(for: branch))
                .accessibilityActions {
                    if let url = branch.gitHubURL {
                        // copy:
                        Button("Open on GitHub") { MessageLinks.open(url) }
                    }
                }
        }
    }

    private var gitHubAction: [MacNamedAction] {
        // copy:
        branch.gitHubURL.map { url in [MacNamedAction(name: "Open on GitHub") { MessageLinks.open(url) }] } ?? []
    }

    private var label: some View {
        columns {
            Text(branch.name)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
                .opacity(branch.gitHubURL == nil ? 1 : 0)
        } rest: {
            Text(branch.repo)
                .gallopText(.bodyS, color: SemanticColor.textTertiary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(width: Self.repoWidth, alignment: .trailing)
            Spacer().frame(width: Self.gap)
            pullRequest
                .frame(width: Self.pullRequestWidth, alignment: .trailing)
            Spacer().frame(width: Self.gap)
            state
                .frame(width: Self.stateWidth, alignment: .trailing)
            Spacer().frame(width: 12)
            ZStack {
                if opens {
                    GallopIconView(icon: .chevronRightSmall, fallbackSystemName: "chevron.right", size: 16)
                        .foregroundStyle(SemanticColor.iconTertiary)
                }
            }
            .frame(width: 16, height: 16)
            Spacer().frame(width: 12)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    /// The name as a link over the row, outside its button, pressed only on the text itself. The space for the other
    /// columns is empty and lets presses through to the row.
    @ViewBuilder private var nameLink: some View {
        if let url = branch.gitHubURL {
            columns {
                TextLink(title: branch.name, style: .bodyM, color: SemanticColor.textPrimary) { MessageLinks.open(url) }
                    .help(TeamList.gitHubHelp(for: branch))
            } rest: {
                Spacer().frame(width: Self.repoWidth + Self.gap + Self.pullRequestWidth + Self.gap + Self.stateWidth
                               + Self.endWidth)
            }
        }
    }

    /// The row's line: the name in its flexible column at 51, then the 16 gap and the fixed columns after it.
    private func columns<Name: View, Rest: View>(@ViewBuilder name: () -> Name,
                                                 @ViewBuilder rest: () -> Rest) -> some View {
        HStack(spacing: 0) {
            name()
                .frame(minWidth: Self.nameMinWidth, maxWidth: .infinity, alignment: .leading)
            Spacer().frame(width: Self.gap)
            rest()
        }
        .padding(.leading, Self.leading)
        .frame(height: Self.height)
    }

    /// The number, 6, then the checks mark's 16 slot, kept empty with no checks so the numbers end in one column.
    @ViewBuilder private var pullRequest: some View {
        if let pullRequest = branch.pullRequest {
            HStack(spacing: 6) {
                Text(verbatim: "#\(pullRequest.number)")
                    .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    .lineLimit(1)
                    .fixedSize()
                PullRequestChecksMark(checks: pullRequest.checks, showsRunning: false)
                    .frame(width: 16, height: 16)
            }
            .help(TeamList.help(for: pullRequest))
        } else {
            Color.clear.frame(height: 16)
        }
    }

    /// The state's word, 6, then its mark's 16 slot (navigation note, section 9.2).
    private var state: some View {
        HStack(spacing: 6) {
            Text(TeamList.stateWord(branch.state))
                .gallopText(.bodyS, color: branch.state == .idle ? SemanticColor.textTertiary
                            : SemanticColor.textSecondary)
                .lineLimit(1)
                .fixedSize()
            ZStack {
                switch branch.state {
                case .working:
                    SessionWorkingMark()
                case .prompt:
                    GallopIconView(icon: .warning, fallbackSystemName: "exclamationmark.triangle.fill", size: 16)
                        .foregroundStyle(SemanticColor.notificationErrorTextDefault)
                case .idle:
                    Color.clear
                }
            }
            .frame(width: 16, height: 16)
        }
    }
}

/// Settings' Team page: the switch that takes the person's live sessions off their teammates' Team pages, drawn as the
/// notification settings' switches are, with no line under it since the page's subtitle says what it does. It waits
/// for the server to say where the person stands.
struct TeamSharingSettings: View {
    @ObservedObject var team: TeamStore
    @ObservedObject var social: SocialStore

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            // copy:
            Toggle("Show my live sessions on Team", isOn: Binding(
                get: { team.sharing ?? false },
                set: { on in Task { await team.setSharing(on, social: social) } }
            ))
            .gallopText(.bodyMStrong)
            .disabled(team.sharing == nil || social.busy)
            .macAccessibleAction(label: "Show my live sessions on Team", value: team.sharing == true ? "on" : "off",
                                 isEnabled: team.sharing != nil && !social.busy) {
                Task { await team.setSharing(!(team.sharing ?? false), social: social) }
            }
            if let error = social.errorMessage {
                Text(error).gallopText(.bodyS, color: SemanticColor.notificationErrorTextDefault)
            }
        }
        .task { await team.refresh(social: social) }
    }
}
