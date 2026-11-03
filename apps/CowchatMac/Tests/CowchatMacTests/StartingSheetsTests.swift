import XCTest
@testable import CowchatMac

/// The starting sheets' rules that need no server (navigation note, section 7, and the add-new-company note): New
/// room's kinds and name check, and the people list's order, height and more row.
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
}
