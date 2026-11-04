import SwiftUI
import XCTest
@testable import CowchatMac

/// On every page the list's first line of text sits 32 under the header's last line of text, and a header with no line
/// under its title keeps no empty one (navigation round 2, section 4.2 as amended). `ProtoPageTopTests` measures the
/// same on the pages as they are drawn.
@MainActor
final class PageLayoutTests: XCTestCase {
    func testAHeaderWithNoLineEndsUnderItsTitleOrItsAccessory() {
        XCTAssertEqual(PageLayout.titleTop, 11.5, "Where a room's header puts its title")
        XCTAssertEqual(height(of: PageHeader(title: "Friends", caption: "4 friends")), 58)
        // A hosting view's fitting size rounds up to whole points.
        XCTAssertEqual(height(of: PageHeader(title: "Friends")), (11.5 + 28).rounded(.up), "Under the title's line")
        let inbox = PageHeader(title: "Inbox") { Color.clear.frame(width: 120, height: CowchatPillButtonStyle.height) }
        XCTAssertEqual(height(of: inbox), (11.5 + 36).rounded(.up), "Under the accessory, whose top is the title's")
    }

    func testEachListsFirstLineOfTextSits32UnderTheHeadersLastText() {
        XCTAssertEqual(InboxRow.textInset, 16)
        XCTAssertEqual(PageLayout.belowLastText(hasCaption: false, accessoryHeight: CowchatPillButtonStyle.height), 8,
                       "Mark all read reaches 8 under the Inbox title's line")
        XCTAssertEqual(PageLayout.belowLastText(hasCaption: false, accessoryHeight: 20), 0,
                       "A shorter accessory adds nothing")
        XCTAssertEqual(PageLayout.belowLastText(hasCaption: true), 1.5, "The 58 header ends 1.5 under its line")
        XCTAssertEqual(PageLayout.listTop(firstTextInset: InboxRow.textInset, belowLastText: 8), 8)
        XCTAssertEqual(TeamGroupHead.textInset, 6, "Friends starts with a group's head")
        XCTAssertEqual(PageLayout.listTop(firstTextInset: TeamGroupHead.textInset, belowLastText: 1.5), 24.5)
        XCTAssertEqual(PageLayout.listTop(firstTextInset: 0, belowLastText: 1.5), 30.5,
                       "Team's labels' line has no inset")
    }

    private func height(of view: some View) -> CGFloat {
        NSHostingView(rootView: view.frame(width: 600)).fittingSize.height
    }
}
