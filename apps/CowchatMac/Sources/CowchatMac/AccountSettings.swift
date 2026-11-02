import AppKit
import SwiftUI

/// Settings' Account page, the first in Settings: the name and the handle in 40 fields with Save, the sign-ins the
/// account is linked to with Link for the rest, and Sign out. Every button is 36.
struct AccountSettings: View {
    @ObservedObject var social: SocialStore
    let signOut: () -> Void
    @State private var name = ""
    @State private var handle = ""
    @State private var saveError: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 28) {
            VStack(alignment: .leading, spacing: 12) {
                // copy:
                CapsuleField(placeholder: "Your name", text: $name, onSubmit: save)
                // copy:
                CapsuleField(placeholder: "Your username", text: $handle, leading: "@", onSubmit: save)
                if let saveError {
                    Text(saveError)
                        .gallopText(.bodyS, color: SemanticColor.textError)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                // copy:
                Button("Save", action: save)
                    .buttonStyle(CowchatPillButtonStyle(prominent: true))
                    .disabled(!canSave)
                    .macAccessibleAction(label: "Save", isEnabled: canSave, action: save)
            }
            VStack(alignment: .leading, spacing: 4) {
                // copy:
                SettingsListHead(title: "Signed in with")
                VStack(spacing: 0) {
                    ForEach(Array(social.identities.enumerated()), id: \.element.provider) { index, identity in
                        if index > 0 { AgentHookSeparator() }
                        SignInRow(provider: identity.provider) {
                            Text(identity.label)
                                .gallopText(.bodyS, color: SemanticColor.textTertiary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                    ForEach(Array(social.availableLinkProviders.enumerated()), id: \.element) { index, provider in
                        if index > 0 || !social.identities.isEmpty { AgentHookSeparator() }
                        SignInRow(provider: provider) { linkEnd(provider) }
                    }
                }
                .gallopCard(cornerRadius: 14)
            }
            // copy:
            Button("Sign out", action: signOut)
                .buttonStyle(CowchatPillButtonStyle())
                .macAccessibleAction(label: "Sign out", action: signOut)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onAppear(perform: fill)
        .onChange(of: social.account) { _ in fill() }
        .onChange(of: name) { _ in saveError = nil }
        .onChange(of: handle) { _ in saveError = nil }
        .onDisappear { if social.linkingProvider != nil { social.cancelLink() } }
    }

    /// Link on a provider the account isn't linked to, or the wait for the browser with Cancel.
    @ViewBuilder private func linkEnd(_ provider: String) -> some View {
        if social.linkingProvider == provider {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small).frame(width: 16, height: 16)
                // copy:
                Button("Cancel") { social.cancelLink() }
                    .buttonStyle(CowchatPillButtonStyle())
                    .macAccessibleAction(label: "Cancel linking") { social.cancelLink() }
            }
        } else {
            // copy:
            Button("Link") { social.linkProvider(provider) }
                .buttonStyle(CowchatPillButtonStyle())
                .disabled(social.busy || social.linkingProvider != nil)
                .macAccessibleAction(label: "Link \(SignInProvider.name(of: provider))",
                                     isEnabled: !social.busy && social.linkingProvider == nil) {
                    social.linkProvider(provider)
                }
        }
    }

    private var trimmedName: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var trimmedHandle: String { SocialAPI.normalizedHandle(handle) }

    private var canSave: Bool {
        !trimmedName.isEmpty && !trimmedHandle.isEmpty && !social.busy && social.linkingProvider == nil
            && (trimmedName != social.account?.displayName || trimmedHandle != social.account?.handle)
    }

    private func fill() {
        name = social.account?.displayName ?? ""
        handle = social.account?.handle ?? ""
    }

    private func save() {
        guard canSave else { return }
        Task {
            if !(await social.saveProfile(displayName: trimmedName, handle: trimmedHandle)) {
                saveError = social.errorMessage
            }
        }
    }
}

/// A list's head in Settings, the sidebar's section label: Caption in Text/Tertiary, with 4 under it.
struct SettingsListHead: View {
    let title: String

    var body: some View {
        Text(title)
            .gallopText(.caption, color: SemanticColor.textTertiary)
            .accessibilityAddTraits(.isHeader)
    }
}

/// A sign-in provider's name as the Account page writes it.
enum SignInProvider {
    // copy:
    static func name(of provider: String) -> String { provider == "github" ? "GitHub" : "Google" }
}

/// One sign-in on the Account page, drawn as the agents rows are: 12 above and below a 36 line holding the provider's
/// mark in a 24 box, 12, its name in Body M, and the row's end.
private struct SignInRow<End: View>: View {
    let provider: String
    @ViewBuilder let end: () -> End

    var body: some View {
        HStack(spacing: 12) {
            ProviderMark(provider: provider).frame(width: 24, height: 24)
            Text(SignInProvider.name(of: provider)).gallopText(.bodyM, color: SemanticColor.textPrimary).lineLimit(1)
            Spacer(minLength: 12)
            end()
        }
        .frame(height: 36)
        .padding(.vertical, 12)
    }
}

/// Settings › Agents' second list: the account's bots, each with Connect, which shows a new credential once, and
/// Revoke in its ⋯, then a New bot row. The list sits under Claude Code and Codex, in a card of its own.
struct BotSettings: View {
    @ObservedObject var social: SocialStore
    @State private var isCreating = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            // copy:
            SettingsListHead(title: "Bots")
            VStack(spacing: 0) {
                ForEach(social.agents) { bot in
                    BotRow(bot: bot, busy: social.busy, connect: {
                        Task { await social.connectAgent(bot.id) }
                    }, revoke: {
                        Task { await social.revokeAgent(bot.id) }
                    })
                    AgentHookSeparator()
                }
                NewBotRow { isCreating = true }
            }
            .gallopCard(cornerRadius: 14)
        }
        .sheet(isPresented: Binding(
            get: { isCreating || social.issuedAgent != nil },
            set: { if !$0 { isCreating = false; social.dismissAgentCredential() } }
        )) {
            // Creating a bot shows its credential in the same sheet.
            if let issued = social.issuedAgent {
                BotCredentialSheet(issued: issued, endpoint: social.endpoint) {
                    isCreating = false
                    social.dismissAgentCredential()
                }
            } else {
                NewBotSheet(social: social) { isCreating = false }
            }
        }
    }
}

