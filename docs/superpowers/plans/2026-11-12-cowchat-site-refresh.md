# Cowchat marketing page: the plan the port follows

Copied on 12 November 2026 from cowboyinc/prototype-site `docs/plans/cowchat-site.md` at main 39d5d7f63, where the page was built and reviewed (PR 531). Patrick decided on 12 November that the full page goes live on cowchat.cowboy.inc now and does not wait for the release that carries the session-room tasks, so the "When it can go live" section and the cutting rule in step 6 below no longer gate this port. The port is PR 106 on this repository.

---

# Cowchat's marketing site: the plan (3 November 2026)

Patrick asked on 3 November 2026 for cowchat.cowboy.inc to be refreshed, with msgblast.app as inspiration and the cowboy.inc and Marshal sites as references. The research, with the full claims table and what each reference does, is `docs/plans/cowchat-site-why.md`. Patrick answered the plan's questions the same day (Decisions, below). He picked the hero from the third round (Hero, below), and the page is built at `/cowchat-site` with its copy and its four drawn shots.

## When it can go live

The page describes the rebuilt app, and none of it is in the shipped release (0.9.15, 30 October). The prototype page can be built now. The port goes live with the release that ships what the hero, section 3 and the install section claim, which is Chad's tasks 1 to 10 and 13. Task 4 is in that list because tasks 1, 5 and 8 depend on it. A later section whose task has not shipped by then is cut for launch, never softened: section 4 needs task 11, and section 5 needs task 12.

## The page, top to bottom

1. **Bar.** Cowchat's icon and name on the left, GitHub and Download for Mac on the right, and no section links. A phone keeps the icon, the name and GitHub.
2. **Hero.** It names the category and says why it beats what the reader does today. The app icon sits over the headline and line. Under them are two buttons of one size, Download for Mac and a second that scrolls to section 7 to install the server and plugin, and under those an Open source text link, as msgblast has it. Below them, on a full-width band as Marshal's hero shot sits, is shot 1: a session's room with a teammate in it, who mentions the session; it thinks and answers with its pull request.
3. **Every session gets a room.** The core idea: start Claude Code or Codex and its room is there to talk to. Words left, shot 2 right: a session's room with its report, its pull request row and two screenshots.
4. **Bring a teammate in.** The reason to stop copying prompts between people. A centred heading over shot 3, wide: the Join sheet over the Inbox, where the reader accepts a teammate's invitation to a room and brings one of their own sessions.
5. **Your team's sessions.** Shot 4 left, the Team page grouped by person with repository, pull request and state, and words right.
6. **Works with.** msgblast's "Bring the favorites you already have", grouped by how each joins, as one card of three rows. Claude Code and Codex, each with its official mark and name, join by themselves through the plugin and hooks. Any agent with a shell joins from a prompt through the CLI and skill file. People join as friends by handle after signing in with Google or GitHub.
7. **Install** (`#install`). cowboy.inc's command row, one per piece, each with Copy:
   - The Mac app, whose first run installs the Claude Code plugin and walks through trusting the Codex hook (spec task 12, Chad task 13): `brew install --cask cowboyinc/tap/cowchat`, beside Download for Mac.
   - The Cowchat server and CLI for Mac and Linux: `brew install cowboyinc/tap/cowchat`.
   - The Claude Code plugin with its session hooks (Chad task 6), under the Claude Code mark.
   - The Codex hooks with the wake bridge, Cowchat's MCP server for Codex (Chad task 7), under the Codex mark.

   The first two commands ship today. The plans give no command for the last two. They say only that the Mac app's first run installs the plugin and trusts the Codex hook. Today's wake bridge is experimental, is built from source and is registered with `codex mcp add` (cowchat `docs/codex-wake.md`), and the release archives do not include it. Chad's tasks 6 and 7 have to name each install command, and task 2 has to say where a hook finds its key on a machine without the Mac app, since the plan has the app write it. Until then those two rows are stubbed in the prototype, and the port does not go live with a stub.
8. **Open source.** View on GitHub, the license and one line saying the server runs on your own machine and local clients need no account. The rebuilt app is sign-in first (spec items 1 and 7), so the line is about the server and CLI, never the app.
9. **Closing.** The icon again, one line and the hero's two buttons, as msgblast closes.
10. **Footer.** Cowchat (GitHub, Releases, the skill file), Cowboy (cowboy.inc, General Store, Marshal) and Legal (Terms, Privacy), with msgblast's line that the agents' names and marks belong to their owners.

Each section has its own form, after Patrick's note that Marshal's first build was homogeneous. Codex writes every word once the layout is built; this plan fixes what each section holds, not its words.

## The claims

Each section says only these, cited in the route's `CLAUDE.md`. The research's table has each claim's wording.

