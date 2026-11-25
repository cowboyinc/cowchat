#if DEBUG
import Foundation
import UIKit
import CowchatKit

/// Proves that a web sheet's navigation to `http://127.0.0.1:<port>` reaches this app's listener, with no provider
/// login. Run with `-ProviderSignInLoopbackCheck ephemeral` (or `shared`) and read the `LOOPBACK_CHECK:` line.
@MainActor
enum LoopbackCheck {
    static let argument = "ProviderSignInLoopbackCheck"
    private static var started = false

    static func runIfRequested() {
        guard !started, let mode = UserDefaults.standard.string(forKey: argument) else { return }
        started = true
        Task { report(await run(ephemeral: mode != "shared")) }
    }

    private struct Closed: Error {}

    private static func run(ephemeral: Bool) async -> String {
        let callback = SocialOAuthCallback()
        defer { callback.cancel() }
        let browser = WebAuthBrowser(ephemeral: ephemeral)
        do {
            let port = try await callback.start()
            let code = String(repeating: "t", count: 40)
            let url = URL(string: "http://127.0.0.1:\(port)/cowchat/oauth?request_id=test&completion_code=\(code)")!
            callback.expect(requestID: "test")
            try browser.open(url) { callback.cancel(Closed()) }
            _ = try await callback.wait(requestID: "test", expiresIn: 45)
            let wasPresented = WebAuthBrowser.keyWindow?.rootViewController?.presentedViewController != nil
            browser.close()
            for _ in 0..<50 where WebAuthBrowser.keyWindow?.rootViewController?.presentedViewController != nil {
                try? await Task.sleep(for: .milliseconds(100))
            }
            let stillPresented = WebAuthBrowser.keyWindow?.rootViewController?.presentedViewController != nil
            if stillPresented { return "not reached (listener reached, but the sheet did not dismiss)" }
            return wasPresented ? "reached" : "reached (no in-app sheet was presented)"
        } catch is Closed {
            browser.close()
            return "not reached (the sheet was closed before the listener was called)"
        } catch {
            browser.close()
            return "not reached (\((error as? LocalizedError)?.errorDescription ?? "\(error)"))"
        }
    }

    private static func report(_ outcome: String) {
        let line = "LOOPBACK_CHECK: \(outcome)"
        print(line)
        fflush(stdout)
        NSLog("%@", line)
    }
}
#endif
