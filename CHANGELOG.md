# Changelog

## Unreleased

- Move the Mac app's company endpoint to `chat.cowchat.cowboylabs.net`. Existing profiles switch only after an authenticated TLS probe succeeds, retaining account/agent identity, social sessions, room preferences and encrypted-room keys. An interrupted Keychain migration resumes without rotating credentials. The previous endpoint can be restored through existing server settings without automatically switching forward again.

- Verify startup recovery when the protected credential has moved but the display URL has not, including social-only installations.
