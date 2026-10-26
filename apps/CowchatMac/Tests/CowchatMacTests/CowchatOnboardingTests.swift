import XCTest
@testable import CowchatMac

final class CowchatOnboardingTests: XCTestCase {
    func testExistingUserIsNotInterruptedByUpgrade() throws {
        let (defaults, suiteName) = freshDefaults()
        defer { defaults.removePersistentDomain(forName: suiteName) }

        CowchatOnboarding.migrateExistingUser(defaults: defaults, hadExistingAgentID: true)

        XCTAssertEqual(
            defaults.integer(forKey: CowchatOnboarding.completedVersionKey),
            CowchatOnboarding.currentVersion
        )
        XCTAssertTrue(defaults.bool(forKey: CowchatOnboarding.migrationAttemptedKey))
    }

    func testFreshUserStillReceivesOnboarding() throws {
        let (defaults, suiteName) = freshDefaults()
        defer { defaults.removePersistentDomain(forName: suiteName) }

        CowchatOnboarding.migrateExistingUser(defaults: defaults, hadExistingAgentID: false)

        XCTAssertNil(defaults.object(forKey: CowchatOnboarding.completedVersionKey))
        XCTAssertTrue(defaults.bool(forKey: CowchatOnboarding.migrationAttemptedKey))
    }

    func testFreshUserWhoQuitsBeforeCompletionIsNotMigratedOnSecondLaunch() throws {
        let (defaults, suiteName) = freshDefaults()
        defer { defaults.removePersistentDomain(forName: suiteName) }

        CowchatOnboarding.migrateExistingUser(defaults: defaults, hadExistingAgentID: false)
        defaults.set("cowchat-mac-generated", forKey: "CowchatMac.agentID")
        CowchatOnboarding.migrateExistingUser(defaults: defaults, hadExistingAgentID: true)

        XCTAssertNil(defaults.object(forKey: CowchatOnboarding.completedVersionKey))
    }

    func testExplicitReplayChoiceIsNotOverridden() throws {
        let (defaults, suiteName) = freshDefaults()
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set(0, forKey: CowchatOnboarding.completedVersionKey)

        CowchatOnboarding.migrateExistingUser(defaults: defaults, hadExistingAgentID: true)

        XCTAssertEqual(defaults.integer(forKey: CowchatOnboarding.completedVersionKey), 0)
    }

    func testStarterNamePreviewsPreserveUnicodeAndBoundSuffixes() {
        XCTAssertEqual(SocialStarterNames.bot("Chad DePue"), "Chad_DePue_bot")
        XCTAssertEqual(SocialStarterNames.home("  Renée • 李  "), "Renée_李_Home")
        XCTAssertEqual(SocialStarterNames.bot("🐄"), "Cowboy_bot")
        XCTAssertEqual(SocialStarterNames.home(String(repeating: "é", count: 100)), String(repeating: "é", count: 70) + "_Home")
        XCTAssertEqual(WizardStep.login.number, 1)
        XCTAssertEqual(WizardStep.ready.number, 4)
    }

    private func freshDefaults() -> (UserDefaults, String) {
        let suiteName = "CowchatOnboardingTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        return (defaults, suiteName)
    }
}
