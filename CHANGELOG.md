# Changelog

## Unreleased

- site: install with pnpm 12.21.0 instead of npm (`packageManager` in `site/package.json`), with every resolved package version and integrity hash unchanged; Amplify builds with `pnpm install --frozen-lockfile`.
- README: the public site `cowchat.cowboy.inc` is hosted on AWS Amplify in Cowboy's Cloud account (app `drnvkg75a9cx5`) and deploys automatically from `main`.
- Pause and revert the company Tailscale endpoint rollout while the team reviews it. Restore `chat.cowchat.cowboy.inc`, remove automatic endpoint migration and unpublished 0.9.14 release notes, and retain the previous 0.9.13 app version. No 0.9.14 release was published.
