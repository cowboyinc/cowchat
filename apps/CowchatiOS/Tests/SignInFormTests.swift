import XCTest
@testable import CowchatiOS
import CowchatKit

final class SignInFormTests: XCTestCase {
    func testStartsOnGlobalAndCannotConnectUntilSomethingRecognisedIsPasted() {
        var form = SignInForm()
        XCTAssertEqual(form.server, .defaultGlobal)
        XCTAssertFalse(form.canConnect)
        XCTAssertNil(form.found)

        form.pasted = "ask Ryan for an invite"
        XCTAssertFalse(form.canConnect)

        form.pasted = "cinv_abc"
        XCTAssertTrue(form.canConnect)
        XCTAssertEqual(form.found, "Invite")
        XCTAssertEqual(form.credential, .invite("cinv_abc"))
    }

    func testOwnServerNeedsAHostAndAValidPort() {
        var form = SignInForm()
        form.place = .ownServer
        form.pasted = "k1"
        XCTAssertNil(form.server)
        XCTAssertFalse(form.canConnect)

        form.host = " mac.local "
        XCTAssertEqual(form.server, .tcp(host: "mac.local", port: 9229))
        XCTAssertTrue(form.canConnect)

        form.port = "99999"
        XCTAssertNil(form.server)
    }

    func testOtherServerNeedsASecureWebSocketURL() {
        var form = SignInForm()
        form.place = .otherServer
        form.pasted = "csoc_a"
        form.url = "ws://chat.example.test"
        XCTAssertNil(form.server)

        form.url = " wss://chat.example.test/ws "
        XCTAssertEqual(form.server, .global(url: "wss://chat.example.test/ws"))
        XCTAssertEqual(form.found, "Account key")
        XCTAssertTrue(form.canConnect)
    }

    func testProviderSignInIsOfferedOnlyForWebSocketServers() {
        var form = SignInForm()
        XCTAssertEqual(form.providerURL, ServerChoice.defaultGlobalURL)
        form.place = .ownServer
        XCTAssertNil(form.providerURL)
        form.place = .otherServer
        form.url = "wss://chat.example.test/ws"
        XCTAssertEqual(form.providerURL, "wss://chat.example.test/ws")
    }
}