/// One bot, drawn as the agents rows are: the 20 chatbot mark in a 24 box, 12, its name in Body M, then Connect and a
/// 24 ⋯ holding Revoke, which ends the row at the card's edge as the agents rows' ends do.
private struct BotRow: View {
    let bot: SocialAgent
    let busy: Bool
    let connect: () -> Void
    let revoke: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            GallopIconView(icon: .chatbot, fallbackSystemName: "cpu", size: 20)
                .foregroundStyle(SemanticColor.iconSecondary)
                .frame(width: 24, height: 24)
            Text(bot.name).gallopText(.bodyM, color: SemanticColor.textPrimary).lineLimit(1).truncationMode(.tail)
            Spacer(minLength: 12)
            HStack(spacing: 8) {
                // copy:
                Button("Connect", action: connect)
                    .buttonStyle(CowchatPillButtonStyle())
                    .disabled(busy)
                    .macAccessibleAction(label: "Connect \(bot.name)", isEnabled: !busy, action: connect)
                // copy:
                PageRowMoreMenu(isVisible: true, accessibilityLabel: "Actions for \(bot.name)") {
                    // copy:
                    Button("Revoke", role: .destructive, action: revoke)
                }
            }
        }
        .frame(height: 36)
        .padding(.vertical, 12)
        .contentShape(Rectangle())
        .contextMenu {
            // copy:
            Button("Revoke", role: .destructive, action: revoke)
        }
    }
}

/// The list's last row, which opens the name-and-create sheet: the 20 add mark in a 24 box, 12, "New bot" in Body M.
private struct NewBotRow: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                GallopIconView(icon: .add, fallbackSystemName: "plus", size: 20)
                    .foregroundStyle(SemanticColor.iconSecondary)
                    .frame(width: 24, height: 24)
                // copy:
                Text("New bot").gallopText(.bodyM, color: SemanticColor.textSecondary)
                Spacer(minLength: 0)
            }
            .frame(height: 36)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .macAccessibleAction(label: "New bot", action: action)
    }
}

