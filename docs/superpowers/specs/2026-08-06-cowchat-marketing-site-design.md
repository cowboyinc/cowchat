# Cowchat Marketing Site — Design

**Date:** 2026-08-06
**Status:** Approved
**Target:** https://cowchat.cowboy.inc (replaces https://clawchat.live)

## Context & current state

- `site/` in this repo is already a Next.js 15 port of clawchat.live, rebranded
  to cowchat (commit f6bf692), with a root `amplify.yml` (`appRoot: site`) and a
  `prebuild` script that copies `SKILLS.md` → `public/skills.txt`.
- The infrastructure already exists: `aws-infrastructure/environments/prd/main.tf`
  defines module `cowchat_site` (Amplify app `cowchat-site`, repo
  `cowboyinc/cowchat`, branch `main`, domain association creating
  `cowchat.cowboy.inc` in the cowboy.inc hosted zone, WAF on, Bot Control
  deliberately **off** so agents can fetch `/skills.txt`). A hosted server is
  provisioned at `chat.cowchat.cowboy.inc` (EC2).
- The domain currently shows the Amplify "first deployment" placeholder: the
  Terraform module intentionally does not connect the GitHub repo (tokens stay
  out of TF state); repo connection and env vars are one-time Amplify console
  steps.
- Gallop (`@cowboyinc/gallop-foundations` / `-components`) is published only to
  GitHub Packages (private registry). Cowchat is a public repo, so a registry
  dependency would break `npm ci` for external contributors.

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Scope | Rich one-pager | Simple, but actually demonstrates capabilities (old site had one throwaway feature line) |
| Repo | `site/` in cowchat repo | Infra already encodes this (Amplify app → this repo); site lives next to product; skills.txt sync is a local copy |
| Gallop | Vendor built CSS + fonts | No auth anywhere; public contributors and Amplify build untouched; drift acceptable for a one-pager |
| Build approach | Evolve `site/` in place | Preserves working amplify.yml + skills.txt plumbing; convert to TS, add Tailwind 4 |
| Extra content | Mac app section only | Live stats, protocol snippet, and codex-bridge section explicitly declined |

## Stack & structure

- Next.js 15 App Router, TypeScript, Tailwind CSS 4.
- Vendored `gallop.css` — copy of gallop's
  `packages/foundations/src/gallop.css` (Tailwind v4 `@theme` token sheet,
  ~602 lines), imported after `@import "tailwindcss"` per its own consumer
  instructions. Record the source commit in a comment at the top of the
  vendored file so refreshes are diffable.
- Fonts: `SeasonSansUprightsVF.woff2` and `SeasonMixUprightsVF.woff2` copied
  from gallop `apps/docs/assets/fonts/`, self-hosted with `@font-face` +
  `font-display: swap`. *Flag:* this puts Season font binaries in a public
  repo; they are already served publicly by gallop.cowboy.inc. If licensing
  becomes a concern, switch to referencing them from a Cowboy-hosted URL.
- Pages: `/` (one-pager) and `/how-it-works` (kept, restyled with the same
  tokens). No other routes.
- `prebuild`/`predev` copy of `SKILLS.md → public/skills.txt` stays untouched.
- `amplify.yml` stays untouched (`appRoot: site`, npm, `.next` artifacts).

## Page design (`/`, top to bottom)

1. **Hero** — "cowchat 🐄" wordmark, the existing tagline ("Get two AI
   chatbots collaborating in real time."), and the signature paste-prompt
   box with copy button as primary CTA (paste into one chatbot; it sets up the
   second via `https://cowchat.cowboy.inc/skills.txt`). The prompt text keeps
   its current wording.
2. **Capabilities** — four Gallop-styled cards, one line of copy each:
   Rooms · Sealed-ballot votes · Leader election · End-to-end encryption.
3. **Mac app** — real screenshot of the Cowchat Mac app (dark-mode window,
   captured via the window-ID screenshot flow from the dev loop), one line of
   copy, "Download for Mac" button linking to the latest GitHub release DMG
   (link to the release *page*, not a version-pinned asset URL, so it doesn't
   go stale).
4. **Run your own** — terminal-styled block:
   `brew install cowboyinc/tap/cowchat` then `cowchat-server serve`.
5. **Footer** — How it works · GitHub · Skills · MIT/Apache-2.0.

## Visual style

- Gallop light + dark. Dark mode activates via Gallop's `.dark` class on
  `<html>`, driven by a small inline head script mapping
  `prefers-color-scheme` (no toggle UI, no flash of wrong theme).
- Warm hay/bison palette, Season display type for headings, `type-*` /
  semantic token utilities only — never raw palette values (Gallop's own
  rule).
- Single-column, centered, generous whitespace; the page must read well at
  mobile widths (the prompt box and terminal block scroll horizontally inside
  their containers rather than breaking layout).

## Deployment

Shipping = PR → main. Landing on main triggers the Amplify auto-build
(`enable_auto_build = true`). Remaining one-time console steps (operator, has
AWS access):

1. In the Amplify console, confirm the `cowchat-site` app is connected to
   `cowboyinc/cowchat` (Terraform intentionally leaves this manual).
2. Set env var `AMPLIFY_MONOREPO_APP_ROOT=site` on the app (required for the
   monorepo `applications:` build-spec format).
3. Verify the post-merge build succeeds and https://cowchat.cowboy.inc serves
   the site.

Follow-up (out of scope here): redirect clawchat.live → cowchat.cowboy.inc.

## Verification

- `npm run build` and `tsc --noEmit` pass in `site/`.
- Both pages render correctly in light and dark locally.
- All links resolve: GitHub repo, latest release, `/skills.txt`,
  `/how-it-works`.
- `public/skills.txt` present in the build output.

## Out of scope

- Live stats (active rooms/agents) — declined; would need a stats endpoint on
  the hosted server.
- Protocol snippet section and codex-wake-bridge section — declined.
- Any change to `SKILLS.md`, the server, or the Mac app.
- clawchat.live redirect (follow-up after launch).
