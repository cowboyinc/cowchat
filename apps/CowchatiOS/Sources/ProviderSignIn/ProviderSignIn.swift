import Foundation
import CowchatKit

enum ProviderSignInResult: Equatable {
    case signedIn(deviceKey: String, account: SocialAccount)
    case cancelled
    case failed(String)
}

protocol ProviderSignInBackend {
    func start(provider: String, challenge: String, port: UInt16) async throws -> SocialOAuthStart
    func exchange(requestID: String, verifier: String, completionCode: String) async throws -> SocialOAuthExchange
}

@MainActor
protocol SignInBrowser {
    /// `onClose` runs only when the person closes the browser, never after `close()`.
    func open(_ url: URL, onClose: @escaping () -> Void) throws
    func close()
}

struct SocialAPIBackend: ProviderSignInBackend {
    let api: SocialAPI

    func start(provider: String, challenge: String, port: UInt16) async throws -> SocialOAuthStart {
        try await api.request(["auth", "social", "start"], auth: false, method: "POST",
                              body: ["provider": provider, "code_challenge": challenge, "callback_port": Int(port)])
    }

    func exchange(requestID: String, verifier: String, completionCode: String) async throws -> SocialOAuthExchange {
        try await api.request(["auth", "social", "exchange"], auth: false, method: "POST",
                              body: ["request_id": requestID, "code_verifier": verifier, "completion_code": completionCode])
    }
}

/// The Mac app's loopback sign-in, run while the app stays in the foreground under an in-app web sheet:
/// the server's last redirect goes to `http://127.0.0.1:<port>/cowchat/oauth`, which this process is listening on.
@MainActor
struct ProviderSignIn {
    let backend: ProviderSignInBackend
    let browser: SignInBrowser

    private struct BrowserClosed: Error {}

    func run(provider: String) async -> ProviderSignInResult {
        let callback = SocialOAuthCallback()
        defer { callback.cancel() }
        do {
            let port = try await callback.start()
            let verifier = try SocialAPI.verifier()
            let start = try await backend.start(provider: provider, challenge: SocialAPI.challenge(verifier), port: port)
            guard Self.isProviderPage(start.authorizationUrl, provider: provider) else {
                throw CowchatConnectionError.transport("Could not open secure browser sign-in.")
            }
            callback.expect(requestID: start.requestId)
            try browser.open(start.authorizationUrl) { callback.cancel(BrowserClosed()) }
            let completionCode: String
            do {
                completionCode = try await callback.wait(requestID: start.requestId, expiresIn: start.expiresIn)
            } catch {
                browser.close()
                throw error
            }
            browser.close()
            let reply = try await backend.exchange(requestID: start.requestId, verifier: verifier, completionCode: completionCode)
            guard reply.status == "complete", let key = reply.deviceKey, key.hasPrefix("csoc_"), let account = reply.account else {
                throw CowchatConnectionError.invalidResponse
            }
            return .signedIn(deviceKey: key, account: account)
        } catch is BrowserClosed {
            return .cancelled
        } catch is CancellationError {
            return .cancelled
        } catch {
            return .failed((error as? LocalizedError)?.errorDescription ?? error.localizedDescription)
        }
    }

    private static func isProviderPage(_ url: URL, provider: String) -> Bool {
        url.scheme == "https" && url.user == nil && url.password == nil
            && url.host == (provider == "github" ? "github.com" : "accounts.google.com")
            && (url.port == nil || url.port == 443)
    }
}
