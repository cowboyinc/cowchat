import SwiftUI

/// Shared sheet chrome keeps integration and release notes consistent with Settings.
struct CowchatSheetHeader: View {
    let title: String
    let subtitle: String
    let close: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 20) {
            VStack(alignment: .leading, spacing: 8) {
                Text(title).gallopText(.h3, color: SemanticColor.textPrimary)
                Text(subtitle).gallopText(.bodyM, color: SemanticColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.system(size: 15, weight: .medium))
                    .foregroundStyle(SemanticColor.iconSecondary)
                    .frame(width: 32, height: 32)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.cancelAction)
            .accessibilityLabel("Close \(title)")
            .help("Close")
        }
        .padding(28)
    }
}

struct CowchatPillButtonStyle: ButtonStyle {
    var prominent = false
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .gallopText(.bodySStrong, color: prominent ? SemanticColor.buttonPrimaryTextDefault : SemanticColor.buttonSecondaryTextDefault)
            .padding(.horizontal, 18)
            .frame(height: 36)
            .background(
                prominent ? SemanticColor.buttonPrimaryDefault : SemanticColor.buttonSecondaryDefault,
                in: Capsule()
            )
            .overlay(Capsule().stroke(SemanticColor.borderDefault, lineWidth: prominent ? 0 : 0.5))
            .opacity(!isEnabled ? 0.45 : configuration.isPressed ? 0.75 : 1)
    }
}

struct WhatsNewView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            CowchatSheetHeader(title: "What’s new", subtitle: "The latest improvements to Cowchat.") { dismiss() }
            Rectangle().fill(SemanticColor.borderDefault).frame(height: 0.5)
            ScrollView {
                WhatsNewContent().padding(28)
            }
        }
        .frame(width: 720, height: 640)
        .background(SemanticColor.surface600)
    }
}

struct WhatsNewContent: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 30) {
            HStack(spacing: 10) {
                Text("October 16, 2026").gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                Text("0.9.2").gallopText(.caption, color: SemanticColor.textTertiary)
                    .padding(.horizontal, 10).padding(.vertical, 4)
                    .background(SemanticColor.surface400, in: Capsule())
            }

            VStack(alignment: .leading, spacing: 18) {
                SlackBridgeIllustration()
                entry("Your global rooms, now in Slack", description: "Connect a global Cowchat room to a Slack channel. New messages flow both ways, so people and agents can stay in the same conversation.", hint: "Open a global room → Room actions → Slack.")
            }
            divider
            entry("Bring room bots into the conversation", description: "Use a bot’s @handle in the connected Slack channel to address it in Cowchat. Handles resolve within that room.", hint: "Your agents still need to be running or have a wake connection configured.")
            divider
            entry("A connection you control", description: "Choose exactly which global rooms connect to Slack. The server keeps relaying while your Mac is offline. Pause or disconnect a room at any time.", hint: "Only new messages are shared. Earlier room history stays in Cowchat.")
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var divider: some View {
        Rectangle().fill(SemanticColor.borderDefault).frame(height: 0.5)
    }

    private func entry(_ title: String, description: String, hint: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).gallopText(.h5, color: SemanticColor.textPrimary)
            Text(description).gallopText(.bodyM, color: SemanticColor.textSecondary)
            Text(hint).gallopText(.bodyS, color: SemanticColor.textTertiary)
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

struct SlackBridgeIllustration: View {
    var body: some View {
        HStack(spacing: 18) {
            endpoint("number", title: "Slack", detail: "Your channel")
            Image(systemName: "arrow.left.arrow.right")
                .font(.system(size: 18, weight: .medium))
                .foregroundStyle(SemanticColor.iconTertiary)
            endpoint("bubble.left.and.bubble.right", title: "Cowchat", detail: "Your global room")
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 24)
        .background(SemanticColor.surface400, in: RoundedRectangle(cornerRadius: 16))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Messages flow both ways between your Slack channel and global Cowchat room")
    }

    private func endpoint(_ icon: String, title: String, detail: String) -> some View {
        VStack(spacing: 7) {
            Image(systemName: icon)
                .font(.system(size: 23, weight: .medium))
                .foregroundStyle(SemanticColor.textPrimary)
                .frame(width: 48, height: 48)
                .background(SemanticColor.surface700, in: RoundedRectangle(cornerRadius: 13))
            Text(title).gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
            Text(detail).gallopText(.caption, color: SemanticColor.textTertiary)
        }
        .frame(width: 160)
    }
}
