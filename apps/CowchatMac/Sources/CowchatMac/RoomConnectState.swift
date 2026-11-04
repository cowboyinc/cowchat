import AppKit
import SwiftUI

/// In-feed "what is this agent doing" pill, rendered under that agent's
/// latest message. Wraps freely — pulse text is prose, not a status code.
/// Lives inside the eager feed VStack on purpose: with lazy layout gone,
/// pills changing the feed's height converges in a single pass.
struct ThinkingPulsePill: View {
    let pulse: AgentThinkingPulse

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Circle()
                .fill(SemanticColor.buttonPrimaryDefault)
                .frame(width: 7, height: 7)
                .modifier(PulsingDot(active: true))
                .padding(.top, 5)
            Text(ThinkingPulsePresentation.label(for: pulse))
                .gallopText(.bodyS, color: SemanticColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(
            SemanticColor.surface600,
            in: RoundedRectangle(cornerRadius: 12, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(SemanticColor.borderDefault, lineWidth: 1)
        }
        .frame(maxWidth: 640, alignment: .leading)
    }
}

/// Subtle opacity pulse for the waiting dot; no motion for static variants.
struct PulsingDot: ViewModifier {
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
                Button {
                    let pasteboard = NSPasteboard.general
                    pasteboard.clearContents()
                    pasteboard.setString("brew upgrade cowchat", forType: .string)
                    hasCopiedBrewCommand = true
                } label: {
                    // Same width-reservation as the prompt card's Copy button.
                    ZStack {
                        Text("Copied").hidden()
                        Text(hasCopiedBrewCommand ? "Copied" : "Copy")
                    }
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
