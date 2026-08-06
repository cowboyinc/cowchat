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

    /// `MessageFeedRow.body` asks for this once per row on every evaluation, so
    /// an uncached LaunchServices round trip lands on every message in the feed
    /// each pass. Memoized for the process lifetime: an app installed while
    /// Cowchat is running is picked up on the next launch.
    private static var applicationURLCache: [String: URL?] = [:]

    @MainActor
    static func applicationURL(for app: ResolvedApp) -> URL? {
        if let cached = applicationURLCache[app.bundleID] { return cached }
        let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: app.bundleID)
        applicationURLCache[app.bundleID] = url
        return url
    }

    @MainActor
    static func open(_ app: ResolvedApp) {
        guard let url = applicationURL(for: app) else { return }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        NSWorkspace.shared.openApplication(at: url, configuration: configuration)
    }
}
