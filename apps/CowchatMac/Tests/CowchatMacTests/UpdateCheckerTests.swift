import XCTest

@testable import CowchatMac

final class UpdateCheckerTests: XCTestCase {
    private func releaseJSON(
        tag: String, url: String = "https://github.com/cowboyinc/cowchat/releases/tag/v9.9.9",
        draft: Bool = false, prerelease: Bool = false
    ) -> Data {
        let payload: [String: Any] = [
            "tag_name": tag,
            "html_url": url,
            "draft": draft,
            "prerelease": prerelease,
            "assets": [],
        ]
        return try! JSONSerialization.data(withJSONObject: payload)
    }

    func testParseVersionAcceptsTaggedAndBareForms() {
        XCTAssertEqual(UpdateChecker.parseVersion("v0.7.0"), [0, 7, 0])
        XCTAssertEqual(UpdateChecker.parseVersion("0.7.0"), [0, 7, 0])
        XCTAssertEqual(UpdateChecker.parseVersion("1.10"), [1, 10])
        XCTAssertNil(UpdateChecker.parseVersion(""))
        XCTAssertNil(UpdateChecker.parseVersion("v"))
        XCTAssertNil(UpdateChecker.parseVersion("0.7.0-rc1"))
        XCTAssertNil(UpdateChecker.parseVersion("0..7"))
        XCTAssertNil(UpdateChecker.parseVersion("release"))
    }

    func testVersionComparisonPadsMissingComponents() {
        XCTAssertTrue(UpdateChecker.isVersion([0, 7, 1], newerThan: [0, 7, 0]))
        XCTAssertTrue(UpdateChecker.isVersion([1, 0], newerThan: [0, 9, 9]))
        XCTAssertTrue(UpdateChecker.isVersion([0, 7, 0, 1], newerThan: [0, 7]))
        XCTAssertFalse(UpdateChecker.isVersion([0, 7], newerThan: [0, 7, 0]))
        XCTAssertFalse(UpdateChecker.isVersion([0, 7, 0], newerThan: [0, 7, 0]))
        XCTAssertFalse(UpdateChecker.isVersion([0, 6, 2], newerThan: [0, 7, 0]))
    }

    func testDecodeLatestReleaseReadsTagAndPage() {
        let release = UpdateChecker.decodeLatestRelease(
            from: releaseJSON(tag: "v0.8.0"))
        XCTAssertEqual(release?.displayVersion, "0.8.0")
        XCTAssertEqual(release?.version, [0, 8, 0])
        XCTAssertEqual(
            release?.pageURL.absoluteString,
            "https://github.com/cowboyinc/cowchat/releases/tag/v9.9.9")
    }

    func testDecodeLatestReleaseRejectsDraftPrereleaseAndGarbage() {
        XCTAssertNil(
            UpdateChecker.decodeLatestRelease(from: releaseJSON(tag: "v0.8.0", draft: true)))
        XCTAssertNil(
            UpdateChecker.decodeLatestRelease(from: releaseJSON(tag: "v0.8.0", prerelease: true)))
        XCTAssertNil(UpdateChecker.decodeLatestRelease(from: releaseJSON(tag: "nightly")))
        XCTAssertNil(UpdateChecker.decodeLatestRelease(from: Data("not json".utf8)))
    }

    func testOfferedReleaseRequiresStrictlyNewerVersion() {
        XCTAssertNotNil(
            UpdateChecker.offeredRelease(
                from: releaseJSON(tag: "v0.8.0"), currentVersion: "0.7.0", skippedVersion: nil))
        XCTAssertNil(
            UpdateChecker.offeredRelease(
                from: releaseJSON(tag: "v0.7.0"), currentVersion: "0.7.0", skippedVersion: nil))
        XCTAssertNil(
            UpdateChecker.offeredRelease(
                from: releaseJSON(tag: "v0.6.2"), currentVersion: "0.7.0", skippedVersion: nil))
        // Unknown current version (e.g. debug builds without a bundle) never prompts.
        XCTAssertNil(
            UpdateChecker.offeredRelease(
                from: releaseJSON(tag: "v0.8.0"), currentVersion: nil, skippedVersion: nil))
    }

    func testOfferedReleaseHonorsSkipUntilSomethingNewer() {
        XCTAssertNil(
            UpdateChecker.offeredRelease(
                from: releaseJSON(tag: "v0.8.0"), currentVersion: "0.7.0",
                skippedVersion: "0.8.0"))
        XCTAssertNotNil(
            UpdateChecker.offeredRelease(
                from: releaseJSON(tag: "v0.8.1"), currentVersion: "0.7.0",
                skippedVersion: "0.8.0"))
    }

    func testDetectBrewManagedInstallRequiresCaskroomDirectory() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("UpdateCheckerTests-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }

        let caskDir = root.appendingPathComponent("Caskroom/cowchat")
        XCTAssertFalse(
            UpdateChecker.detectBrewManagedInstall(caskroomPaths: [caskDir.path]))

        try FileManager.default.createDirectory(
            at: caskDir, withIntermediateDirectories: true)
        XCTAssertTrue(
            UpdateChecker.detectBrewManagedInstall(caskroomPaths: [caskDir.path]))

        // A plain file at the path does not count as a brew install.
        let filePath = root.appendingPathComponent("Caskroom/cowchat-file")
        FileManager.default.createFile(atPath: filePath.path, contents: Data())
        XCTAssertFalse(
            UpdateChecker.detectBrewManagedInstall(caskroomPaths: [filePath.path]))
    }

    @MainActor
    func testBrewUpgradeCommandTargetsTheCask() {
        XCTAssertEqual(UpdateChecker.brewUpgradeCommand, "brew upgrade --cask cowchat")
    }

    @MainActor
    func testSkipAvailableReleasePersistsSkippedVersion() {
        let suiteName = "UpdateCheckerTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let checker = UpdateChecker(defaults: defaults, currentVersion: "0.7.0")
        checker.availableRelease = UpdateChecker.decodeLatestRelease(
            from: releaseJSON(tag: "v0.8.0"))
        XCTAssertNotNil(checker.availableRelease)

        checker.skipAvailableRelease()
        XCTAssertNil(checker.availableRelease)
        XCTAssertEqual(defaults.string(forKey: UpdateChecker.skippedVersionKey), "0.8.0")
    }
}
