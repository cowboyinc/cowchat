# CowchatiOS

The iOS app for Cowchat, for iPhone and iPad. It shares its connection, model,
account and design code with the Mac app through the `../CowchatKit` package.

Requirements: Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen)
(`brew install xcodegen`).

## Generate and build

```bash
xcodegen generate
xcodebuild build -project CowchatiOS.xcodeproj -scheme CowchatiOS \
  -destination 'platform=iOS Simulator,name=iPhone 17' -derivedDataPath build-sim
```

The `.xcodeproj` is generated and gitignored. Edit `project.yml`, not the project.

## Gallop

The app compiles the Mac app's Gallop design-system files and bundles its Season fonts. Icons come from the
CowchatKit package, which ships them as an asset catalog iOS can load (see `apps/CowchatKit/Scripts/build-gallop-icons.py`).

## Run in the Simulator

Open `CowchatiOS.xcodeproj` in Xcode, pick an iPhone simulator, and press Run.

## Run on a phone

Connect an iPhone, unlock it, and trust this Mac. Then:

```bash
scripts/run-on-device.sh
```

It builds with automatic signing, installs, and launches the app.
The first run may ask you to trust the developer profile on the phone under
Settings > General > VPN & Device Management.

## Make an archive

```bash
scripts/archive.sh 7
```

The argument is the build number. The script archives Release and exports an
App Store Connect `.ipa` to `build/export/`, then prints its path. It never uploads.
The version (`MARKETING_VERSION`) is 0.1.0 in `project.yml`.

## Upload to TestFlight

Once, create the app record in App Store Connect with bundle id
`inc.cowboy.cowchat.ios`. Then upload the `.ipa` by hand with Transporter, or
open the archive in Xcode Organizer and choose Distribute App. Each upload needs
a build number higher than the last.

## Live test

`scripts/live-test.sh <cowchat-cli> <api-key-file> [tcp-port]` drives the app in the Simulator against a running
`cowchat-server` on loopback. The server needs a `design-review` room and an encrypted `sealed` room that holds a
message sent with the room key `correct horse battery`. The test signs in, reads history and live messages, sends
with a mention, backgrounds and returns, checks the unread mark, and unlocks and writes to the encrypted room.
