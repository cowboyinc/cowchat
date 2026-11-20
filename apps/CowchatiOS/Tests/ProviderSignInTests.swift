import XCTest
@testable import CowchatiOS
import CowchatKit

private let code = String(repeating: "c", count: 40)
private let account = SocialAccount(id: "acct-1", handle: "ada", displayName: "Ada Lovelace")

private final class FakeBackend: ProviderSignInBackend {
    var startHost = "github.com"
    var expiresIn = 600
    var exchangeResult: Result<SocialOAuthExchange, Error> =
        .success(SocialOAuthExchange(status: "complete", deviceKey: "csoc_device", account: account))
    var startedPort: UInt16?
    var exchanged: (requestID: String, verifier: String, code: String)?

    func start(provider: String, challenge: String, port: UInt16) async throws -> SocialOAuthStart {
        startedPort = port
        return SocialOAuthStart(
            requestId: "req-1",
            authorizationUrl: URL(string: "https://\(startHost)/login/oauth/authorize?client_id=x")!,
            expiresIn: expiresIn
        )
    }

    func exchange(requestID: String, verifier: String, completionCode: String) async throws -> SocialOAuthExchange {
        exchanged = (requestID, verifier, completionCode)
        return try exchangeResult.get()
    }
}

/// Stands in for the web sheet: it either follows the server's final redirect to the loopback listener,
/// or the person closes it, or nothing happens.
@MainActor
private final class FakeBrowser: SignInBrowser {
    enum Behaviour { case redirect(port: () -> UInt16?), close, idle }
    var behaviour: Behaviour
    var opened: [URL] = []
    var closedByFlow = 0

    init(_ behaviour: Behaviour) { self.behaviour = behaviour }

    func open(_ url: URL, onClose: @escaping () -> Void) throws {
        opened.append(url)
        switch behaviour {
        case .redirect(let port):
            let callback = URL(string: "http://127.0.0.1:\(port()!)/cowchat/oauth?request_id=req-1&completion_code=\(code)")!
            Task { _ = try? await URLSession.shared.data(from: callback) }
        case .close:
            Task { @MainActor in onClose() }
        case .idle:
            break
        }
    }

    func close() { closedByFlow += 1 }
}

@MainActor
final class ProviderSignInTests: XCTestCase {
    func testRedirectToLoopbackExchangesForDeviceKey() async {
        let backend = FakeBackend()
        let browser = FakeBrowser(.redirect(port: { backend.startedPort }))
        let result = await ProviderSignIn(backend: backend, browser: browser).run(provider: "github")

        XCTAssertEqual(result, .signedIn(deviceKey: "csoc_device", account: account))
        XCTAssertEqual(backend.exchanged?.requestID, "req-1")
        XCTAssertEqual(backend.exchanged?.code, code)
        XCTAssertEqual(browser.closedByFlow, 1)
    }

    func testClosingTheSheetCancelsWithoutExchanging() async {
        let backend = FakeBackend()
        let browser = FakeBrowser(.close)
        let result = await ProviderSignIn(backend: backend, browser: browser).run(provider: "github")

        XCTAssertEqual(result, .cancelled)
        XCTAssertNil(backend.exchanged)
    }

    func testListenerNeverCalledExpires() async {
        let backend = FakeBackend()
        backend.expiresIn = 1
        let browser = FakeBrowser(.idle)
        let result = await ProviderSignIn(backend: backend, browser: browser).run(provider: "github")

        XCTAssertEqual(result, .failed("Sign-in expired. Please try again."))
        XCTAssertNil(backend.exchanged)
        XCTAssertEqual(browser.closedByFlow, 1)
    }

    func testExchangeErrorFails() async {
        let backend = FakeBackend()
        backend.exchangeResult = .failure(FakeError(message: "Sign-in request expired"))
        let browser = FakeBrowser(.redirect(port: { backend.startedPort }))
        let result = await ProviderSignIn(backend: backend, browser: browser).run(provider: "github")

        XCTAssertEqual(result, .failed("Sign-in request expired"))
    }

    func testExchangeWithoutDeviceKeyFails() async {
        let backend = FakeBackend()
        backend.exchangeResult = .success(SocialOAuthExchange(status: "pending", deviceKey: nil, account: nil))
        let browser = FakeBrowser(.redirect(port: { backend.startedPort }))
        let result = await ProviderSignIn(backend: backend, browser: browser).run(provider: "github")

        XCTAssertEqual(result, .failed("Cowchat returned an invalid response."))
    }

    func testAuthorizationURLOnAnotherHostIsNotOpened() async {
        let backend = FakeBackend()
        backend.startHost = "github.com.evil.example"
        let browser = FakeBrowser(.idle)
        let result = await ProviderSignIn(backend: backend, browser: browser).run(provider: "github")

        XCTAssertEqual(result, .failed("Could not open secure browser sign-in."))
        XCTAssertEqual(browser.opened, [])
    }
}