- **Shipped in 0.9.15:** `mac` (README; 0.9.10 notes), `install` (README), `cli` (README; the release archives hold `cowchat` and `cowchat-server` for Mac and Linux), `open` (README license; the repository is public), `local` (README, Local), `any-agent` (README, Protocol), `friends` and `approve` (0.9.8 notes; `docs/friends.md`).
- **Ships with Chad's tasks** (spec in `cowchat-simple.md`, tasks in `cowchat-server-for-chad.md`): `session-room` (spec item 1; tasks 1, 6, 7), `talk` (spec task 3; tasks 3, 6, 7), `state` (spec item 8; task 9), `for-people` (spec item 3; task 8), `add-person` (spec item 5; tasks 4, 11), `inbox` (spec item 4; task 5), `finished` (spec item 9; task 10), `team` (spec item 11; task 12), `first-run` (spec task 12; task 13), `plugin` (spec task 6; tasks 2, 6) and `codex-hooks` (spec task 7; tasks 2, 7). A spec item is a line of "What a person sees" in `cowchat-simple.md`; a bare task number is Chad's.
- **By section:** hero `friends`, `approve`, `session-room`, `talk`, and `add-person` for more than one teammate; 3 `session-room`, `talk`, `for-people`, `finished`; 4 `add-person`, `approve`, `friends`; 5 `team`, `state`; 6 `session-room`, `plugin`, `codex-hooks`, `any-agent`, `friends`; 7 `install`, `first-run`, `cli`, `plugin`, `codex-hooks`; 8 `open`, `local` (the server and CLI only).
- **Never said:** replacing Slack, history kept forever, encryption, free, counts, speed, Windows, or any agent other than Claude Code and Codex by name.

## Product shots

The four shots are drawn by the page in React from Gallop's tokens and icons, as Marshal's are, using Benji Taylor's stageshow skill as the method and the bar (`~/.agents/skills/stageshow/`; Patrick, 3 November). Its kit is not used: it is a design system of its own, with its own colour literals, and the tokens come from Gallop (Patrick: "we still ground in gallop"). The plan replaced captures of `Cowchat Proto.app` from a synthetic fixture world on 3 November. The drawn shots do not depend on the prototype's next round landing, and they read as product shots, where a whole window shrunk into a frame reads as a screenshot in a box.

