import Foundation
import SwiftUI

struct FriendInvitationRoute: Codable, Equatable, Identifiable {
    let slug: String
    let endpoint: URL
    var id: String { endpoint.absoluteString + "/" + slug }

    static func parse(_ url: URL) -> Self? {
        guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.user == nil, parts.password == nil, parts.fragment == nil else { return nil }
        let slug: String
        let endpoint: URL
        if parts.scheme == "cowchat" {
            guard parts.host == "friend", parts.port == nil else { return nil }
            let segments = parts.path.split(separator: "/", omittingEmptySubsequences: false)
            guard segments.count == 2, segments[0].isEmpty else { return nil }
            slug = String(segments[1])
            let query = parts.queryItems ?? []
            guard query.count <= 1, query.allSatisfy({ $0.name == "server" && $0.value != nil }) else { return nil }
            if let value = query.first?.value {
                guard let candidate = URL(string: value), let normalized = normalizeEndpoint(candidate) else { return nil }
                endpoint = normalized
            } else { endpoint = URL(string: ConnectionProfile.defaultGlobalURLString)! }
        } else {
            guard parts.scheme == "https", parts.query == nil, let host = parts.host else { return nil }
            let segments = parts.path.split(separator: "/", omittingEmptySubsequences: false)
            guard segments.count == 3, segments[0].isEmpty, segments[1] == "friends" else { return nil }
            slug = String(segments[2])
            var origin = URLComponents()
            origin.scheme = "wss"; origin.host = host; origin.port = parts.port; origin.path = "/ws"
            guard let candidate = origin.url, let normalized = normalizeEndpoint(candidate) else { return nil }
            endpoint = normalized
        }
        guard validSlug(slug) else { return nil }
        return Self(slug: slug, endpoint: endpoint)
    }

    static func validSlug(_ slug: String) -> Bool {
        (3...40).contains(slug.utf8.count) && slug.utf8.allSatisfy {
            (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || $0 == 45 || $0 == 95
        }
    }

    static func normalizeEndpoint(_ url: URL) -> URL? {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.scheme == "wss", parts.host != nil, parts.user == nil, parts.password == nil,
              parts.query == nil, parts.fragment == nil, parts.path == "/ws" else { return nil }
        parts.host = parts.host?.lowercased()
        if parts.port == 443 { parts.port = nil }
        return parts.url
    }

    func matches(_ endpoint: URL) -> Bool { Self.normalizeEndpoint(endpoint) == Self.normalizeEndpoint(self.endpoint) }
}

struct FriendInvitationConfirmationSnapshot {
    let invitation: FriendInvitationRoute
    let scopeID: String
    let preview: SocialFriendLinkPreview

    func matches(invitation: FriendInvitationRoute, scopeID: String?) -> Bool {
        self.invitation == invitation && self.scopeID == scopeID
    }
}

struct FriendInvitationConfirmation: View {
    @ObservedObject var social: SocialStore
    let invitation: FriendInvitationRoute
    let onDismiss: () -> Void
    @State private var confirmation: FriendInvitationConfirmationSnapshot?
    @State private var failure: String?
    @State private var sending = false
    @State private var reviewRequests = false

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("A friend on Cowchat").gallopText(.h3)
            if !invitation.matches(social.endpoint) {
                Text("This invitation belongs to a different Cowchat server. Your server and sign-in have not changed.").gallopText(.bodyM)
            } else if social.phase == .awaitingBrowser {
                ProgressView("Finish signing in in your browser")
                Button("Cancel sign-in") { social.cancelSignIn() }
            } else if social.phase == .signedOut {
                Text("Sign in to preview this invitation. You will confirm before a friend request is sent.").gallopText(.bodyM)
                ForEach(social.providers, id: \.self) { provider in
                    Button("Continue with \(provider == "github" ? "GitHub" : "Google")") { social.signIn(provider: provider) }
                        .buttonStyle(CowchatPillButtonStyle(prominent: true))
                }
                if social.providers.isEmpty { Button("Load sign-in options") { Task { await social.refresh() } } }
            } else if let confirmation, confirmation.matches(invitation: invitation, scopeID: social.savedSession?.scopeId) {
                let preview = confirmation.preview
                Text("\(preview.account.displayName) (@\(preview.account.handle))").gallopText(.bodyL)
                switch preview.relationship {
                case .self: Text("This is your invitation link.")
                case .friends: Text("You are already friends.")
                case .outgoingPending: Text("Your friend request is waiting for their approval.")
                case .incomingPending:
                    Text("They already sent you a friend request. Review it to accept or decline.")
                    Button("Review request") { reviewRequests = true }
                case .none:
                    Text("Send a friend request? Friendship does not grant access to rooms or agents.").gallopText(.bodyM)
                    Button(sending ? "Sending…" : "Send friend request") {
                        sending = true
                        Task {
                            if await social.requestFriend(confirmation: confirmation) { onDismiss() }
                            else if confirmation.matches(invitation: invitation, scopeID: social.savedSession?.scopeId) { failure = social.errorMessage ?? "The friend request could not be sent. Try again." }
                            sending = false
                        }
                    }.buttonStyle(CowchatPillButtonStyle(prominent: true)).disabled(sending)
                }
            } else if let failure {
                Text(failure).gallopText(.bodyM)
                Button("Try again") { Task { await load() } }
            } else { ProgressView("Loading invitation…") }
            if confirmation != nil, let failure { Text(failure).foregroundStyle(.red) }
            HStack { Spacer(); Button("Close", action: onDismiss).keyboardShortcut(.cancelAction) }
        }
        .padding(28).frame(width: 500)
        .task(id: invitation.id + (social.savedSession?.scopeId ?? "") + String(describing: social.phase)) { await load() }
        .sheet(isPresented: $reviewRequests) { FriendsView(social: social) }
    }

    private func load() async {
        confirmation = nil; failure = nil
        guard invitation.matches(social.endpoint), social.phase == .signedIn else { return }
        do {
            guard let scope = social.savedSession?.scopeId else { return }
            let value = try await social.previewFriend(invitation)
            guard !Task.isCancelled, scope == social.savedSession?.scopeId else { return }
            confirmation = FriendInvitationConfirmationSnapshot(invitation: invitation, scopeID: scope, preview: value)
        }
        catch { if !Task.isCancelled { failure = error.localizedDescription } }
    }
}
