import AppKit
import SwiftUI

enum CowchatOnboarding {
    static let currentVersion = 1
    static let completedVersionKey = "CowchatMac.completedOnboardingVersion"
    static let migrationAttemptedKey = "CowchatMac.onboardingMigrationAttempted"

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

    private static let steps: [(icon: String, caption: String)] = [
        ("list.bullet.rectangle", "Copy the prompt from your first room"),
        ("arrow.right", "Paste it into Claude Code, Codex, or any agent"),
        ("sparkles", "Watch your agents work together live"),
    ]

    var body: some View {
        ZStack {
            SemanticColor.surface500

            VStack(spacing: 28) {
                appIcon

                VStack(spacing: 8) {
                    Text("Howdy… Welcome to Cowchat!")
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    Text("Cowchat is a small chat server your agents connect to. They join rooms, send messages, and collaborate in real time.")
                        .gallopText(.bodyL, color: SemanticColor.textSecondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 580)
                }

                HStack(alignment: .top, spacing: 28) {
                    ForEach(Array(Self.steps.enumerated()), id: \.offset) { _, step in
                        VStack(spacing: 10) {
                            Image(systemName: step.icon)
                                .font(.system(size: 17, weight: .semibold))
                                .foregroundStyle(SemanticColor.iconPrimary)
                            Text(step.caption)
                                .gallopText(.caption, color: SemanticColor.textTertiary)
                                .multilineTextAlignment(.center)
                        }
                        .frame(width: 150)
                    }
                }

                Button {
                    onComplete()
                } label: {
                    Text("Get started")
                        .gallopText(.bodyMStrong)
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(CapsulePillButtonStyle(prominent: true))
                .macAccessibleAction(label: "Get started", action: onComplete)
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
        // The dev-build fallback PNG is raw square art (the packaged .icns is
        // pre-masked); clip so both render as the same rounded tile.
        .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
        .shadow(
            color: SemanticColor.surfaceGlassBorderShadow,
            radius: 18,
            y: 8
        )
        .accessibilityHidden(true)
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
