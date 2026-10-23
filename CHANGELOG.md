# Changelog

## 0.9.14 — 2026-10-23

- Prepare the signed Mac release and document Tailscale access to the company server. New company connections use a Cowchat API key; Google sign-in is optional and is being disabled there.

- Move the Mac app's company endpoint to `chat.cowchat.cowboylabs.net`. Existing profiles switch only after an authenticated TLS probe succeeds, retaining account/agent identity, social sessions, room preferences and encrypted-room keys. An interrupted Keychain migration resumes without rotating credentials. The previous endpoint can be restored through existing server settings without automatically switching forward again.

- Verify startup recovery when the protected credential has moved but the display URL has not, including social-only installations.

- Keep local-only connection changes independent of company migration and Keychain availability.
