import Foundation

/// `GET /social/v1/team` (Chad's plan, task 12): each teammate who shares their work, with their live sessions, and
/// whether the person's own work is shared. A session carries its folder, branch and state, the session room it works
/// in when the person is in that room, and the newest pull request it posted where the person can read it.
struct TeamReply: Decodable {
    let sharing: Bool
    let people: [TeamPerson]
}

struct TeamPerson: Decodable, Equatable {
    let account: SocialAccount
    let sessions: [TeamSession]
}

struct TeamSession: Decodable, Equatable {
    struct RoomRef: Decodable, Equatable {
        let roomId: String
        let name: String
    }

    /// The session's agent ID.
    let id: String
    let folder: String
    let branch: String
    /// Its presence status, as `LiveSession.State(presence:)` reads it.
    let state: String?
    let startedAt: String
    let room: RoomRef?
    let pullRequest: MessagePullRequest?
    /// When the message carrying the pull request was sent.
    let pullRequestAt: String?
    /// The repository its folder's GitHub remote names, "owner/name" as `MessagePullRequest.repository` names one,
    /// when the hook sent it (navigation note, section 9.3, a question for Chad).
    var repository: String?

    /// The session as `LiveSession` holds one, for its name and state.
    var live: LiveSession {
        LiveSession(id: id, harness: .claudeCode, folder: folder, branch: branch, state: LiveSession.State(presence: state),
                    startedAt: startedAt.cowchatDate ?? .distantPast)
    }
}

/// One row of the Team page: a branch someone works on, with every session on it merged.
struct TeamBranch: Identifiable, Equatable {
    /// The person's account ID, the folder and the branch.
    let id: String
    /// The branch as a session is named: without its person's prefix, or the folder on main. Never numbered.
    let name: String
    let folder: String
    let branch: String
    /// Working when any session on the branch works, else at a prompt when any is at one, else idle.
    let state: LiveSession.State
    /// The latest start of any session on the branch, which orders the person's rows.
    let latestStart: Date
    /// The room the branch's newest session in one works in, when the person is in it.
    let room: TeamSession.RoomRef?
    /// The newest pull request a session on the branch posted where the person can read it.
    let pullRequest: MessagePullRequest?
    /// The newest session's repository, "owner/name", when the server sent one.
    var repository: String?

    /// The repo column: the folder, or nothing when the name already is the folder.
    var repo: String { name == folder ? "" : folder }

    /// Where the name links on GitHub (navigation note, section 9.3): the pull request's page, else the branch's page
    /// in the repository, or the repository's page with no branch, else nowhere.
    var gitHubURL: URL? {
        if let url = pullRequest?.url { return url }
        guard let repository, !repository.isEmpty else { return nil }
        var address = "https://github.com/\(repository)"
        if !branch.isEmpty {
            let segments = branch.split(separator: "/", omittingEmptySubsequences: false).map {
                $0.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed.subtracting(["/"])) ?? String($0)
            }
            address += "/tree/" + segments.joined(separator: "/")
        }
        return URL(string: address)
    }
}

extension LiveSession.State {
    /// A branch's state from its sessions: working when any works, else at a prompt when any is at one, else idle.
    init(of sessions: [LiveSession]) {
        if sessions.contains(where: { $0.state == .working }) {
            self = .working
        } else if sessions.contains(where: { $0.state == .prompt }) {
            self = .prompt
        } else {
            self = .idle
        }
    }
}

/// One person on the Team page, with their branches newest first.
struct TeamGroup: Identifiable, Equatable {
    let person: SocialAccount
    let isViewer: Bool
    let branches: [TeamBranch]

    var id: String { person.id }
}

/// What the Team page makes of the server's reply: one row per branch, people in order, five rows each and a row for
/// the rest, the search and its count.
enum TeamList {
    /// How many rows a person shows before their more row.
    static let limit = 5

    /// One group per person with live work: teammates in order of their display names, then the person's own last.
    static func groups(from people: [TeamPerson], viewer: String) -> [TeamGroup] {
        people.map { person in
            TeamGroup(person: person.account, isViewer: person.account.id == viewer,
                      branches: branches(of: person.sessions, person: person.account.id))
        }
        .filter { !$0.branches.isEmpty }
        .sorted { lhs, rhs in
            if lhs.isViewer != rhs.isViewer { return rhs.isViewer }
            return lhs.person.displayName.localizedStandardCompare(rhs.person.displayName) == .orderedAscending
        }
    }

