import AppKit
import SwiftUI

/// The connect-prompt empty state an agentless room renders in place of the
/// old RoomSetupView takeover (spec §3). Dumb view: everything injected.
struct RoomConnectStateView: View {
    let roomName: String
    let prompt: String
    let variant: RoomPaneState.ConnectVariant

    @State private var hasCopiedPrompt = false
    @State private var showsSlowJoinHint = false
    @State private var isTroubleshootingPresented = false

    var body: some View {
        VStack(spacing: 22) {
            HStack(spacing: 14) {
                Image(systemName: "list.bullet.rectangle")
                Image(systemName: "arrow.right")
                Image(systemName: "sparkles")
            }
            .font(.system(size: 17, weight: .semibold))
            .foregroundStyle(SemanticColor.iconPrimary)

            Text("Paste this prompt into an AI chatbot")
                .gallopText(.h5, color: SemanticColor.textPrimary)

            HStack(alignment: .bottom, spacing: 14) {
                Text(prompt)
                    .textSelection(.enabled)
                    .gallopText(.bodyM, color: SemanticColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)

                Button(hasCopiedPrompt ? "Copied" : "Copy") { copyPrompt() }
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                    .padding(.horizontal, 18)
                    .frame(height: 38)
                    .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                    .macAccessibleAction(label: "Copy connect prompt", action: copyPrompt)
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

            statusLine

            if showsSlowJoinHint, variant == .connected {
                Text("First join can take a few minutes while your agent installs cowchat.")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
            }

            Button("Not connecting?") { isTroubleshootingPresented = true }
                .buttonStyle(.plain)
                .gallopText(.caption, color: SemanticColor.textTertiary)
                .macAccessibleAction(label: "Connection troubleshooting") {
                    isTroubleshootingPresented = true
                }
                .popover(isPresented: $isTroubleshootingPresented) {
                    ConnectTroubleshootingView()
                        .frame(width: 360)
                        .padding(18)
                }
        }
        .padding(28)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: "\(roomName)-\(variant == .connected)") {
            showsSlowJoinHint = false
            guard variant == .connected else { return }
            try? await Task.sleep(nanoseconds: 60_000_000_000)
            if !Task.isCancelled { showsSlowJoinHint = true }
        }
    }

    private var statusLine: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(variant == .connected ? SemanticColor.success : SemanticColor.warning)
                .frame(width: 7, height: 7)
                .modifier(PulsingDot(active: variant == .connected))
            Text(statusText)
                .gallopText(.caption, color: SemanticColor.textTertiary)
        }
    }

    private var statusText: String {
        switch variant {
        case .connected: return "Connected — waiting for your first agent…"
        case .connecting: return "Connecting…"
        case .offline: return "Offline — reconnect before pasting."
        }
    }

    private func copyPrompt() {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(prompt, forType: .string)
        hasCopiedPrompt = true
    }
}

/// Subtle opacity pulse for the waiting dot; no motion for static variants.
private struct PulsingDot: ViewModifier {
    let active: Bool
    @State private var dimmed = false

    func body(content: Content) -> some View {
        content
            .opacity(active && dimmed ? 0.35 : 1)
            .animation(
                active ? .easeInOut(duration: 1.1).repeatForever(autoreverses: true) : .default,
                value: dimmed
            )
            .onAppear { if active { dimmed = true } }
    }
}

/// The three real first-run failure modes, one line each (spec §5). Shared by
/// the connect state's popover and EmptyChatView's connection-failed variant.
struct ConnectTroubleshootingView: View {
    @State private var hasCopiedBrewCommand = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            item(
                title: "Older server on port 9229",
                body: "If you've installed cowchat with Homebrew before, an older server may be answering. Run the command below, then quit and reopen Cowchat."
            )
            HStack(spacing: 10) {
                Text("brew upgrade cowchat")
                    .gallopText(.code, color: SemanticColor.textSecondary)
                    .textSelection(.enabled)
                Button(hasCopiedBrewCommand ? "Copied" : "Copy") {
                    let pasteboard = NSPasteboard.general
                    pasteboard.clearContents()
                    pasteboard.setString("brew upgrade cowchat", forType: .string)
                    hasCopiedBrewCommand = true
                }
                .buttonStyle(.plain)
                .gallopText(.bodySStrong, color: SemanticColor.buttonSecondaryTextDefault)
                .macAccessibleAction(label: "Copy brew upgrade command") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString("brew upgrade cowchat", forType: .string)
                    hasCopiedBrewCommand = true
                }
            }
            item(
                title: "Private rooms need the key",
                body: "Agents must connect with the key at ~/.cowchat/auth.key to see a private room — or make the room public."
            )
            item(
                title: "Your agent needs internet",
                body: "Your agent fetches the Cowchat skill from cowchat.cowboy.inc — it needs internet even though Cowchat itself is local."
            )
        }
    }

    private func item(title: String, body: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
            Text(body)
                .gallopText(.caption, color: SemanticColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
