import SwiftUI
import XCTest
@testable import CowchatMac

/// On every page the list's first line of text sits 32 under the header's last line of text, and a header with no line
/// under its title keeps no empty one (navigation round 2, section 4.2 as amended).
@MainActor
final class PageLayoutTests: XCTestCase {
    func testAHeaderWithNoLineEndsUnderItsTitleOrItsAccessory() {
        XCTAssertEqual(height(of: PageHeader(title: "Friends", caption: "4 friends")), 58)
        XCTAssertEqual(height(of: PageHeader(title: "Friends")), 10 + 28, "Under the title's line")
        let inbox = PageHeader(title: "Inbox") { Color.clear.frame(width: 120, height: CowchatPillButtonStyle.height) }
        XCTAssertEqual(height(of: inbox), 10 + 36, "Under the accessory, which is taller")
    }

    func testEachListsFirstLineOfTextSits32UnderTheHeadersLastText() {
        XCTAssertEqual(InboxRow.textInset, 16)
        XCTAssertEqual(PageLayout.belowTitle(accessoryHeight: CowchatPillButtonStyle.height), 4,
                       "Mark all read reaches 4 under the Inbox title's line")
        XCTAssertEqual(PageLayout.belowTitle(accessoryHeight: 20), 0, "A shorter accessory adds nothing")
        XCTAssertEqual(PageLayout.listTop(firstTextInset: InboxRow.textInset, belowLastText: 4), 12)
        XCTAssertEqual(TeamGroupHead.textInset, 6, "Friends starts with a group's head")
        XCTAssertEqual(PageLayout.listTop(firstTextInset: TeamGroupHead.textInset), 26)
        XCTAssertEqual(PageLayout.listTop(firstTextInset: 0), 32, "Team's labels' line has no inset")
    }

    private func height(of view: some View) -> CGFloat {
        NSHostingView(rootView: view.frame(width: 600)).fittingSize.height
    }
}
