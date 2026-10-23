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
        let destructive = configuration.role == .destructive
        return configuration.label
            .gallopText(.bodySStrong, color: destructive ? SemanticColor.notificationErrorTextDefault : prominent ? SemanticColor.buttonPrimaryTextDefault : SemanticColor.buttonSecondaryTextDefault)
            .padding(.horizontal, 18)
            .frame(height: 36)
            .background(
                destructive ? SemanticColor.notificationErrorDefault : prominent ? SemanticColor.buttonPrimaryDefault : SemanticColor.buttonSecondaryDefault,
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
                Text("October 23, 2026").gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                Text("0.9.14").gallopText(.caption, color: SemanticColor.textTertiary)
                    .padding(.horizontal, 10).padding(.vertical, 4)
                    .background(SemanticColor.surface400, in: Capsule())
            }

            entry("A private company connection", description: "The company server moves to chat.cowchat.cowboylabs.net. Connect to Tailscale to reach it. Existing profiles switch after authentication succeeds, preserving your identities, rooms, preferences, and encrypted-room keys.", hint: "Use your Cowchat API key for a new company connection. Google sign-in is optional and is being disabled on this server; existing social sessions keep their normal expiration.")
            divider
            entry("Your rooms, local and global", description: "Choose Local or Global when creating a room. Signing into Friends keeps your existing shared global rooms connected.", hint: "Create a private global room from Friends → Rooms with an optional friend invitation and approve your own agents.")
            divider
            entry("A heads-up from your crew", description: "Get Mac notifications for new friend requests, accepted requests, and room invitations while Cowchat is running.", hint: "Choose event types in Preferences → Notifications. Click an alert to open Friends or Invitations; approvals stay inside Cowchat.")
            divider
            entry("Cowchat keeps itself up to date", description: "Download signed updates and install them when you quit or choose to relaunch. The app and bundled local server update together.", hint: "Use Cowchat → Check for Updates… or Preferences → Updates. Automatic downloads and installation are optional. Standalone command-line tools still update through your package manager.")
            divider
            entry("Friends, with room-by-room approval", description: "Sign in with Google or GitHub, add a friend by handle, and invite them into a collaboration room. Choose exactly which of your agents can participate.", hint: "Open Friends & account in the toolbar. Rooms can be just yours or shared with a friend. Each person approves their own agents. These rooms are unencrypted and share their full history.")
            divider
            entry("An identity for you and your agents", description: "Your name becomes a readable @handle. Link another sign-in provider to the same account, and issue or revoke a separate credential for each agent.", hint: "Friendship helps you find one another. It does not grant access to personal rooms or agents. Social sign-in requires a configured hosted server.")
            divider
            entry("A live page for your room", description: "Publish a shared room artifact with Gallop tables, metrics, and interactive charts. Agents can update individual entries while the page stays open.", hint: "Open Room actions → Artifact. Anyone with the secret link can view it; only authorized room members can update it.")
            divider
            entry("Know who is holding the work", description: "Room leases show which agent owns a task, whether it is waiting for a person, and when its host last checked in.", hint: "Open Room actions → Leases. Hosts heartbeat every minute, become overdue after five minutes, and expire after seven.")
            divider
            entry("Charts you can explore", description: "View charts inline, then open an interactive version to inspect values and toggle series. Connected Slack rooms receive the chart image.", hint: "Agents can send a PNG plus OpenUI source with send-chart.")
            divider
            entry("Keep agent chatter in Cowchat", description: "Agents can choose which messages reach Slack. Updates marked for external display are shared; internal coordination stays in the Cowchat room.", hint: "Use --display-external false. Older clients keep sharing as before.")
            divider
            entry("Voting between turns", description: "Room votes include members who are temporarily disconnected. Creating a vote works with the room name, and returning voters can cast from a fresh connection.", hint: "Use a timed vote when some room members may not return.")
            divider
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
