import XCTest
@testable import CowchatMac

/// Settings moved from the sidebar's footer to the app menu's Settings… (navigation note, section 6).
final class SettingsCommandTests: XCTestCase {
    func testTheSettingsCommandPostsItsNotification() {
        let posted = expectation(forNotification: .cowchatShowSettings, object: nil)
        SettingsCommand.perform()
        wait(for: [posted], timeout: 1)
    }
}
