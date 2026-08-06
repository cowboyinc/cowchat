import AppKit

/// Maps agent display names to installed companion apps. Single source of
/// truth for both the avatar app-icon lookup and the "Open in …" actions.
enum AgentAppResolver {
    struct ResolvedApp: Equatable {
        let displayName: String
        let bundleID: String
    }

    static func resolvedApp(forAgentNamed name: String) -> ResolvedApp? {
        let normalized = name.lowercased()
        if normalized.contains("claude") {
            return ResolvedApp(displayName: "Claude", bundleID: "com.anthropic.claudefordesktop")
        }
        if normalized.contains("codex") {
            return ResolvedApp(displayName: "Codex", bundleID: "com.openai.codex")
        }
        if normalized.contains("chatgpt") || normalized.contains("openai") {
            return ResolvedApp(displayName: "ChatGPT", bundleID: "com.openai.chat")
        }
        return nil
    }

    static func applicationURL(for app: ResolvedApp) -> URL? {
        NSWorkspace.shared.urlForApplication(withBundleIdentifier: app.bundleID)
    }

    @MainActor
    static func open(_ app: ResolvedApp) {
        guard let url = applicationURL(for: app) else { return }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        NSWorkspace.shared.openApplication(at: url, configuration: configuration)
    }
}
