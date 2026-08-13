import Foundation

/// Lightweight launch-time update check. Asks GitHub for the latest release,
/// compares it against the running app's version, and exposes the newer
/// release for a one-shot prompt. No self-updating — the prompt links to the
/// release page and the user decides.
@MainActor
final class UpdateChecker: ObservableObject {
    struct Release: Equatable {
        let displayVersion: String
        let version: [Int]
        let pageURL: URL
    }

    static let latestReleaseURL = URL(
        string: "https://api.github.com/repos/cowboyinc/cowchat/releases/latest")!
    static let skippedVersionKey = "cowchat.update.skipped-version"

    /// Non-nil when a newer, non-skipped release is available; drives the prompt.
    @Published var availableRelease: Release?

    private let defaults: UserDefaults
    private let currentVersion: String?

    init(
        defaults: UserDefaults = .standard,
        currentVersion: String? = Bundle.main.object(
            forInfoDictionaryKey: "CFBundleShortVersionString") as? String
    ) {
        self.defaults = defaults
        self.currentVersion = currentVersion
    }

    /// Best-effort: any network or parse failure means no prompt.
    func check() async {
        guard availableRelease == nil else { return }
        var request = URLRequest(url: Self.latestReleaseURL, timeoutInterval: 10)
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
            (response as? HTTPURLResponse)?.statusCode == 200
        else { return }
        availableRelease = Self.offeredRelease(
            from: data,
            currentVersion: currentVersion,
            skippedVersion: defaults.string(forKey: Self.skippedVersionKey)
        )
    }

    func skipAvailableRelease() {
        guard let release = availableRelease else { return }
        defaults.set(release.displayVersion, forKey: Self.skippedVersionKey)
        availableRelease = nil
    }

    nonisolated static func offeredRelease(
        from data: Data, currentVersion: String?, skippedVersion: String?
    ) -> Release? {
        guard let release = decodeLatestRelease(from: data),
            let current = parseVersion(currentVersion ?? "")
        else { return nil }
        guard isVersion(release.version, newerThan: current) else { return nil }
        if let skipped = parseVersion(skippedVersion ?? ""),
            !isVersion(release.version, newerThan: skipped)
        {
            return nil
        }
        return release
    }

    nonisolated static func decodeLatestRelease(from data: Data) -> Release? {
        struct Payload: Decodable {
            let tag_name: String
            let html_url: String
            let draft: Bool?
            let prerelease: Bool?
        }
        guard let payload = try? JSONDecoder().decode(Payload.self, from: data),
            payload.draft != true, payload.prerelease != true,
            let version = parseVersion(payload.tag_name),
            let url = URL(string: payload.html_url)
        else { return nil }
        let display =
            payload.tag_name.hasPrefix("v")
            ? String(payload.tag_name.dropFirst()) : payload.tag_name
        return Release(displayVersion: display, version: version, pageURL: url)
    }

    /// "v0.7.0" / "0.7.0" → [0, 7, 0]. Nil for anything else.
    nonisolated static func parseVersion(_ string: String) -> [Int]? {
        let trimmed = string.hasPrefix("v") ? String(string.dropFirst()) : string
        guard !trimmed.isEmpty else { return nil }
        let parts = trimmed.split(separator: ".", omittingEmptySubsequences: false)
            .map { Int($0) }
        guard !parts.contains(nil) else { return nil }
        return parts.compactMap { $0 }
    }

    nonisolated static func isVersion(_ lhs: [Int], newerThan rhs: [Int]) -> Bool {
        for index in 0..<max(lhs.count, rhs.count) {
            let left = index < lhs.count ? lhs[index] : 0
            let right = index < rhs.count ? rhs[index] : 0
            if left != right { return left > right }
        }
        return false
    }
}