    /// A person's sessions as one row per branch, newest first by the latest start of any session on it, so the order
    /// does not change with what the sessions are doing.
    static func branches(of sessions: [TeamSession], person: String) -> [TeamBranch] {
        let byBranch = Dictionary(grouping: sessions) { "\($0.folder)\u{0}\($0.branch)" }
        return byBranch.values.compactMap { sessions -> TeamBranch? in
            guard let first = sessions.first else { return nil }
            let newestFirst = sessions.sorted {
                ($0.startedAt.cowchatDate ?? .distantPast, $0.id) > ($1.startedAt.cowchatDate ?? .distantPast, $1.id)
            }
            let posted = sessions.compactMap { session -> (MessagePullRequest, Date)? in
                guard let pullRequest = session.pullRequest else { return nil }
                return (pullRequest, session.pullRequestAt?.cowchatDate ?? .distantPast)
            }
            return TeamBranch(
                id: "\(person)/\(first.folder)/\(first.branch)", name: first.live.baseName, folder: first.folder,
                branch: first.branch, state: LiveSession.State(of: sessions.map(\.live)),
                latestStart: newestFirst.first?.startedAt.cowchatDate ?? .distantPast,
                room: newestFirst.lazy.compactMap(\.room).first,
                pullRequest: posted.max { $0.1 < $1.1 }?.0, repository: newestFirst.first?.repository)
        }
        .sorted { ($0.latestStart, $0.id) > ($1.latestStart, $1.id) }
    }

    /// Whether a row matches the search: its person's name, its branch, its repo, or its pull request's number or
    /// title. Case is ignored, and a number may be written with or without its #.
    static func matches(_ branch: TeamBranch, of person: SocialAccount, query: String) -> Bool {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else { return true }
        var fields = [person.displayName, person.handle, branch.name, branch.branch, branch.folder]
        if let pullRequest = branch.pullRequest {
            fields += ["#\(pullRequest.number)", "\(pullRequest.number)", pullRequest.title]
        }
        return fields.contains { $0.lowercased().contains(query) }
    }

    /// The groups the search leaves: each person with a matching row, holding only those rows.
    static func filtered(_ groups: [TeamGroup], query: String) -> [TeamGroup] {
        guard isSearching(query) else { return groups }
        return groups.compactMap { group in
            let rows = group.branches.filter { matches($0, of: group.person, query: query) }
            return rows.isEmpty ? nil : TeamGroup(person: group.person, isViewer: group.isViewer, branches: rows)
        }
    }

    static func isSearching(_ query: String) -> Bool {
        !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    // copy:
    /// The count under the title, as `listCount` writes one: "50 branches", or "4 of 50 branches" while the search
    /// narrows the list.
    static func count(shown: Int, total: Int, narrowed: Bool) -> String {
        let noun = total == 1 ? "branch" : "branches"
        return narrowed ? "\(shown) of \(total) \(noun)" : "\(total) \(noun)"
    }

    /// A row opens a room when the person is in the room it works in, on their account.
    static func opensRoom(_ branch: TeamBranch, accountRoomIDs: Set<String>) -> Bool {
        branch.room.map { accountRoomIDs.contains($0.roomId) } ?? false
    }

    // copy:
    /// The State column's word. "Waiting" names no harness, since a branch can hold sessions of both.
    static func stateWord(_ state: LiveSession.State) -> String {
        switch state {
        case .working: return "Working"
        case .prompt: return "Waiting"
        case .idle: return "Idle"
        }
    }

    // copy:
    /// A row's accessible label, naming its columns: "fee-ledger, repo dashboard, pull request 1806, checks passed,
    /// working".
    static func accessibilityLabel(for branch: TeamBranch) -> String {
        [branch.name, branch.repo.isEmpty ? nil : "repo \(branch.repo)",
         branch.pullRequest.map { "pull request \($0.number)" }, branch.pullRequest?.checksLabel?.lowercased(),
         stateWord(branch.state).lowercased()]
            .compactMap { $0 }
            .joined(separator: ", ")
    }

    // copy:
    /// The name link's help: "Open #1806 on GitHub", or "Open fee-ledger on GitHub" with no pull request.
    static func gitHubHelp(for branch: TeamBranch) -> String {
        "Open \(branch.pullRequest.map { "#\($0.number)" } ?? branch.name) on GitHub"
    }

    // copy:
    /// The pull request column's help: "dashboard #1806, Fee ledger from saved hire prices, checks passed".
    static func help(for pullRequest: MessagePullRequest) -> String {
        ([pullRequest.name, pullRequest.title] + [pullRequest.checksLabel.map { $0.lowercased() }].compactMap { $0 })
            .joined(separator: ", ")
    }
}

/// The Team page's state: the teammates' work from the server, polled with the friends, the search, each person's open
/// more row, and whether the person shares their own work.
@MainActor
final class TeamStore: ObservableObject {
    @Published private(set) var groups: [TeamGroup] = []
    /// False until the first poll answers.
    @Published private(set) var isLoaded = false
    /// Whether the person's own work is on Team, once the server has said.
    @Published private(set) var sharing: Bool?
    @Published var query = ""
    /// The people whose more row is open. It is not kept across launches.
    @Published var expanded: Set<String> = []

