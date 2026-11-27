# Changelog

## Unreleased

- site: Neon's copy pass of 26 November 2026, made in cowboyinc/prototype-site#707 first. Three sections lose the line under their heading, Bring someone in gets a plainer line, the closing band reads Try Cowchat, shot 2's report is shorter, and the sample's people and team have ordinary names with the same initials. On 27 November the line under the headline became Neon's "Talk to your teammates and coding agents in the same room."
- site: Next.js 15.5.29 (from 15.5.22) fixes the critical Next.js advisories, and overrides lift the PostCSS and sharp copies Next.js pins to 8.5.26 and 0.35.5; `pnpm audit` in `site/` reports no known vulnerabilities.
- tools/cowchat-artifacts: Zod 4.6.7 (from 4.3.6) fixes the Zod memory-exhaustion advisory, and the embedded artifact renderer is rebuilt with it. The Zod 3 copy that `@openuidev/react-ui` pulls in through `@ag-ui/core` has no patched 3.x release and stays.
- CLAUDE.md (and its AGENTS.md symlink): point the key-file paths and the "Adding Features" steps for `cowchat-core` and `cowchat-client` at the cowboyinc/cowchat-core repository, where those crates now live.
- site: install with pnpm 12.21.0 instead of npm (`packageManager` in `site/package.json`), with every resolved package version and integrity hash unchanged; Amplify builds with `pnpm install --frozen-lockfile`.
- README: the public site `cowchat.cowboy.inc` is hosted on AWS Amplify in Cowboy's Cloud account (app `drnvkg75a9cx5`) and deploys automatically from `main`.
- Pause and revert the company Tailscale endpoint rollout while the team reviews it. Restore `chat.cowchat.cowboy.inc`, remove automatic endpoint migration and unpublished 0.9.14 release notes, and retain the previous 0.9.13 app version. No 0.9.14 release was published.
