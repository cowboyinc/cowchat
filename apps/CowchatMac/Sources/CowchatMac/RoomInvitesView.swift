import AppKit
import SwiftUI

/// Invite management for one room: list active invites, mint single-use or
/// open ones, revoke. Fresh tokens are shown once — the server keeps only
/// hashes, so a token that leaves this sheet uncopied is gone.
struct RoomInvitesView: View {
    @EnvironmentObject private var store: ChatStore
    @Environment(\.dismiss) private var dismiss
    let room: Room

    @State private var invites: [RoomInvite] = []
    @State private var isLoading = true
    @State private var errorMessage: String?
    @State private var isMinting = false
    @State private var freshToken: String?
    @State private var freshTokenIsOpen = false
    @State private var copiedFreshField: String?
    @State private var revokingIDs: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Invites")
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    Text("Invites let a stranger into \(room.name): redeeming one vends a fresh API key plus access to this room.")
                        .gallopText(.bodyM, color: SemanticColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer()
                Button {
                    dismiss()
                } label: {
                    GallopIconView(icon: .dismiss, fallbackSystemName: "xmark", size: 14)
                        .foregroundStyle(SemanticColor.iconSecondary)
                        .frame(width: 32, height: 32)
                        .background(Circle().fill(SemanticColor.surface700))
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .help("Close")
                .macAccessibleAction(label: "Close invites") { dismiss() }
            }

            if let freshToken {
                freshTokenCard(freshToken)
            }

            if let errorMessage {
                Text(errorMessage)
                    .gallopText(.caption, color: SemanticColor.textError)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Group {
                if isLoading {
                    ProgressView()
                        .controlSize(.small)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if invites.isEmpty {
                    Text("No active invites. Mint one below, or use Copy connect prompt — every copy carries its own single-use invite.")
                        .gallopText(.bodyM, color: SemanticColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                } else {
                    ScrollView {
                        VStack(spacing: 8) {
                            ForEach(invites) { invite in
                                inviteRow(invite)
                            }
                        }
                    }
                    .scrollIndicators(.hidden)
                }
            }
            .frame(maxHeight: .infinity)

            HStack(spacing: 10) {
                Spacer()
                mintButton("New open invite", singleUse: false, style: .secondary)
                mintButton("New single-use invite", singleUse: true, style: .primary)
            }
        }
        .padding(26)
        .frame(width: 560, height: 520)
        .background(SemanticColor.surface600)
        .task { await reload() }
    }

    private enum MintButtonStyle { case primary, secondary }

    private func mintButton(
        _ title: String,
        singleUse: Bool,
        style: MintButtonStyle
    ) -> some View {
        Button(title) { mint(singleUse: singleUse) }
            .buttonStyle(.plain)
            .gallopText(
                .bodyMStrong,
                color: style == .primary
                    ? SemanticColor.buttonPrimaryTextDefault
                    : SemanticColor.buttonSecondaryTextDefault
            )
            .padding(.horizontal, 18)
            .frame(height: 38)
            .background(
                style == .primary
                    ? SemanticColor.buttonPrimaryDefault
                    : SemanticColor.buttonSecondaryDefault,
                in: Capsule()
            )
            .overlay {
                if style == .secondary {
                    Capsule().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
                }
            }
            .disabled(isMinting || !store.connectionStatus.isConnected)
            .opacity(isMinting || !store.connectionStatus.isConnected ? 0.45 : 1)
            .fixedSize()
            .macAccessibleAction(label: title, isEnabled: !isMinting) {
                mint(singleUse: singleUse)
            }
    }

    private func freshTokenCard(_ token: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(freshTokenIsOpen ? "New open invite — shown once" : "New single-use invite — shown once")
                .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
            Text(token)
                .textSelection(.enabled)
                .gallopText(.caption, color: SemanticColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
            HStack(spacing: 10) {
                freshCopyButton("Copy prompt", field: "prompt") {
                    store.connectPrompt(for: room, embedding: token)
                }
                freshCopyButton("Copy token", field: "token") { token }
                Spacer()
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(SemanticColor.surfaceGlassOnDefault, in: RoundedRectangle(cornerRadius: 12))
        .overlay {
            RoundedRectangle(cornerRadius: 12)
                .stroke(SemanticColor.buttonPrimaryDefault, lineWidth: 1)
        }
    }

    private func freshCopyButton(
        _ title: String,
        field: String,
        text: @escaping () -> String
    ) -> some View {
        Button(copiedFreshField == field ? "Copied" : title) {
            let pasteboard = NSPasteboard.general
            pasteboard.clearContents()
            pasteboard.setString(text(), forType: .string)
            copiedFreshField = field
        }
        .buttonStyle(.plain)
        .gallopText(.bodySStrong, color: SemanticColor.buttonSecondaryTextDefault)
        .padding(.horizontal, 12)
        .frame(height: 30)
        .background(SemanticColor.buttonSecondaryDefault, in: Capsule())
        .overlay {
            Capsule().stroke(SemanticColor.borderDefault, lineWidth: 0.5)
        }
        .fixedSize()
        .macAccessibleAction(label: title) {
            let pasteboard = NSPasteboard.general
            pasteboard.clearContents()
            pasteboard.setString(text(), forType: .string)
            copiedFreshField = field
        }
    }

    private func inviteRow(_ invite: RoomInvite) -> some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 8) {
                    Text(invite.singleUse ? "Single-use" : "Open")
                        .gallopText(.caption, color: SemanticColor.buttonPrimaryTextDefault)
                        .padding(.horizontal, 8)
                        .frame(height: 20)
                        .background(
                            invite.singleUse
                                ? SemanticColor.buttonPrimaryDefault
                                : SemanticColor.iconSecondary,
                            in: Capsule()
                        )
                    if invite.revoked {
                        // A spent single-use invite persists as revoked with a
                        // redemption recorded — that's success, not revocation.
                        Text(invite.singleUse && invite.redeemedCount > 0 ? "Redeemed" : "Revoked")
                            .gallopText(
                                .caption,
                                color: invite.singleUse && invite.redeemedCount > 0
                                    ? SemanticColor.textTertiary
                                    : SemanticColor.textError
                            )
                    }
                    if !invite.mine {
                        Text("by another agent")
                            .gallopText(.caption, color: SemanticColor.textTertiary)
                    }
                }
                Text(rowDetail(invite))
                    .gallopText(.caption, color: SemanticColor.textTertiary)
            }
            Spacer()
            if !invite.revoked {
                Button(revokingIDs.contains(invite.id) ? "Revoking…" : "Revoke") {
                    revoke(invite)
                }
                .buttonStyle(.plain)
                .gallopText(.bodySStrong, color: SemanticColor.textError)
                .disabled(revokingIDs.contains(invite.id))
                .macAccessibleAction(label: "Revoke invite") { revoke(invite) }
            }
        }
        .padding(.horizontal, 14)
        .frame(height: 56)
        .frame(maxWidth: .infinity)
        .background(SemanticColor.surface500, in: RoundedRectangle(cornerRadius: 12))
        .overlay {
            RoundedRectangle(cornerRadius: 12)
                .stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
        .opacity(invite.revoked ? 0.55 : 1)
    }

    private func rowDetail(_ invite: RoomInvite) -> String {
        var parts: [String] = []
        if !invite.singleUse {
            parts.append(
                invite.redeemedCount == 1
                    ? "1 redemption" : "\(invite.redeemedCount) redemptions"
            )
        }
        parts.append("id \(invite.inviteID.prefix(8))…")
        let created = invite.createdAt.cowchatRelativeTime
        if !created.isEmpty { parts.append(created) }
        return parts.joined(separator: " · ")
    }

    private func reload() async {
        isLoading = true
        do {
            invites = try await store.invites(for: room)
            errorMessage = nil
        } catch {
            errorMessage = error.localizedDescription
        }
        isLoading = false
    }

    private func mint(singleUse: Bool) {
        guard !isMinting else { return }
        isMinting = true
        errorMessage = nil
        Task {
            do {
                let token = try await store.mintInvite(for: room, singleUse: singleUse)
                freshToken = token
                freshTokenIsOpen = !singleUse
                copiedFreshField = nil
                await reload()
            } catch {
                errorMessage = error.localizedDescription
            }
            isMinting = false
        }
    }

    private func revoke(_ invite: RoomInvite) {
        revokingIDs.insert(invite.id)
        errorMessage = nil
        Task {
            do {
                try await store.revokeInvite(id: invite.id)
                await reload()
            } catch {
                errorMessage = error.localizedDescription
            }
            revokingIDs.remove(invite.id)
        }
    }
}
