import XCTest
@testable import CowchatMac

/// The sidebar's list model (navigation note, sections 3.3 to 3.11): the one order across the three connections, the
/// filter, company rooms, the search's groups, the place icons and the short time.
final class SidebarListTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_000_000)

    private func stamp(_ minutesAgo: Double) -> String {
        ISO8601DateFormatter().string(from: now.addingTimeInterval(-minutesAgo * 60))
    }

    private func room(_ id: String, on server: WorkspaceStore.Server = .accountGlobal, minutesAgo: Double,
                      parent: String? = nil, description: String? = nil, session: LiveSession? = nil,
                      archived: Bool = false, unread: Bool = false, messages: Bool = false) -> SidebarRoom {
        SidebarRoom(server: server,
                    room: Room(roomID: id, name: id, description: description, parentID: parent,
                               createdAt: stamp(10_000), createdBy: nil, visibility: "public",
                               lastActivity: stamp(minutesAgo), memberCount: nil, encrypted: false),
                    session: session, isArchived: archived, isUnread: unread, matchesMessages: messages)
    }

    private func session(_ id: String, folder: String = "cowchat", branch: String = "main",
                         state: LiveSession.State = .idle, startedMinutesAgo: Double = 600,
                         waitingMinutesAgo: Double? = nil) -> LiveSession {
        LiveSession(id: id, harness: .codex, folder: folder, branch: branch, state: state,
                    startedAt: now.addingTimeInterval(-startedMinutesAgo * 60), roomID: "session-\(id)",
                    waitingSince: waitingMinutesAgo.map { now.addingTimeInterval(-$0 * 60) })
    }

    private func names(_ rows: [SidebarRow]) -> [String] {
        rows.map { row in
            switch row {
            case let .cell(room, depth, disclosure, showsDot):
                let mark = disclosure.map { $0 == .open ? "v " : "> " } ?? ""
                return String(repeating: "  ", count: depth) + mark + room.room.name + (showsDot ? " *" : "")
            case let .more(_, hidden, showsAll): return showsAll ? "less" : "\(hidden) more"
            case let .label(title): return "[\(title)]"
            case let .joinable(item): return "join " + item.name
            }
        }
    }

    // MARK: Order

    func testTheThreeConnectionsShareOneOrderByActivity() {
        let rooms = [
            room("lobby", on: .local, minutesAgo: 300), room("junction", on: .global, minutesAgo: 5),
            room("fee-ledger", minutesAgo: 9), room("scratch", on: .local, minutesAgo: 1),
        ]
        let rows = RoomSidebarPresentation.rows(for: .all, rooms: rooms, collapsed: [], showsAllChildren: [])
        XCTAssertEqual(names(rows), ["scratch", "junction", "fee-ledger", "lobby"])
    }

    func testALiveSessionsRoomTakesItsStartAndItsWaitInItsTime() {
        let started = room("started", minutesAgo: 120, session: session("a", startedMinutesAgo: 3))
        let waiting = room("waiting", minutesAgo: 120,
                           session: session("b", state: .prompt, startedMinutesAgo: 600, waitingMinutesAgo: 1))
        let busy = room("busy", minutesAgo: 2)
        let rows = RoomSidebarPresentation.rows(for: .all, rooms: [started, busy, waiting], collapsed: [],
                                                showsAllChildren: [])
        XCTAssertEqual(names(rows), ["waiting", "busy", "started"], "A session that starts waiting rises to the top")
    }

    func testTheListOrdersByTheTimesItIsGiven() {
        // While the pointer is over the list, the times from when it came over order it, and a new room goes last.
        let rooms = [room("a", minutesAgo: 1), room("b", minutesAgo: 2), room("new", minutesAgo: 0)]
        let frozen = ["accountGlobal/a": now.addingTimeInterval(-600), "accountGlobal/b": now]
        let rows = RoomSidebarPresentation.rows(for: .all, rooms: rooms, collapsed: [], showsAllChildren: [],
                                                activity: { frozen[$0.id] ?? .distantPast })
        XCTAssertEqual(names(rows), ["b", "a", "new"])
    }

    // MARK: Places

    func testALiveSessionsRoomIsOnThisMacAndOnlineOnceItEnds() {
        XCTAssertEqual(room("live", minutesAgo: 1, session: session("a")).place, .thisMac)
        XCTAssertEqual(room("ended", minutesAgo: 1).place, .online, "An ended session's room, or a teammate's")
        XCTAssertEqual(room("scratch", on: .local, minutesAgo: 1).place, .thisMac)
        XCTAssertEqual(room("junction", on: .global, minutesAgo: 1).place, .online)
    }

    func testThePlaceIconSwapsWhileItsConnectionIsDown() {
        typealias Icon = SidebarPlaceIcon
        XCTAssertEqual(Icon.icon(place: .online, isLocalServer: false, connection: .up), .online)
        XCTAssertEqual(Icon.icon(place: .online, isLocalServer: false, connection: .connecting), .onlineDown)
        XCTAssertEqual(Icon.icon(place: .online, isLocalServer: false, connection: .offline), .onlineDown)
        XCTAssertEqual(Icon.icon(place: .online, isLocalServer: false, connection: .failed), .onlineFailed)
        XCTAssertEqual(Icon.icon(place: .thisMac, isLocalServer: false, connection: .up), .thisMac,
                       "A live session's room")
        XCTAssertEqual(Icon.icon(place: .thisMac, isLocalServer: false, connection: .offline), .onlineDown,
                       "A live session's room is on the account's connection")
        XCTAssertEqual(Icon.icon(place: .thisMac, isLocalServer: true, connection: .offline), .thisMac)
        XCTAssertEqual(Icon.icon(place: .thisMac, isLocalServer: true, connection: .failed), .thisMacFailed)
        XCTAssertEqual(Icon.onlineDown.help(connection: .connecting), "Connecting…")
        XCTAssertEqual(Icon.onlineDown.help(connection: .offline), "Offline")
        XCTAssertEqual(Icon.thisMacFailed.help(connection: .failed), "Can't reach this Mac's server")
        XCTAssertFalse(Icon.online.isDown)
        XCTAssertTrue(Icon.onlineFailed.isDown)
    }

    func testTheConnectionReadsDownOnlyOnceItHasBeenDownAWhile() {
        XCTAssertEqual(SidebarConnection(status: .disconnected, isDown: false), .up)
        XCTAssertEqual(SidebarConnection(status: .connecting, isDown: true), .connecting)
        XCTAssertEqual(SidebarConnection(status: .disconnected, isDown: true), .offline)
        XCTAssertEqual(SidebarConnection(status: .failed("no"), isDown: true), .failed)
        XCTAssertEqual(SidebarConnection(status: .connected, isDown: true), .up)
    }

    // MARK: The filter

    func testEachFilterValueHoldsItsRooms() {
        let rooms = [
            room("live", minutesAgo: 1, session: session("a")), room("district", minutesAgo: 2),
            room("old", minutesAgo: 3, archived: true), room("scratch", on: .local, minutesAgo: 4),
        ]
        func list(_ filter: SidebarFilter) -> [String] {
            names(RoomSidebarPresentation.rows(for: filter, rooms: rooms, collapsed: [], showsAllChildren: []))
        }
        XCTAssertEqual(list(.all), ["live", "district", "scratch"])
        XCTAssertEqual(list(.sessions), ["live"])
        XCTAssertEqual(list(.rooms), ["district", "scratch"])
        XCTAssertEqual(list(.archived), ["old"])
    }

    func testSignedOutOffersAllAndArchived() {
        XCTAssertEqual(SidebarFilter.offered(signedIn: true), [.all, .sessions, .rooms, .archived])
        XCTAssertEqual(SidebarFilter.offered(signedIn: false), [.all, .archived])
    }

    func testArchivedIsFlat() {
        let rooms = [room("district", minutesAgo: 5, archived: true),
                     room("design", minutesAgo: 1, parent: "district", archived: true)]
        XCTAssertEqual(names(RoomSidebarPresentation.rows(for: .archived, rooms: rooms, collapsed: [],
                                                          showsAllChildren: [])),
                       ["design", "district"])
    }

    func testTheEmptyLines() {
        XCTAssertNil(RoomSidebarPresentation.emptyLine(filter: .all, isSearching: false, isSearchingMessages: false))
        XCTAssertEqual(RoomSidebarPresentation.emptyLine(filter: .sessions, isSearching: false,
                                                         isSearchingMessages: false), "No live sessions.")
        XCTAssertEqual(RoomSidebarPresentation.emptyLine(filter: .rooms, isSearching: false,
                                                         isSearchingMessages: false), "No rooms.")
        XCTAssertEqual(RoomSidebarPresentation.emptyLine(filter: .archived, isSearching: false,
                                                         isSearchingMessages: false), "No archived rooms.")
        XCTAssertEqual(RoomSidebarPresentation.emptyLine(filter: .all, isSearching: true, isSearchingMessages: true),
                       "Searching…")
        XCTAssertEqual(RoomSidebarPresentation.emptyLine(filter: .rooms, isSearching: true,
                                                         isSearchingMessages: false), "Nothing matches.")
    }

    // MARK: Company rooms

    func testAnActiveChildLiftsItsParentsBlock() {
        let rooms = [
            room("fee-ledger", minutesAgo: 9), room("district", minutesAgo: 60),
            room("design", minutesAgo: 1, parent: "district"), room("ops", minutesAgo: 30, parent: "district"),
        ]
        let rows = RoomSidebarPresentation.rows(for: .all, rooms: rooms, collapsed: [], showsAllChildren: [])
        XCTAssertEqual(names(rows), ["v district", "  design", "  ops", "fee-ledger"])
    }

    func testAChildWhoseParentIsAbsentStandsAtTheTop() {
        let rooms = [room("district", minutesAgo: 60, archived: true),
                     room("design", minutesAgo: 1, parent: "district"),
                     room("listings", on: .global, minutesAgo: 2, parent: "district")]
        let rows = RoomSidebarPresentation.rows(for: .all, rooms: rooms, collapsed: [], showsAllChildren: [])
        XCTAssertEqual(names(rows), ["design", "listings"], "Parents are matched on the same server only")
    }

    func testARoomInsideAChildSitsUnderTheTopParent() {
        let rooms = [room("district", minutesAgo: 60), room("design", minutesAgo: 10, parent: "district"),
                     room("icons", minutesAgo: 1, parent: "design")]
        XCTAssertEqual(names(RoomSidebarPresentation.rows(for: .all, rooms: rooms, collapsed: [],
                                                          showsAllChildren: [])),
                       ["v district", "  icons", "  design"])
    }

    func testAClosedParentHidesItsRoomsAndShowsTheirUnreadDot() {
        let rooms = [room("district", minutesAgo: 60), room("design", minutesAgo: 1, parent: "district", unread: true)]
        XCTAssertEqual(names(RoomSidebarPresentation.rows(for: .all, rooms: rooms,
                                                          collapsed: ["accountGlobal/district"],
                                                          showsAllChildren: [])),
                       ["> district *"])
    }

    func testMoreThanFiveRoomsInsideShowTheFiveNewestAndAMoreRow() {
        let rooms = [room("district", minutesAgo: 60)]
            + (1...7).map { room("child-\($0)", minutesAgo: Double($0), parent: "district") }
        let closed = RoomSidebarPresentation.rows(for: .all, rooms: rooms, collapsed: [], showsAllChildren: [])
        XCTAssertEqual(names(closed), ["v district"] + (1...5).map { "  child-\($0)" } + ["2 more"])
        let open = RoomSidebarPresentation.rows(for: .all, rooms: rooms, collapsed: [],
                                                showsAllChildren: ["accountGlobal/district"])
        XCTAssertEqual(names(open), ["v district"] + (1...7).map { "  child-\($0)" } + ["less"])
    }

    // MARK: Search

    private var searchRooms: [SidebarRoom] {
        [
            room("general-store", on: .global, minutesAgo: 2880),
            room("district", minutesAgo: 60), room("design", minutesAgo: 30, parent: "district",
                                                   description: "Store page work"),
            room("store-pricing", on: .global, minutesAgo: 50_000, archived: true),
            room("unread-counts", minutesAgo: 1, session: session("u", folder: "cowchat", branch: "p/store-badge")),
            room("scratch", on: .local, minutesAgo: 5, messages: true),
        ]
    }

    private var searchJoinable: [SidebarJoinable] {
        [SidebarJoinable(name: "store-launch", server: .global, at: now.addingTimeInterval(-4320),
                         source: .room(searchRooms[0].room)),
         SidebarJoinable(name: "infra-alerts", server: .global, at: now, source: .room(searchRooms[0].room))]
    }

    func testTheSearchsThreeGroups() {
        let rows = RoomSidebarPresentation.searchRows(query: " store ", filter: .all, rooms: searchRooms,
                                                      joinable: searchJoinable)
        XCTAssertEqual(names(rows), [
            "unread-counts", "scratch", "district", "  design", "general-store",
            "[To join]", "join store-launch", "[Archived]", "store-pricing",
        ], "A branch, a message, a child's description with its parent, and a name all match")
    }

    func testTheFilterNarrowsTheSearch() {
        func search(_ filter: SidebarFilter) -> [String] {
            names(RoomSidebarPresentation.searchRows(query: "store", filter: filter, rooms: searchRooms,
                                                     joinable: searchJoinable))
        }
        XCTAssertEqual(search(.sessions), ["unread-counts"])
        XCTAssertEqual(search(.rooms), ["scratch", "district", "  design", "general-store", "[To join]",
                                        "join store-launch", "[Archived]", "store-pricing"])
        XCTAssertEqual(search(.archived), ["store-pricing"], "Archived shows its group without the label")
    }

    func testTheSearchIgnoresAClosedParent() {
        let rows = RoomSidebarPresentation.rows(
            RoomSidebarPresentation.blocks([room("district", minutesAgo: 60),
                                            room("design", minutesAgo: 1, parent: "district")]),
            collapsed: ["accountGlobal/district"], showsAllChildren: [], isSearching: true)
        XCTAssertEqual(names(rows), ["district", "  design"])
    }

    func testASearchWithNoMatchHasNoRows() {
        let rooms = searchRooms.map { room -> SidebarRoom in
            var room = room
            room.matchesMessages = false
            return room
        }
        XCTAssertTrue(RoomSidebarPresentation.searchRows(query: "zzz", filter: .all, rooms: rooms,
                                                         joinable: searchJoinable).isEmpty)
    }

    // MARK: Time

    func testTheShortTimeAtEachBoundary() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        func short(_ seconds: TimeInterval) -> String {
            now.addingTimeInterval(-seconds).cowchatShortTime(relativeTo: now, calendar: calendar)
        }
        XCTAssertEqual(short(-30), "now", "A time after the tick")
        XCTAssertEqual(short(59), "now")
        XCTAssertEqual(short(60), "1m")
        XCTAssertEqual(short(3599), "59m")
        XCTAssertEqual(short(3600), "1h")
        XCTAssertEqual(short(86_399), "23h")
        XCTAssertEqual(short(86_400), "1d")
        XCTAssertEqual(short(7 * 86_400 - 1), "6d")
        let septemberThird = ISO8601DateFormatter().date(from: "2026-09-03T12:00:00Z")!
        XCTAssertEqual(septemberThird.cowchatShortTime(relativeTo: now, calendar: calendar), "Sep 3")
    }
}
