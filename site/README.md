# cowchat.cowboy.inc

This folder is Cowchat's public site. It is a Next.js 15 app exported statically to `out/`, and AWS Amplify serves it from `main` (`../amplify.yml`). It uses Tailwind 4 over a vendored copy of Gallop's foundations CSS (`app/gallop.css`, v0.7.0) and the two Season fonts in `app/fonts/`. The site cannot reach Cowboy's private npm registry, so nothing from Gallop's packages is installed, and what the page needs is copied in.

```sh
npm ci
npm run dev        # http://localhost:3000
npm run typecheck
npm run build      # writes out/
```

`predev` and `prebuild` copy `../SKILLS.md` to `public/skills.txt`, which agents read at `/skills.txt`.

## The page

`/` is the marketing page designed in the prototype (cowboyinc/prototype-site, `/cowchat-site`, PR 531) and ported here section for section on 12 November 2026. `/how-it-works` sends the reader to `/` with a meta refresh, since the static export ignores Next's redirects.

- `components/cowchat-site/CowchatSite.tsx` draws the page, and `copy.ts` holds every word. Codex wrote the words, so a change of words starts in the prototype.
- `components/cowchat-site/shots/` holds the four product shots, drawn in React from Gallop's tokens and icons as recreations of the Mac app's screens. Each shot's `REHEARSAL.md` records what was filmed and checked in the prototype.
- `shared.css` holds the rules the page shares with cowboy.inc's home and Marshal's site, copied from the prototype. `cowchat-site.css` and `shots/shots.css` are the page's own.
- `gallop.tsx` and `icons.tsx` are local copies of Gallop's Button, Spinner and the eight icons the page draws. `app/tokens.css` holds the spacing, radius, type, motion and elevation tokens the page reads that Gallop does not publish, at the prototype's values.
- `components/theme.ts` follows the system's light or dark scheme, before the first paint and whenever the scheme changes.

Comments that cite "plan" mean the prototype's `docs/plans/cowchat-site.md`. The prototype's `copy.test.ts`, which checks the copy's shape, is not here, since this site has no test runner.

## The marks

Claude Code and Codex appear by name and by their official marks (Patrick, 3 November 2026), in the Works with card and on the install section's plugin and hooks rows. Each mark is the owner's file, unaltered, on a 40px tile, or 32px on the install rows. The Claude spark draws at 60% of the tile. OpenAI's Blossom file keeps a clear space of about a quarter of its box on each side, so it fills the tile and the symbol shows at about half the tile's width.

| File in `public/assets/cowchat-site/` | Source | sha256 |
| --- | --- | --- |
| `claude-spark.svg` | Anthropic's press kit, `Anthropic media resources/Anthropic logos/Claude logos/3 Claude Spark/SVG/Claude Spark - Clay.svg`. Claude Code's mark on the page. | `6d53db4be375e899c937c26cf16684a80d6e869b1928d72b37748bef2560e219` |
| `claude-icon.svg` | The same kit, `Anthropic media resources/Anthropic logos/Claude logos/4 Claude icon/SVG/ClaudeIcon-Rounded.svg`. A Claude Code session's mark in the shots. | `059e22f525d67c6258c4f64514f0b0e717c914df8a706936d0299d5e6b8082d9` |
| `openai-blossom-black.svg` | Patrick's download of `https://openai.com/brand/` on 3 November 2026, `SVGs/OAI_OpenAI-Blossom_Black.svg`. Codex's mark in the light theme. | `75c1e9fffa5e8c437bec1d67197a73992bca45d166c6ff23215185dea8fae92a` |
| `openai-blossom-white.svg` | The same download, `SVGs/OAI_OpenAI-Blossom_White.svg`. Codex's mark in the dark theme. | `01d158767c4eec0e47bd617e67759c33da0accd1438be1a8d29dfdb99ce87285` |

Anthropic's press kit is the Newsroom's "Download press kit" link, `https://www.anthropic.com/press-kit`, which redirects to `https://www-cdn.anthropic.com/ae59ca4ca194dac9c9dc3bc78c5829468cb0e8af.zip` (last modified 6 February 2026). OpenAI's download holds no Codex mark, so Codex takes the OpenAI symbol, as the Claude spark stands for Claude Code.

What each owner's pages say about the marks, as read for the port on 12 November 2026:

- **Anthropic's press kit** holds logos and executive bios and carries no usage terms. Both Claude files above still match it byte for byte.
- **Anthropic's Usage Policy** (effective 12 November 2026) says nothing about Anthropic's names, logos or trademarks.
- **Anthropic's Commercial Terms** (effective 17 June 2025) say they grant neither party any rights to the other's intellectual property, and say nothing else about Anthropic's marks.
- **Anthropic's Consumer Terms** (effective 8 October 2025), section 12, "Use of our brand", say: "You may not, without our prior written permission, use our name, logos, or other trademarks in connection with products or services other than the Services, or in any other way that implies our affiliation, endorsement, or sponsorship. To seek permission, please email us at marketing@anthropic.com." This page draws Claude's marks on another company's product page, so whether it needs that permission is Patrick's call.
- **OpenAI's download** carried no terms. The brand page's own lines say that co-branded material needs OpenAI's approval, through partnercomms@openai.com, and, as a search engine quotes the same page, that OpenAI's marks may not be more prominent than your own or imply endorsement. Whether this page needs that approval is Patrick's call. The brand page refuses automated requests, so nobody fetches it from a script; a person downloads the files in a browser.

The footer says that the agents' names and marks belong to their owners.

## The app icon and the shots' screenshots

`public/assets/cowchat-site/cowchat-icon.png` is the Mac app's `apps/CowchatMac/Sources/CowchatMac/Resources/CowchatIcon.png` (last changed in 8e32baf), scaled to 256 (sha256 `fe7e7d170669bb2c6e777d92d0343c79613a0fcafbb4bbde14154989ad1d1f58`). The page rounds it as macOS masks it.

`shop-home.jpg` and `shop-widget.jpg` beside it are the prototype's own synthetic shop pages (its `public/assets/chat/browser/`), drawn as the two screenshots on the report in the second shot.