    private var accountID: String?
    /// The team chosen in the sidebar's team row, or nil for the server's choice when the account has no teams.
    private(set) var teamID: String?
    private var generation = 0
    private var people: [TeamPerson] = []

    /// Starts over for the account signed in.
    func use(accountID: String?) {
        guard accountID != self.accountID else { return }
        self.accountID = accountID
        teamID = nil
        startOver()
    }

    /// Starts over for the team chosen in the sidebar's team row.
    func use(teamID: String?) {
        guard teamID != self.teamID else { return }
        self.teamID = teamID
        startOver()
    }

    private func startOver() {
        generation += 1
        groups = []
        people = []
        isLoaded = false
        sharing = nil
        query = ""
        expanded = []
    }

    /// Asks the server for the chosen team's work. A server without the route leaves the page as it was.
    func refresh(social: SocialStore) async {
        guard let accountID, social.savedSession?.account.id == accountID else { return }
        let ticket = generation
        let query = teamID.map { [URLQueryItem(name: "team_id", value: $0)] } ?? []
        guard let reply: TeamReply = try? await social.readingAPI().social(["team"], query: query),
              ticket == generation else {
            return
        }
        receive(reply)
    }

    func receive(_ reply: TeamReply) {
        people = reply.people
        let fresh = TeamList.groups(from: reply.people, viewer: accountID ?? "")
        if fresh != groups { groups = fresh }
        if sharing != reply.sharing { sharing = reply.sharing }
        isLoaded = true
    }

    /// Turns the person's own work on or off on Team, then asks again.
    func setSharing(_ on: Bool, social: SocialStore) async {
        await social.perform { api in
            let _: SocialAcknowledgement = try await api.social(["team", "sharing"], method: "PUT", body: ["sharing": on])
        }
        await refresh(social: social)
    }

    // MARK: The page

    var total: Int { groups.reduce(0) { $0 + $1.branches.count } }

    var isSearching: Bool { TeamList.isSearching(query) }

    /// The people the page shows: everyone, or while searching, those with a matching row.
    var shownGroups: [TeamGroup] { TeamList.filtered(groups, query: query) }

    var countLine: String {
        TeamList.count(shown: shownGroups.reduce(0) { $0 + $1.branches.count }, total: total, narrowed: isSearching)
    }

    /// A person's rows: every match while searching, or else the first five until their more row is open.
    func rows(of group: TeamGroup) -> [TeamBranch] {
        isSearching || expanded.contains(group.id) ? group.branches : Array(group.branches.prefix(TeamList.limit))
    }

    /// How many rows the five leave out; the more row shows only when it is more than none and nobody is searching.
    func hiddenCount(of group: TeamGroup) -> Int {
        isSearching ? 0 : max(0, group.branches.count - TeamList.limit)
    }

    // copy:
    func moreTitle(of group: TeamGroup) -> String {
        expanded.contains(group.id) ? "Hide \(hiddenCount(of: group))" : "Show \(hiddenCount(of: group)) more"
    }

    func toggleMore(of group: TeamGroup) {
        if expanded.contains(group.id) { expanded.remove(group.id) } else { expanded.insert(group.id) }
    }

    /// Whose a session is, from its folder and branch, among the people sharing their work.
    func owner(of session: LiveSession) -> SocialAccount? {
        people.first { person in
            person.sessions.contains { $0.folder == session.folder && $0.branch == session.branch }
        }?.account
    }
}
