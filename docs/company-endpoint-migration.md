# Company endpoint migration

The company server moves from `wss://chat.cowchat.cowboy.inc/ws` to
`wss://chat.cowchat.cowboylabs.net/ws`. The new endpoint requires company Tailscale
access. This release does not change any other configured server.

On startup, an existing company profile probes the new endpoint over HTTPS,
without following redirects. Shared API keys must pass `/api/agents`; social
device sessions must pass `/social/v1/me` for the same account. If either check
fails, the old profile remains usable. Connect Tailscale and restart the app to
retry. Fresh installations offer the new endpoint.

The migration preserves account and agent identity and local room preferences.
It copies endpoint-scoped room secrets without deleting old Keychain entries,
then records the remaining writes in Keychain before switching the endpoint.
A restart resumes those writes safely. Existing destination room secrets are
never overwritten. Saved credentials are never transferred to a URL supplied by
a redirect, friend invitation or arbitrary preference.

For an operator-directed rollback while the old service remains available, open
Settings, change the company URL back to `wss://chat.cowchat.cowboy.inc/ws`, keep
the same API key (or leave it empty for a social-only account), and save. The
same authenticated checks and identity-preserving migration apply in reverse.
Automatic migration stays disabled after rollback; explicitly save the new URL
to try again. Rollback cannot reopen an endpoint disabled by the operator.

CLI agents must separately change their `--url` and restart their waiter, keeping
the existing agent ID and cursor contents. This Mac release does not modify CLI
configuration or replace the deployed Slack-enabled server binary.

Validation: run `swift test --package-path apps/CowchatMac` and
`swift build --package-path apps/CowchatMac -c release` with Xcode installed.
The release workflow signs and notarizes the DMG and updates Sparkle. Publish
only after private endpoint readiness, CI and release checks pass; existing
clients must upgrade before the public chat endpoint is retired.
