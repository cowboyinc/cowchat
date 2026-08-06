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

                Button {
                    onComplete()
                } label: {
                    Text(hasCopiedPrompt ? "Continue" : "Skip for now")
                        .gallopText(.bodyMStrong)
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(CapsulePillButtonStyle(prominent: true))
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

            Button {
                isCopyExplanationPresented = true
            } label: {
                Text("Copy")
                    .gallopText(.bodyMStrong)
            }
            .buttonStyle(CapsulePillButtonStyle(prominent: true))
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
                Button {
                    isCopyExplanationPresented = false
                    onComplete()
                } label: {
                    Text("Skip")
                        .gallopText(.bodyMStrong)
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(CapsulePillButtonStyle(prominent: false))
                .macAccessibleAction(label: "Skip copying and continue") {
                    isCopyExplanationPresented = false
                    onComplete()
                }

                Button {
                    copyPrompt()
                } label: {
                    Text("Copy")
                        .gallopText(.bodyMStrong)
                        .frame(maxWidth: .infinity)
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(CapsulePillButtonStyle(prominent: true))
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

/// Capsule ramp copied from the cowboy `AuthPillButtonStyle` shape (12pt
/// vertical / 20pt horizontal padding, `prominent` selects the primary vs.
/// secondary token family). Deliberately trimmed to default/pressed/disabled
/// — the reference style's hover and focus-ring states need per-callsite
/// `@State`/`@FocusState` wiring at every one of onboarding's four call
/// sites; simplification authorized for Task 14 (disclosed in the report).
private struct CapsulePillButtonStyle: ButtonStyle {
    let prominent: Bool
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .padding(.vertical, 12)
            .padding(.horizontal, 20)
            .foregroundStyle(labelColor(isPressed: configuration.isPressed))
            .background(fillColor(isPressed: configuration.isPressed), in: Capsule())
            .contentShape(Capsule())
            .opacity(isEnabled ? 1 : 0.5)
    }

    private func fillColor(isPressed: Bool) -> Color {
        if prominent {
            return isPressed ? SemanticColor.buttonPrimaryPressed : SemanticColor.buttonPrimaryDefault
        }
        return isPressed ? SemanticColor.buttonSecondaryPressed : SemanticColor.buttonSecondaryDefault
    }

    private func labelColor(isPressed: Bool) -> Color {
        if prominent {
            return isPressed ? SemanticColor.buttonPrimaryTextPressed : SemanticColor.buttonPrimaryTextDefault
        }
        return isPressed ? SemanticColor.buttonSecondaryTextPressed : SemanticColor.buttonSecondaryTextDefault
    }
}
