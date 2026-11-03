import XCTest
@testable import CowchatMac

/// The starting sheets' rules that need no server (navigation note, section 7, and the add-new-company note): New
/// room's kinds and name check, New session's agent and Start, and the people list's order, height and more row.
final class StartingSheetsTests: XCTestCase {
    func testNewRoomListsTheKindsTheConnectionsAllow() {
        XCTAssertEqual(NewRoomKind.offered(signedIn: true, hasAPIKey: true), [.account, .thisMac, .apiKey])
        XCTAssertEqual(NewRoomKind.offered(signedIn: false, hasAPIKey: true), [.thisMac, .apiKey])
        XCTAssertEqual(NewRoomKind.offered(signedIn: true, hasAPIKey: false), [.account, .thisMac])
        XCTAssertEqual(NewRoomKind.offered(signedIn: false, hasAPIKey: false), [.thisMac])
        XCTAssertEqual(NewRoomKind.starting(signedIn: true), .account)
        XCTAssertEqual(NewRoomKind.starting(signedIn: false), .thisMac)
        for kind in NewRoomKind.allCases { XCTAssertEqual(NewRoomKind(server: kind.server), kind) }
    }

    func testNewRoomChecksTheName() {
        XCTAssertNil(NewRoomKind.nameProblem("booking-copy"))
        XCTAssertNil(NewRoomKind.nameProblem(""), "An empty name only keeps Create disabled")
        XCTAssertEqual(NewRoomKind.nameProblem(String(repeating: "a", count: 101)),
                       "Room names can contain at most 100 characters.")
        XCTAssertEqual(NewRoomKind.nameProblem("a\u{7}b"), "Room names cannot contain control characters.")
    }

    func testNewSessionStartsWithTheLastAgentWhileItIsConnected() {
        let all: (LiveSession.Harness) -> Bool = { _ in true }
        let onlyCodex: (LiveSession.Harness) -> Bool = { $0 == .codex }
        XCTAssertEqual(NewSessionSheet.startingHarness(last: .codex, isConnected: all), .codex)
        XCTAssertEqual(NewSessionSheet.startingHarness(last: nil, isConnected: all), .claudeCode)
        XCTAssertEqual(NewSessionSheet.startingHarness(last: .claudeCode, isConnected: onlyCodex), .codex,
                       "The last agent is skipped once it is not connected")
        XCTAssertNil(NewSessionSheet.startingHarness(last: .codex, isConnected: { _ in false }))
    }

    func testNewSessionStartsWithAFolderAConnectedAgentAndABranchForANewWorktree() {
        let folder = SessionFolder(name: "cowchat")
        func canStart(folder: SessionFolder? = folder, connected: Bool = true, newWorktree: Bool = false,
                      branch: String = "", busy: Bool = false) -> Bool {
            NewSessionSheet.canStart(folder: folder, harnessIsConnected: connected, newWorktree: newWorktree,
                                     branch: branch, busy: busy)
        }
        XCTAssertTrue(canStart())
        XCTAssertFalse(canStart(folder: nil))
        XCTAssertFalse(canStart(connected: false))
        XCTAssertFalse(canStart(newWorktree: true), "A new worktree needs its branch")
        XCTAssertTrue(canStart(newWorktree: true, branch: "pmandia/team-columns"))
        XCTAssertFalse(canStart(busy: true))
    }

    private let people = ["chad", "adrian", "logan", "martin", "nico", "theo"].map {
        SocialAccount(id: $0, handle: $0.capitalized, displayName: $0.capitalized)
    }

    func testThePeopleListPutsThePickedFirstAndKeepsThemThroughASearch() {
        let ids = { (rows: [SocialAccount]) in rows.map(\.id) }
        XCTAssertEqual(ids(PeopleList.rows(people, picked: [], query: "")), people.map(\.id))
        XCTAssertEqual(ids(PeopleList.rows(people, picked: ["nico", "chad"], query: "")),
                       ["nico", "chad", "adrian", "logan", "martin", "theo"], "Picked first, in the order picked")
        XCTAssertEqual(ids(PeopleList.rows(people, picked: ["nico"], query: "lo")), ["nico", "logan"],
                       "A picked row stays though it does not match")
        XCTAssertEqual(ids(PeopleList.rows(people, picked: [], query: "@Mar")), ["martin"], "A handle, with its @")
    }

    func testThePeopleListIsAsTallAsFiveRowsAndTheMoreRow() {
        XCTAssertEqual(PeopleList.height(count: 0), 44, "The empty line")
        XCTAssertEqual(PeopleList.height(count: 2), 2 * 44 + 8)
        XCTAssertEqual(PeopleList.height(count: 5), 5 * 44 + 4 * 8)
        XCTAssertEqual(PeopleList.height(count: 6), 284, "Five rows and the more row")
        XCTAssertEqual(PeopleList.moreTitle(hidden: 1, showsAll: false, noun: ("friend", "friends")), "Show 1 more")
        XCTAssertEqual(PeopleList.moreTitle(hidden: 2, showsAll: true, noun: ("friend", "friends")), "Hide 2 friends")
        XCTAssertEqual(PeopleList.moreTitle(hidden: 1, showsAll: true, noun: ("person", "people")), "Hide 1 person")
    }

    /// New room's suggestions match by name and handle, leave out the people picked, keep the server's order and stop
    /// at three (navigation round 2, section 6.2).
    func testNewRoomSuggestsUpToThreeUnpickedMatchesInTheServersOrder() {
        let ids = { (rows: [SocialAccount]) in rows.map(\.id) }
        XCTAssertEqual(ids(PeopleList.suggestions(people, picked: [], query: "")), [], "Nothing typed suggests no one")
        XCTAssertEqual(ids(PeopleList.suggestions(people, picked: [], query: "n")), ["adrian", "logan", "martin"],
                       "Three, in the server's order, with Nico left until more is typed")
        XCTAssertEqual(ids(PeopleList.suggestions(people, picked: ["adrian", "logan"], query: "n")), ["martin", "nico"])
        XCTAssertEqual(ids(PeopleList.suggestions(people, picked: [], query: "@The")), ["theo"], "A handle, with its @")
        XCTAssertEqual(ids(PeopleList.suggestions(people, picked: [], query: "zz")), [])
    }

    /// Picking puts the friend first, once, and Remove takes them out. Return picks the first suggestion, and with
    /// nothing typed it submits the sheet.
    func testNewRoomPicksNewestFirstAndReturnPicksTheFirstSuggestion() {
        XCTAssertEqual(PeopleList.picking("chad", into: []), ["chad"])
        XCTAssertEqual(PeopleList.picking("theo", into: ["chad"]), ["theo", "chad"], "The newest pick first")
        XCTAssertEqual(PeopleList.picking("chad", into: ["theo", "chad"]), ["chad", "theo"], "A person is picked once")
        XCTAssertEqual(PeopleList.removing("theo", from: ["theo", "chad"]), ["chad"])

        XCTAssertEqual(PeopleList.submit(people, picked: ["adrian"], query: "n"), .pick("logan"))
        XCTAssertEqual(PeopleList.submit(people, picked: [], query: "  "), .submitEmpty)
        XCTAssertEqual(PeopleList.submit(people, picked: [], query: "zz"), .nothing)
    }

    func testNewRoomsPeopleAreaIsThreeRowsTall() {
        XCTAssertEqual(PickedPeople.areaHeight, 148)
    }
}
