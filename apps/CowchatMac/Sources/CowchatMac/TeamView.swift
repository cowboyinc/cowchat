import SwiftUI

/// The Team page: the header with the count under the title and the search at its end, then one group per person who
/// shares their work, each with a head, up to five rows and a row for the rest, in one column capped at 760.
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
                // copy: the spec's "Search people, branches or repos" does not fit the 240 capsule in Body M.
                SearchCapsule(placeholder: "Search people or branches", text: $team.query,
                              isFocused: $isSearchFocused, clearLabel: "Clear team search", width: 240)
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
            LazyVStack(alignment: .leading, spacing: 24) {
                ForEach(team.shownGroups) { group in
                    VStack(alignment: .leading, spacing: 0) {
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
            .padding(.top, 4)
            .padding(.bottom, 16)
            .padding(.horizontal, PageLayout.listInset)
            .reportsPageRowsWidth()
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.automatic)
    }
}

/// A person's head on the Team page, the message row's sender line: 16 in, their 24 avatar, 11, and their name in
/// Body M Strong, 32 tall. It is not a button.
struct TeamGroupHead: View {
    let person: SocialAccount

    var body: some View {
        HStack(spacing: 11) {
            SocialPersonAvatar(name: person.displayName, size: 24, letters: 1)
            Text(person.displayName)
                .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
        }
        .padding(.leading, 16)
        .frame(height: 32)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// One branch on the Team page, drawn from Home's session row: 40 tall and one line, the name starting under the
/// person's name at 51, then the Repo (120, right-aligned), the Pull request (72), the turning mark while any session
/// on it works, and the chevron on a row that opens a room. Only those rows are buttons.
struct TeamRow: View {
    let branch: TeamBranch
    let opens: Bool
    let press: () -> Void

    /// The head's 16, its 24 avatar and the 11 after it.
    static let leading: CGFloat = 16 + 24 + 11
    static let height: CGFloat = 40
    static let nameMinWidth: CGFloat = 160
    static let repoWidth: CGFloat = 120
    static let pullRequestWidth: CGFloat = 72

    var body: some View {
        if opens, let room = branch.room {
            Button(action: press) { label }
                .buttonStyle(PageRowButtonStyle())
                // copy:
                .help("Open \(room.name)")
                .macAccessibleAction(label: accessibilityLabel, action: press)
        } else {
            label
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(accessibilityLabel)
        }
    }

    private var label: some View {
        HStack(spacing: 0) {
            Text(branch.name)
                .gallopText(.bodyM, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(minWidth: Self.nameMinWidth, maxWidth: .infinity, alignment: .leading)
            Spacer().frame(width: 16)
            Text(branch.repo)
                .gallopText(.bodyS, color: SemanticColor.textTertiary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(width: Self.repoWidth, alignment: .trailing)
            Spacer().frame(width: 16)
            pullRequest
                .frame(width: Self.pullRequestWidth, alignment: .leading)
            Spacer().frame(width: 12)
            ZStack {
                if branch.isWorking { SessionWorkingMark() }
            }
            .frame(width: 28, height: 16)
            Spacer().frame(width: 4)
            ZStack {
                if opens {
                    GallopIconView(icon: .chevronRightSmall, fallbackSystemName: "chevron.right", size: 16)
                        .foregroundStyle(SemanticColor.iconTertiary)
                }
            }
            .frame(width: 16, height: 16)
            Spacer().frame(width: 12)
        }
        .padding(.leading, Self.leading)
        .frame(height: Self.height)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    @ViewBuilder private var pullRequest: some View {
        if let pullRequest = branch.pullRequest {
            HStack(spacing: 6) {
                Text(verbatim: "#\(pullRequest.number)")
                    .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    .lineLimit(1)
                    .fixedSize()
                PullRequestChecksMark(checks: pullRequest.checks, showsRunning: false)
            }
            .help(TeamList.help(for: pullRequest))
        } else {
            Color.clear.frame(height: 16)
        }
    }

    // copy:
    /// "fee-ledger, dashboard, #1806, working"
    private var accessibilityLabel: String {
        [branch.name, branch.repo, branch.pullRequest.map { "#\($0.number)" } ?? "", branch.isWorking ? "working" : ""]
            .filter { !$0.isEmpty }
            .joined(separator: ", ")
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
