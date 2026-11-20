import SwiftUI
import CowchatKit

struct ProviderSignInSection: View {
    let serverURL: String

    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    /// Nil until the server answers.
    @State private var providers: [String]?
    @State private var running: String?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let providers {
                if providers.contains("google") { button("google", title: "Continue with Google", mark: "GoogleMark") }
                if providers.contains("github") { button("github", title: "Continue with GitHub", mark: nil) }
                if providers.contains("google") || providers.contains("github") { divider }
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
            if let error { Text(error).gallopText(.bodyM, color: SemanticColor.textError) }
        }
        .task {
            #if DEBUG
            LoopbackCheck.runIfRequested()
            #endif
            guard let api = api() else { providers = []; return }
            let capabilities: SocialCapabilities? = try? await api.request(["social", "v1", "capabilities"], auth: false)
            providers = capabilities?.providers ?? []
        }
    }

    private func api() -> SocialAPI? {
        URL(string: serverURL.trimmingCharacters(in: .whitespaces)).flatMap { try? SocialAPI(endpoint: $0) }
    }

    private var divider: some View {
        HStack(spacing: 12) {
            Rectangle().fill(SemanticColor.borderDefault).frame(height: 1)
            Text("or").gallopText(.bodyS, color: SemanticColor.textTertiary)
            Rectangle().fill(SemanticColor.borderDefault).frame(height: 1)
        }
        .padding(.vertical, 2)
        .accessibilityHidden(true)
    }

    private func button(_ provider: String, title: String, mark: String?) -> some View {
        Button { signIn(provider) } label: {
            HStack(spacing: 8) {
                if let mark { Image(mark).resizable().frame(width: 20, height: 20).accessibilityHidden(true) }
                Text(title)
            }
        }
            .buttonStyle(GallopButtonStyle(kind: .outline))
            .disabled(running != nil || store.isAdding)
            .accessibilityIdentifier("signin.\(provider)")
    }

    private func signIn(_ provider: String) {
        let url = serverURL.trimmingCharacters(in: .whitespaces)
        guard let api = api() else { return }
        running = provider
        error = nil
        Task {
            let flow = ProviderSignIn(backend: SocialAPIBackend(api: api), browser: WebAuthBrowser(ephemeral: false))
            let result = await flow.run(provider: provider)
            running = nil
            switch result {
            case .signedIn(let key, let account):
                let name = account.displayName.trimmingCharacters(in: .whitespaces)
                do {
                    _ = try await store.add(server: .global(url: url), credential: .key(key),
                                            displayName: name.isEmpty ? account.handle : name,
                                            accountID: account.id)
                    dismiss()
                } catch {
                    self.error = error.localizedDescription
                }
            case .cancelled:
                break
            case .failed(let message):
                error = message
            }
        }
    }
}