/// The frame the bot sheets share with the Join sheet: 480 wide on Surface/600, 26 padding, parts 20 apart, the title
/// in Heading 4 with an optional Body M Text/Tertiary line 3 under it, and the buttons at the bottom.
private struct BotSheetFrame<Content: View, Buttons: View>: View {
    let title: String
    var line: String?
    @ViewBuilder let content: () -> Content
    @ViewBuilder let buttons: () -> Buttons

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .gallopText(.h4, color: SemanticColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let line {
                    Text(line)
                        .gallopText(.bodyM, color: SemanticColor.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            content()
            HStack(spacing: 10) { buttons() }
                .buttonStyle(CowchatPillButtonStyle())
        }
        .padding(26)
        .frame(width: 480, alignment: .leading)
        .fixedSize(horizontal: false, vertical: true)
        .background(SemanticColor.surface600)
    }
}

/// New bot: its name in a 40 field, then Cancel and Create at the right. Escape is Cancel and Return is Create.
private struct NewBotSheet: View {
    @ObservedObject var social: SocialStore
    let cancel: () -> Void
    @State private var name = ""

    var body: some View {
        // copy:
        BotSheetFrame(title: "New bot") {
            VStack(alignment: .leading, spacing: 8) {
                // copy:
                CapsuleField(placeholder: "Bot name", text: $name, onSubmit: create)
                if let error = social.errorMessage {
                    Text(error)
                        .gallopText(.caption, color: SemanticColor.textError)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
        } buttons: {
            Spacer(minLength: 0)
            // copy:
            Button("Cancel", action: cancel)
                .keyboardShortcut(.cancelAction)
                .macAccessibleAction(label: "Cancel", action: cancel)
            Button(action: create) {
                HStack(spacing: 6) {
                    if social.busy { ProgressView().controlSize(.small).frame(width: 16, height: 16) }
                    // copy:
                    Text("Create")
                }
            }
            .buttonStyle(CowchatPillButtonStyle(prominent: true))
            .keyboardShortcut(.defaultAction)
            .disabled(!canCreate)
            .macAccessibleAction(label: "Create bot", isEnabled: canCreate, action: create)
        }
        .onAppear { name = SocialStarterNames.bot(social.account?.displayName ?? "Cowboy") }
    }

    private var canCreate: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !social.busy }

    private func create() {
        guard canCreate else { return }
        Task { await social.createAgent(name: name.trimmingCharacters(in: .whitespacesAndNewlines)) }
    }
}

/// A bot's new credential, shown once: the key in a read-only 40 capsule, Copy command (the flags every Cowchat agent
/// command takes, the bot's ID among them, which stays off the screen), Copy key and Done.
private struct BotCredentialSheet: View {
    let issued: SocialCreatedAgent
    let endpoint: URL
    let done: () -> Void
    @State private var copied: String?

    var body: some View {
        // copy:
        BotSheetFrame(title: "Connect \(issued.agent.name)", line: "Copy its key now. It won't be shown again.") {
            Text(verbatim: issued.apiKey)
                .font(.system(size: 13, design: .monospaced))
                .foregroundStyle(SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
                .padding(.horizontal, 13)
                .frame(maxWidth: .infinity, minHeight: 40, maxHeight: 40, alignment: .leading)
                .background(SemanticColor.textfieldDefault, in: Capsule())
                .overlay { Capsule().strokeBorder(SemanticColor.borderDefault, lineWidth: 1) }
        } buttons: {
            // copy:
            copyButton("Copy command", value: BotCredential.command(for: issued, endpoint: endpoint))
            Spacer(minLength: 0)
            // copy:
            copyButton("Copy key", value: issued.apiKey)
            // copy:
            Button("Done", action: done)
                .buttonStyle(CowchatPillButtonStyle(prominent: true))
                .keyboardShortcut(.defaultAction)
                .macAccessibleAction(label: "Done", action: done)
        }
    }

    private func copyButton(_ title: String, value: String) -> some View {
        let copy = {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(value, forType: .string)
            copied = title
        }
        return Button(action: copy) {
            // The wider label holds the width, so the button does not move when it reads Copied.
            ZStack {
                Text(title).hidden()
                // copy:
                Text(copied == title ? "Copied" : title)
            }
        }
        .macAccessibleAction(label: title, action: copy)
    }
}

/// What Copy command puts on the pasteboard: the flags every Cowchat agent command takes for this bot.
enum BotCredential {
    static func command(for issued: SocialCreatedAgent, endpoint: URL) -> String {
        let name = issued.agent.name.replacingOccurrences(of: "'", with: "'\\''")
        return "--url \(endpoint.absoluteString) --key \(issued.apiKey) --name '\(name)' --agent-id \(issued.agent.id)"
    }
}
