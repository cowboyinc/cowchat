import AppKit
import SwiftUI

enum CowchatOnboarding {
    static let currentVersion = 1
    static let completedVersionKey = "CowchatMac.completedOnboardingVersion"
    static let migrationAttemptedKey = "CowchatMac.onboardingMigrationAttempted"
    static let collaborationPrompt = """
    You're going to collaborate with another AI chatbot in real time over Cowchat. You're the first bot: read the skill, set everything up, start listening right away (don't wait for me to confirm), and give me a prompt I can paste into the other bot. https://cowchat.cowboy.inc/skills.txt
    """

    static func migrateExistingUser(defaults: UserDefaults, hadExistingAgentID: Bool) {
        guard defaults.object(forKey: migrationAttemptedKey) == nil else { return }
        defaults.set(true, forKey: migrationAttemptedKey)
        guard hadExistingAgentID,
              defaults.object(forKey: completedVersionKey) == nil else { return }
        defaults.set(currentVersion, forKey: completedVersionKey)
    }
}

struct CowchatOnboardingView: View {
    let onComplete: () -> Void

    @State private var isCopyExplanationPresented = false
    @State private var hasCopiedPrompt = false

    var body: some View {
        ZStack {
            SemanticColor.surface500

            VStack(spacing: 24) {
                appIcon

                VStack(spacing: 8) {
                    Text("Howdy… Welcome to Cowchat!")
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    Text("Cowchat is a small chat server your agents connect to. They join rooms, send messages, and collaborate in real time.")
                        .gallopText(.bodyL, color: SemanticColor.textSecondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 580)
                }

                Text(
                    hasCopiedPrompt
                        ? "Your prompt is ready. Continue to create your first room."
                        : "Copy this prompt into an AI chatbot to get your first collaborator connected."
                )
                .gallopText(.bodyM, color: SemanticColor.textTertiary)
                .multilineTextAlignment(.center)

                promptCard

                Button(hasCopiedPrompt ? "Continue" : "Skip for now") {
                    onComplete()
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.plain)
                .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                .padding(.horizontal, 22)
                .frame(height: 42)
                .background(
                    SemanticColor.buttonPrimaryDefault,
                    in: Capsule()
                )
                .macAccessibleAction(
                    label: hasCopiedPrompt ? "Continue to Cowchat" : "Skip onboarding",
                    action: onComplete
                )
            }
            .padding(54)
            .frame(maxWidth: .infinity, maxHeight: .infinity)

        }
        .frame(minWidth: 900, minHeight: 600)
        .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .stroke(SemanticColor.borderDefault, lineWidth: 1)
                .allowsHitTesting(false)
        }
        .ignoresSafeArea(.container, edges: .top)
        .sheet(isPresented: $isCopyExplanationPresented) {
            copyExplanation
        }
    }

    private var appIcon: some View {
        Group {
            if let icon = CowchatAppDelegate.applicationIcon() {
                Image(nsImage: icon)
                    .resizable()
                    .scaledToFit()
            } else {
                Image(systemName: "bubble.left.and.bubble.right.fill")
                    .resizable()
                    .scaledToFit()
                    .padding(20)
                    .foregroundStyle(SemanticColor.iconPrimary)
            }
        }
        .frame(width: 88, height: 88)
        .background(
            SemanticColor.surface600,
            in: RoundedRectangle(cornerRadius: 22, style: .continuous)
        )
        .shadow(
            color: SemanticColor.surfaceGlassBorderShadow,
            radius: 18,
            y: 8
        )
        .accessibilityHidden(true)
    }

    private var promptCard: some View {
        HStack(alignment: .bottom, spacing: 14) {
            Text(CowchatOnboarding.collaborationPrompt)
                .textSelection(.enabled)
                .gallopText(.bodyMStrong, color: SemanticColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Button("Copy") {
                isCopyExplanationPresented = true
            }
            .buttonStyle(.plain)
            .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
            .padding(.horizontal, 18)
            .frame(height: 38)
            .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
            .macAccessibleAction(label: "Copy collaboration prompt") {
                isCopyExplanationPresented = true
            }
        }
        .padding(18)
        .frame(maxWidth: 620)
        .background(
            SemanticColor.surface600,
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
    }

    private var copyExplanation: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Copy and paste it into your AI chatbot")
                .gallopText(.h5, color: SemanticColor.textPrimary)
            Text("This copies a setup prompt to your clipboard. It does not send anything; you choose the chatbot and when to paste it.")
                .gallopText(.bodyM, color: SemanticColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack(spacing: 10) {
                Button("Skip") {
                    isCopyExplanationPresented = false
                    onComplete()
                }
                .buttonStyle(.plain)
                .gallopText(.bodyMStrong, color: SemanticColor.buttonSecondaryTextDefault)
                .frame(maxWidth: .infinity)
                .frame(height: 40)
                .background(
                    SemanticColor.buttonSecondaryDefault,
                    in: Capsule()
                )
                .macAccessibleAction(label: "Skip copying and continue") {
                    isCopyExplanationPresented = false
                    onComplete()
                }

                Button("Copy") { copyPrompt() }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                    .frame(maxWidth: .infinity)
                    .frame(height: 40)
                    .background(
                        SemanticColor.buttonPrimaryDefault,
                        in: Capsule()
                    )
                    .macAccessibleAction(label: "Copy prompt to clipboard", action: copyPrompt)
            }
        }
        .padding(22)
        .frame(width: 380, height: 220)
        .background(SemanticColor.surface600)
        .accessibilityAddTraits(.isModal)
    }

    private func copyPrompt() {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(CowchatOnboarding.collaborationPrompt, forType: .string)
        hasCopiedPrompt = true
        isCopyExplanationPresented = false
    }
}
