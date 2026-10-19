# Mac app updates

Cowchat uses Sparkle 2.10.0 for in-app updates. Choose **Cowchat → Check for Updates…**, or open **Preferences → Updates** to change automatic checks and automatic download/install. Sparkle asks before enabling background checks. Installation waits for quitting or an explicit relaunch; it does not interrupt a working session.

The app bundle includes its local server. The existing application termination path waits for an app-owned helper to stop before an update replaces the bundle. Independently started servers and standalone CLI installations are not replaced or stopped. Update those with `brew upgrade --formula cowboyinc/tap/cowchat` or your existing package manager.

Homebrew cask installs also support Sparkle. The cask declares `auto_updates true`; Homebrew tracks its installed version separately. Use the in-app updater for normal app updates. Cowchat 0.9.9 and earlier need one manual upgrade to obtain Sparkle.

## Release signing

The app embeds `SUPublicEDKey` and a fixed HTTPS feed URL at `https://github.com/cowboyinc/cowchat/releases/latest/download/appcast.xml`. It requires a signed feed and validates signed archive bytes before extraction. Release-note JavaScript is disabled. The Developer ID signature is retained inside the notarized DMG.

The `SPARKLE_PRIVATE_KEY` repository Actions secret contains a base64-encoded 32-byte Ed25519 seed. Its public key must match the app before release signing proceeds. Keep a protected offline copy of the seed outside the repository. Never put it in a log, ordinary build artifact, app bundle or website. Signing scripts use an explicit temporary key file and never fall back to a developer's Keychain.

The workflow signs the final DMG only after notarization and stapling, then generates and signs the appcast. The app build number, display version, release tag and appcast version must match. All assets are uploaded to a draft before the release becomes public. A retry refuses to replace already-published bytes with different assets.

For a new signing key, provision the Actions secret before the first release. Keep the embedded public key stable for subsequent releases. Rotate keys only using Sparkle's documented signed-update migration procedure. A lost private key is not a reason to disable validation.

## Verification

Run the native Swift tests and universal release build, `apps/CowchatMac/test-dmg-packaging.sh`, the Sparkle framework packaging check, and the appcast/publication tests. The signing tests use temporary test keys and reject modified archives, modified feeds and mismatched signing keys. Existing app termination and supervisor tests cover owned-child shutdown and external-server ownership.

Before promoting a release, verify the packaged framework and both app/helper slices, nested signatures, notarization, and an actual update/install/relaunch through a signed feed. A successful build or signature test alone does not prove installation.