- **True to the app.** Each shot is a recreation of a screen in the Mac prototype as P9 builds it (`cowchat-mac-proto` at `native/CowchatMac/`, the renders in the scratchpad's `proto/p9/` and `proto/design/navigation-2.frozen.md`). Nothing appears that the app does not draw. The app draws a session's place in the sidebar with Apple's `laptopcomputer` symbol, which Gallop lacks, so no shot shows the sidebar's list. Every shot is cropped to the pane its story happens in, so the frame's edge is the pane's edge and the sidebar and the window around it are left out.
- **Data.** Invented people, handles, repositories, branches, pull requests and messages, written by Codex (`SAMPLE` in `copy.ts`). The two screenshots in shot 2 are the prototype's own synthetic shop pages (`public/assets/chat/browser/`). A Claude Code session's mark is Anthropic's Claude icon from the press kit, on the same clay square where the app draws an SF Symbol asterisk.
- **Phone.** Each shot has its own design for a phone, cropped closer at the same type sizes. The hero's and the room's headers are cropped off, the hero's feed sits at its newest message under a fade, the pull request row takes the app's two-line form, the Join sheet sits at the frame's foot under the dimmed Inbox's title and invitation, and the Team page leaves out its switch, its search and its Repo column, since a Mac window has no phone width.
- **Motion.** Only the hero moves. It opens on its poster and returns to it, so the server's render, reduced motion and a screenshot show the whole story. Each shot's `REHEARSAL.md` beside its component records what was filmed, the review and what was not checked.

| Shot | Medium | Focal object | Action | Payoff |
|---|---|---|---|---|
| 1, hero | Loop when in view, full width | A session's room with a teammate in it | The teammate mentions the session; it thinks, then answers | The answer with its pull request, under the question |
| 2, a room per session | Still, beside the copy | The session's report in its room | None | The pull request and two screenshots on the report |
| 3, bring someone in | Still, full width | The Join sheet over the Inbox | None | The invitation, with a session of the reader's own picked to bring |
| 4, team sessions | Still, beside the copy | The Team page's first two people | None | Each branch's state, two of them working |

## Where it is built, and how it reaches cowchat.cowboy.inc

It is built here as Marshal's site was: `/cowchat-site` is its own top-level route with its own bar and footer (`src/app/cowchat-site/`, `src/components/cowchat-site/` with `copy.ts` and `copy.test.ts`, shots under `public/assets/cowchat-site/` so `run.mjs` scopes them to the surface, a registry row with no chrome, and a guard `tools/snap/cowchat-site.mjs` in `run.mjs`). It follows the system's light or dark scheme with Marshal's theme script. "Room" is Cowchat's product noun, and the words guard fails on it everywhere. Its allow list matches one exact string per file with a count (`failures` in `tools/snap/words.mjs`), as the `src/lib/fixtures/developer.ts` entry does, so per-string entries would break each time Codex rewrites a sentence. The layout task adds a rule to `words.mjs` that skips room in `src/app/cowchat-site/` and `src/components/cowchat-site/`, recorded in `docs/decisions.md`. That file belongs to no surface, so the PR runs the full chain. The registry row's description in `src/lib/surfaces.ts` says session, since the rule does not cover it.

The port is another session's. The cowchat repository's `site/` is public, exported statically, served by Amplify from `main`, and vendors Gallop's foundations (v0.5.0) so no private registry is needed. The port copies the page's markup and CSS into `site/app/`, swaps Gallop's Button for the site's own button classes on the same tokens, refreshes the vendored `gallop.css`, and keeps the `predev` and `prebuild` scripts that copy `../SKILLS.md` to `public/skills.txt`, which agents read. The site is a static export (`output: "export"`), which ignores Next's redirects, so `/how-it-works` becomes a page that sends the reader to `/` with a meta refresh and a link. Download links to the latest GitHub release, or to the stable DMG address if cowchat PR #53 lands.

## What it borrows from cowboy.inc and Marshal

- **Type and colour.** Gallop's scale as both pages use it: Season Mix for the hero (56/64) and section headings (44/52), Season Sans for the line under a heading (20/28) and body (15/20), the hay ground, Surface/600 cards and 80 between sections. msgblast's SF Pro and blue button are not carried.
- **Shared parts.** The bar, footer and section-head rules in `site-structured.css` and `site-full.css`, imported as Marshal does; Gallop's Button, a size up in bands; `TextLink`; cowboy.inc's command row with Copy and View.
- **Patterns.** From Marshal: the product as the hero's picture on a full-width band, one form per section, synthetic data, phone crops and the system theme. From cowboy.inc: no eyebrow, no label above a heading, no counter strip, and every claim cited in the route's `CLAUDE.md`. Not borrowed: the desert painting or the Marshal character.

## Hero

**Patrick's pick, 3 November: "Group chat for coding agents" over "Keep the conversation with your teammate and the agents doing the work."** It is the first of the third round's four (below). The headline names the category in words a reader already uses. The line rests on `friends` and `approve`, which shipped in 0.9.8: two friends share a room, and each approves their own agents to read and send there. Inside a session's room it rests on `session-room` and `talk`, which ship at launch, and a conversation with more than one teammate rests on `add-person` (Chad task 11). With task 11 cut for launch, the line is still true of one teammate, so the hero does not wait for it.

The copy pass leaves the headline as it is. It may tighten the line only if every claim stays true, and any change goes to the lead before it is applied.

### How it was reached


Patrick rejected Codex's first five pairs on 3 November ("use codex to help write non ai slop copy"). The second round gave Codex the stop-slop rules as hard bans, and asked first for the value and only then for eight pairs. Codex described the reader's day today like this:

> You run Claude Code or Codex in terminal tabs and windows. When several sessions are running, you go tab by tab to see which has finished and which is waiting at a permission prompt. When you need a teammate's session involved, you copy your session's prompt or result into Slack for them to paste into their session. They copy the answer back so you can paste it into yours. Checking sessions interrupts your work, and carrying messages between them makes each exchange depend on you and your teammate being available to relay it.

Its value in one sentence was "Cowchat is being built to let you check on your Claude Code or Codex sessions and involve a teammate's session in the conversation without carrying messages between your terminals through Slack" (`state`, `talk`, `add-person`). The pairs are in the present tense, since the page goes live only with its claims.

I read the eight against the bans and cut two. Pair 2, "Session chat with pull request previews", names a category a reader would not search for, and "previews" overstates the pull request row a finished message carries. Pair 7, "A session manager for your Mac", rewords a rejected headline. These six survived, as Codex wrote them, with Codex's numbers (an asterisk marks a planned claim):

| | Headline | Line | Claims |
| --- | --- | --- | --- |
| 1 | A desktop inbox for coding sessions | Cowchat alerts you when Claude Code or Codex has a message for you, even while you're working in another app. | `inbox`* |
| 3 | Chat rooms for your terminal sessions | Your Claude Code or Codex session joins a room automatically, named for its folder and branch. | `session-room`* |
| 4 | A messaging app for coding sessions | Mention the Claude Code or Codex session you want to answer, and Cowchat wakes only that session. | `talk`* |
| 5 | A chat app for session updates | Cowchat folds away session messages that aren't marked for people so you can follow the conversation in each room. | `for-people`* |
| 6 | A session launcher for your Mac | Start Claude Code or Codex in a new worktree from Cowchat when you need a separate session. | `new-session`*, `mac` |
| 8 | A team directory for coding sessions | See your teammates' live Claude Code or Codex sessions grouped by person, with each session's repository and state. | `team`*, which the launch may not ship |

**My pick is 1.** It names the product with a word the reader already uses, an inbox, and its line says what changes from going tab by tab: a session's message reaches you while you work in another app. It also describes shot 1, the Inbox beside the session rooms, and it rests on `inbox`, which the launch release ships. Codex picked the same pair. No pair states the comparison with terminal tabs and Slack outright, and 1 comes closest. Pair 8 can lead only if the Team page ships at launch, and pair 6 describes one feature as the whole product.

Patrick read pairs 1, 3, 4 and 8 and said "i dont like any of these?". They read as feature descriptions with a category in front, which was the same miss as Marshal's early hero rounds, and the bans had pushed Codex toward literal lines.

The third round gave Codex the register of the heroes Patrick has approved: "The App Store for AI"; his own "Private AI code review" over "Never let another company into your codebase while outperforming SOTA code review tools"; and msgblast's "All Your AI Agents. Together on your Mac." over "Start with one agent or message multiple at the same time." Each has a short headline that names the category with one idea that sets it apart, and a line that sells the value. It kept the bans on hype words, reversals, groups of three, invented terms and untrue claims, allowed a two-beat headline, and banned describing how Cowchat works. I cut "Your coding sessions at a glance" (a feature description), "Coding chat with results attached" (an invented category) and "Chat for the way you code" (a tagline with no category). The four sent to Patrick, in order:

| | Headline | Line | Rests on |
| --- | --- | --- | --- |
| 1 | Group chat for coding agents | Keep the conversation with your teammate and the agents doing the work. | `friends` and `approve` for one teammate, `add-person`* for more |
| 2 | Open source chat for coding agents | Work with Claude Code and Codex in a Mac app you can make your own. | `open`, `mac`, `session-room`* |
| 3 | One home for your coding sessions | Stay on top of your Claude Code and Codex work from one Mac app. | `mac`, `session-room`*, `state`* |
| 4 | Your coding work, easier to follow | Catch up on what needs you across Claude Code and Codex without making the rounds through your tabs. | `session-room`*, `inbox`* |

The fifth that passed the bans was "Coding agents on your team" over "Work through the same problem with a teammate and the sessions you're both running."

## Decisions (Patrick, 3 November)

1. **Claude Code and Codex appear by name and by their official marks**, in the Works with card, on the install section's plugin and hook rows, and nowhere else unless a mark helps a reader find something. The marks come unaltered from Anthropic's and OpenAI's published brand assets, under their usage guidelines, and the route's `CLAUDE.md` records each file's source. Before the port goes live, the port session reads both guidelines and records what each allows for a third party's page. If either does not allow this use, Patrick decides before launch. The footer carries msgblast's line that names and artwork belong to their owners.
2. **The main actions are Download for Mac and installing the server and plugin**, since the page works against the plan with session hooks and MCP (Patrick: "download or install cowchat server + plugin"). Section 7 holds the install path and says which commands the plans have not given yet.
3. **Open source is a text link under the hero's buttons and a section of its own.**
4. **One page.** `/how-it-works` sends the reader to `/`, and `/skills.txt` stays.

## Steps

1. Patrick picks the hero (done, 3 November), and Chad names the install commands for the plugin and the Codex hooks.
2. Opus draws the four shots with stageshow's method in Gallop, Codex writes their sample, and each shot gets a `REHEARSAL.md` beside it. The synthetic world in the Mac prototype is dropped (3 November).
3. Opus builds `/cowchat-site` with the copy stubbed, the shots, the two marks and the guard. `npm run check -- --surface cowchat-site` passes. The copy test checks the copy's shape and the synthetic names here, and its check that no string is still pending turns on in step 4. The page stood with placeholder shots on 3 November until step 2 drew them. Claude Code's mark is Anthropic's Claude spark. Codex's is OpenAI's Blossom from Patrick's download of OpenAI's brand page, black in the light theme and white in the dark (the route's `CLAUDE.md`, The marks).
4. Codex writes every string as one pass over the built page's screenshots, and Patrick reads them. Codex's pass is in (3 November), with one fix call for five strings; it kept the hero line.
5. Fable and Codex review the rendered page once, with one fix round. There is one PR, with its evidence page added by `npm run artifact:add`.
6. When the release ships, another session ports the page to the cowchat repository's `site/`. It first checks that each claim's task shipped, cuts the sections that did not, and checks each shot against the same screen in the released app, redrawing or cutting one that differs.
