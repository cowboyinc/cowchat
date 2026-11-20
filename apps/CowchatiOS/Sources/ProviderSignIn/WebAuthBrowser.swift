import AuthenticationServices
import UIKit
import CowchatKit

@MainActor
final class WebAuthBrowser: NSObject, SignInBrowser, ASWebAuthenticationPresentationContextProviding {
    private let ephemeral: Bool
    private var session: ASWebAuthenticationSession?

    init(ephemeral: Bool) { self.ephemeral = ephemeral }

    func open(_ url: URL, onClose: @escaping () -> Void) throws {
        // No callback scheme: the sheet never completes by itself. The loopback listener decides when it is done.
        let session = ASWebAuthenticationSession(url: url, callbackURLScheme: nil) { [weak self] _, _ in
            Task { @MainActor in
                guard let self, self.session != nil else { return }
                self.session = nil
                onClose()
            }
        }
        session.presentationContextProvider = self
        session.prefersEphemeralWebBrowserSession = ephemeral
        self.session = session
        guard session.start() else {
            self.session = nil
            throw CowchatConnectionError.transport("Could not open secure browser sign-in.")
        }
    }

    func close() {
        let session = session
        self.session = nil
        session?.cancel()
    }

    nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        MainActor.assumeIsolated { Self.keyWindow ?? ASPresentationAnchor() }
    }

    static var keyWindow: UIWindow? {
        UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow }
            .first
    }
}
