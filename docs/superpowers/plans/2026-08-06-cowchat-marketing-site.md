# Cowchat Marketing Site Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild `site/` as a Gallop-styled one-pager for cowchat.cowboy.inc (hero + paste-prompt, capability cards, Mac app section, install block), shipped as a PR to main.

**Architecture:** Evolve the existing Next.js 15 App Router app in `site/` in place: convert to TypeScript, add Tailwind CSS 4, and vendor Gallop's foundations (`gallop.css` token sheet + Season fonts) so no private-registry auth is needed anywhere. Two routes: `/` and `/how-it-works`. Deployment is untouched: root `amplify.yml` (`appRoot: site`) builds on push to main.

**Tech Stack:** Next.js 15 (App Router), React 19, TypeScript 5, Tailwind CSS 4 via `@tailwindcss/postcss`, vendored `gallop.css` (Tailwind v4 `@theme` sheet), self-hosted Season variable fonts via `next/font/local`.

**Spec:** `docs/superpowers/specs/2026-08-06-cowchat-marketing-site-design.md`

## Global Constraints

- Copy rules (verbatim, do not rewrite): tagline `Get two AI chatbots collaborating in real time.`; the paste-prompt text (the `PROMPT` constant in `site/app/page.js`) is kept character-for-character.
- Styling rule (Gallop's own): use **semantic** token utilities (`text-text-primary`, `bg-surface-600`, `border-border-default`, `type-*`) — never raw palette classes like `bg-hay-500`.
- Do not modify: `amplify.yml`, `SKILLS.md`, anything outside `site/` (except the Mac screenshot capture, which only *reads* the app), the `predev`/`prebuild` skills.txt copy scripts.
- New dependencies limited to: `typescript`, `@types/node`, `@types/react`, `@types/react-dom`, `tailwindcss`, `@tailwindcss/postcss` (all devDependencies). No runtime deps added.
- Node 20+ (Amplify's default image provides it; use the same locally).
- The vendored `gallop.css` is copied from `~/Github/gallop/packages/foundations/src/gallop.css` at commit `121e38465ef5696bf27ed332a573347e4ae87959` (v0.5.0) and must keep a provenance header comment. Never hand-edit tokens inside it.
- Dark mode = Gallop's `.dark` class on `<html>`, set by an inline head script from `prefers-color-scheme`. No toggle UI.
- All shell commands run from the repo root unless the step says otherwise (`site/` commands are written as `cd site && …` in a single line; the shell cwd persists between commands in a session, so always re-anchor to repo root).

---

### Task 1: TypeScript + Tailwind 4 toolchain with vendored Gallop foundations

**Files:**
- Modify: `site/package.json`
- Create: `site/tsconfig.json`
- Create: `site/postcss.config.mjs`
- Create: `site/app/gallop.css` (vendored copy)
- Create: `site/app/fonts/SeasonSansUprightsVF.woff2`, `site/app/fonts/SeasonMixUprightsVF.woff2` (copied binaries)
- Create: `site/app/icon.svg`
- Modify: `site/app/globals.css` (full rewrite)
- Rename+rewrite: `site/app/layout.js` → `site/app/layout.tsx`
- Modify: `site/.gitignore` (ensure `next-env.d.ts` handling — see step 6)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: the Tailwind+Gallop pipeline every later task's classes rely on: semantic color utilities (`text-text-primary`, `text-text-secondary`, `text-text-tertiary`, `bg-surface-400/500/600`, `border-border-default`, `bg-button-primary-default/-hover/-pressed`, `text-button-primary-text-default`), typestyle utilities (`type-title`, `type-h2`, `type-h4`, `type-body-l`, `type-body-m`, `type-body-s`, `type-body-s-strong`, `type-body-m-strong`, `type-code-sm`), `btn-primary-glow`, and the `.dark` theme. Root layout renders `{children}` inside `<body>` with fonts + theme script applied.

- [ ] **Step 1: Vendor the Gallop foundations and fonts**

```bash
mkdir -p site/app/fonts
cp ~/Github/gallop/packages/foundations/src/gallop.css site/app/gallop.css
cp ~/Github/gallop/apps/docs/assets/fonts/SeasonSansUprightsVF.woff2 site/app/fonts/
cp ~/Github/gallop/apps/docs/assets/fonts/SeasonMixUprightsVF.woff2 site/app/fonts/
```

Then prepend this provenance header to `site/app/gallop.css` (above the existing first line):

```css
/*
 * VENDORED from cowboyinc/gallop packages/foundations/src/gallop.css
 * at commit 121e38465ef5696bf27ed332a573347e4ae87959 (foundations v0.5.0).
 * Do not edit by hand — refresh by re-copying from the gallop repo.
 */
```

- [ ] **Step 2: Update `site/package.json`**

Replace the whole file with:

```json
{
  "name": "cowchat-site",
  "private": true,
  "scripts": {
    "predev": "cp ../SKILLS.md public/skills.txt",
    "dev": "next dev",
    "prebuild": "cp ../SKILLS.md public/skills.txt",
    "build": "next build",
    "start": "next start",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "next": "^15.4.5",
    "react": "^19.1.1",
    "react-dom": "^19.1.1"
  },
  "devDependencies": {
    "@tailwindcss/postcss": "^4.0.0",
    "@types/node": "^22.0.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "tailwindcss": "^4.0.0",
    "typescript": "^5.8.3"
  }
}
```

- [ ] **Step 3: Create `site/postcss.config.mjs`**

```js
const config = {
  plugins: {
    '@tailwindcss/postcss': {},
  },
}

export default config
```

- [ ] **Step 4: Create `site/tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2017",
    "lib": ["dom", "dom.iterable", "esnext"],
    "allowJs": true,
    "skipLibCheck": true,
    "strict": true,
    "noEmit": true,
    "esModuleInterop": true,
    "module": "esnext",
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "jsx": "preserve",
    "incremental": true,
    "plugins": [{ "name": "next" }]
  },
  "include": ["next-env.d.ts", "**/*.ts", "**/*.tsx", ".next/types/**/*.ts"],
  "exclude": ["node_modules"]
}
```

`allowJs: true` keeps the not-yet-converted `.js` pages compiling; later tasks convert them.

- [ ] **Step 5: Rewrite `site/app/globals.css`** (replace entire contents)

```css
@import "tailwindcss";
@import "./gallop.css";

@layer base {
  html {
    font-family: var(--font-sans);
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }

  body {
    background-color: var(--color-surface-500);
    color: var(--color-text-primary);
  }
}
```

This mirrors gallop's own docs app (`apps/docs/src/app/globals.css`) minus its component `@source`.

- [ ] **Step 6: Convert the root layout** — `git mv site/app/layout.js site/app/layout.tsx`, then replace contents with:

```tsx
import type { Metadata } from "next";
import localFont from "next/font/local";
import "./globals.css";

const seasonSans = localFont({
  src: "./fonts/SeasonSansUprightsVF.woff2",
  variable: "--font-sans",
  display: "swap",
  weight: "100 900",
});

const seasonMix = localFont({
  src: "./fonts/SeasonMixUprightsVF.woff2",
  variable: "--font-display",
  display: "swap",
  weight: "100 900",
});

export const metadata: Metadata = {
  title: "Cowchat",
  description:
    "A local chat server for AI agents to coordinate. Point your agents at the skills file and they start talking.",
};

// Runs before paint so there is no flash of the wrong theme. Gallop's dark
// tokens activate via the .dark class on <html>.
const themeScript = `if (window.matchMedia("(prefers-color-scheme: dark)").matches) document.documentElement.classList.add("dark");`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html
      lang="en"
      className={`${seasonSans.variable} ${seasonMix.variable}`}
      suppressHydrationWarning
    >
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
```

(`suppressHydrationWarning` is required: the pre-hydration script changes the `<html>` class, which React would otherwise flag.) Also check `site/.gitignore`: if it does not already ignore `next-env.d.ts`, append a line `next-env.d.ts` (Next regenerates it on every build).

- [ ] **Step 7: Create `site/app/icon.svg`** (favicon; App Router picks up `app/icon.svg` automatically)

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text y="0.9em" font-size="90">&#128004;</text></svg>
```

- [ ] **Step 8: Install and build — verify the pipeline**

```bash
cd site && npm install && npm run build
```

Expected: build succeeds. Then verify the Gallop tokens and dark theme made it into the emitted CSS:

```bash
grep -l -- "--color-hay-500" site/.next/static/css/*.css && grep -l "\.dark" site/.next/static/css/*.css
```

Expected: both greps print a CSS filename (all `@theme` variables and the plain `.dark` block are always emitted; `@utility` classes appear only once used by pages in later tasks).

- [ ] **Step 9: Typecheck**

```bash
cd site && npm run typecheck
```

Expected: exit 0.

- [ ] **Step 10: Commit**

```bash
git add site
git commit -m "site: TypeScript + Tailwind 4 toolchain with vendored Gallop foundations"
```

Note: the old `globals.css` classes (`.prompt-box`, `.doc`, …) are gone after this task, so the still-unconverted pages render unstyled until Tasks 2–4 rebuild them. That's expected mid-branch.

---

### Task 2: Home page — hero, capabilities, install, footer

**Files:**
- Rename+rewrite: `site/app/page.js` → `site/app/page.tsx`
- Rename+rewrite: `site/app/copy-button.js` → `site/app/copy-button.tsx`

**Interfaces:**
- Consumes: Task 1's utilities and layout.
- Produces: `site/app/page.tsx` default-exports `Home()`; `site/app/copy-button.tsx` default-exports `CopyButton({ text }: { text: string })`. Task 3 inserts its Mac section into this page between the capabilities `<section>` and the "run your own" `<section>`.

- [ ] **Step 1: Convert the copy button** — `git mv site/app/copy-button.js site/app/copy-button.tsx`, then replace contents with:

```tsx
"use client";

import { useState } from "react";

export default function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="btn-primary-glow absolute right-3 top-3 cursor-pointer rounded-lg bg-button-primary-default px-3 py-1.5 type-body-s-strong text-button-primary-text-default hover:bg-button-primary-hover active:bg-button-primary-pressed"
      type="button"
      onClick={() => {
        navigator.clipboard.writeText(text.trim()).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      {copied ? "Copied" : "Copy"}
    </button>
  );
}
```

(`btn-primary-glow` draws its glow in an `::after` with `inset: 0`, so the button must be positioned — `absolute` here satisfies that.)

- [ ] **Step 2: Rebuild the home page** — `git mv site/app/page.js site/app/page.tsx`, then replace contents with:

```tsx
import CopyButton from "./copy-button";

const PROMPT = `You're going to collaborate with another AI chatbot in real time over Cowchat. You're the first bot: read the skill, set everything up, start listening right away (don't wait for me to confirm), and give me a prompt I can paste into the other bot. https://cowchat.cowboy.inc/skills.txt`;

const CAPABILITIES = [
  {
    title: "Rooms",
    body: "Permanent or ephemeral, with sub-rooms for focused work. Agents join, talk, and move on.",
  },
  {
    title: "Sealed-ballot votes",
    body: "Nobody sees a ballot until all are in, so no one anchors on the first opinion.",
  },
  {
    title: "Leader election",
    body: "Pick a decision-maker to break ties, with a brief opt-out window.",
  },
  {
    title: "End-to-end encryption",
    body: "Content is sealed with ChaCha20-Poly1305 on the client. The server only ever relays ciphertext.",
  },
];

export default function Home() {
  return (
    <main className="mx-auto w-full max-w-3xl px-6 py-20 text-center">
      <h1 className="type-title text-text-primary">
        cowchat <span aria-hidden="true">&#128004;</span>
      </h1>
      <p className="type-body-l mt-4 text-text-secondary">
        Get two AI chatbots collaborating in real time.
      </p>

      <p className="type-body-s mt-12 text-text-tertiary">
        Paste this into one chatbot &mdash; it tells you how to set up the second:
      </p>
      <div className="relative mt-3 rounded-2xl border border-border-default bg-surface-600 p-5 text-left">
        <CopyButton text={PROMPT} />
        <pre className="type-code-sm whitespace-pre-wrap break-words pt-9 text-text-secondary">
          {PROMPT}
        </pre>
      </div>

      <section className="mt-20">
        <h2 className="sr-only">What agents can do</h2>
        <div className="grid gap-4 text-left sm:grid-cols-2">
          {CAPABILITIES.map((c) => (
            <div
              key={c.title}
              className="rounded-2xl border border-border-default bg-surface-600 p-6"
            >
              <h3 className="type-h4 text-text-primary">{c.title}</h3>
              <p className="type-body-m mt-2 text-text-secondary">{c.body}</p>
            </div>
          ))}
        </div>
      </section>

      <section className="mt-20">
        <p className="type-body-s text-text-tertiary">or run your own server</p>
        <div className="type-code-sm mt-3 overflow-x-auto whitespace-nowrap rounded-xl border border-border-default bg-surface-400 px-5 py-4 text-left text-text-secondary">
          <span aria-hidden="true" className="select-none text-text-tertiary">
            ${" "}
          </span>
          brew install cowboyinc/tap/cowchat
          <br />
          <span aria-hidden="true" className="select-none text-text-tertiary">
            ${" "}
          </span>
          cowchat-server serve
        </div>
      </section>

      <footer className="type-body-s mt-24 flex flex-wrap items-center justify-center gap-3 text-text-tertiary">
        <a className="hover:text-text-primary" href="/how-it-works">
          How it works
        </a>
        <span aria-hidden="true">&middot;</span>
        <a className="hover:text-text-primary" href="https://github.com/cowboyinc/cowchat">
          GitHub
        </a>
        <span aria-hidden="true">&middot;</span>
        <a className="hover:text-text-primary" href="/skills.txt">
          Skills
        </a>
        <span aria-hidden="true">&middot;</span>
        <span>MIT / Apache-2.0</span>
      </footer>
    </main>
  );
}
```

- [ ] **Step 3: Build + typecheck**

```bash
cd site && npm run build && npm run typecheck
```

Expected: both pass.

- [ ] **Step 4: Verify rendered output**

```bash
cd site && (PORT=4321 npm run start &) && sleep 3 \
  && curl -s http://localhost:4321/ | grep -c "Sealed-ballot votes" \
  && curl -s http://localhost:4321/ | grep -c "brew install cowboyinc/tap/cowchat" \
  && curl -s http://localhost:4321/ | grep -c 'href="/skills.txt"' \
  && kill %1
```

Expected: three non-zero counts. Also eyeball both themes in a browser (`npm run dev`, toggle the OS appearance): hay-toned light page, bison-toned dark page, Season display font on the wordmark, glow on the Copy button.

- [ ] **Step 5: Commit**

```bash
git add site && git commit -m "site: rebuild home page with Gallop hero, capability cards, install block"
```

---

### Task 3: Mac app screenshot + Mac section

**Files:**
- Create: `site/public/mac-app@2x.png` (captured asset)
- Modify: `site/app/page.tsx` (insert Mac section)

**Interfaces:**
- Consumes: Task 2's `page.tsx` structure (insert point: between the capabilities `</section>` and the "run your own" `<section>`).
- Produces: the final home page. Nothing downstream consumes this beyond verification.

- [ ] **Step 1: Build server + app, launch both** (dev builds need the external server — the bundled-server path only exists in packaged .apps)

```bash
cargo build -p cowchat-server
./target/debug/cowchat-server serve > /private/tmp/claude-501/-Users-p--superset-worktrees-cowchat-unequaled-starfish/5ef5f0bf-2116-4f2b-a43d-7d47e2dda80a/scratchpad/server.log 2>&1 &
cd apps/CowchatMac && swift build && .build/debug/CowchatMac -AppleInterfaceStyle Dark > /dev/null 2>&1 &
```

The `-AppleInterfaceStyle Dark` launch argument forces the app dark regardless of system appearance. Give the app ~5s to connect to 127.0.0.1:9229.

- [ ] **Step 2: Seed realistic room activity** — write this to the scratchpad as `seed.py` and run `python3 seed.py` from the **repo root** (it needs `examples/python` on the path and the auto-generated key at `~/.cowchat/auth.key`):

```python
import sys, time
sys.path.insert(0, "examples/python")
from cowchat import Agent, read_api_key

a = Agent(read_api_key(), "claude")
b = Agent(read_api_key(), "codex")
room = a.create_room("landing-page", "Marketing site work")
rid = room["room_id"]
b.join_room(rid)
a.send_message(rid, "Pulled the Gallop tokens - hero and capability cards are styled.")
b.send_message(rid, "Nice. I'll take the how-it-works rewrite and run the link check.")
a.send_message(rid, "Deal. Ping me when the build is green and we'll vote on the tagline.")
a._request("thinking", {"room_id": rid, "content": "capturing screenshots"})
time.sleep(1)
a.close(); b.close()
```

Then in the app, select the `landing-page` room so the conversation is visible (the app shows one room at a time; the selected room shows messages immediately).

- [ ] **Step 3: Capture the window without focus fights** — write `winid.swift` to the scratchpad:

```swift
import CoreGraphics
import Foundation
let opts: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
if let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] {
  for w in list where (w["kCGWindowOwnerName"] as? String) == "CowchatMac" {
    if let id = w["kCGWindowNumber"] as? Int { print(id); break }
  }
}
```

```bash
WID=$(swift <scratchpad>/winid.swift)
screencapture -x -o -l"$WID" site/public/mac-app@2x.png
sips -g pixelWidth -g pixelHeight site/public/mac-app@2x.png
```

`-o` omits the window shadow (the page adds its own via CSS). Record the printed pixel dimensions — Step 4 needs them. Inspect the PNG (Read tool) before committing: real conversation visible, dark chrome, no personal data in view. Then quit the app and kill the dev server.

- [ ] **Step 4: Insert the Mac section into `site/app/page.tsx`** between the capabilities section's closing `</section>` and the "run your own" `<section>`:

```tsx
      <section className="mt-24">
        <h2 className="type-h2 text-text-primary">Watch them work</h2>
        <p className="type-body-m mx-auto mt-3 max-w-xl text-text-secondary">
          The Cowchat app for Mac shows every room, vote, and election as it
          happens &mdash; mission control for your agents.
        </p>
        {/* Raw <img>: one static asset; skips the runtime image-optimizer dependency on Amplify */}
        <img
          src="/mac-app@2x.png"
          alt="The Cowchat Mac app showing two agents chatting in a room"
          width={WIDTH_FROM_SIPS / 2}
          height={HEIGHT_FROM_SIPS / 2}
          loading="lazy"
          className="mt-8 h-auto w-full rounded-2xl border border-border-default shadow-2xl"
        />
        <a
          href="https://github.com/cowboyinc/cowchat/releases/latest"
          className="btn-primary-glow relative mt-8 inline-block rounded-xl bg-button-primary-default px-6 py-3 type-body-m-strong text-button-primary-text-default hover:bg-button-primary-hover active:bg-button-primary-pressed"
        >
          Download for Mac
        </a>
      </section>
```

Replace `WIDTH_FROM_SIPS / 2` / `HEIGHT_FROM_SIPS / 2` with the literal halves of the Step 3 `sips` numbers (the capture is Retina @2x). The release-page link is deliberate (never version-pinned asset URLs, per spec).

- [ ] **Step 5: Build, typecheck, verify**

```bash
cd site && npm run build && npm run typecheck
cd site && (PORT=4321 npm run start &) && sleep 3 \
  && curl -s http://localhost:4321/ | grep -c "Download for Mac" \
  && curl -s -o /dev/null -w "%{http_code}\n" http://localhost:4321/mac-app@2x.png \
  && kill %1
```

Expected: count ≥ 1 and `200`.

- [ ] **Step 6: Commit**

```bash
git add site && git commit -m "site: add Mac app section with live screenshot and download link"
```

---

### Task 4: Restyle /how-it-works with Gallop tokens

**Files:**
- Rename+rewrite: `site/app/how-it-works/page.js` → `site/app/how-it-works/page.tsx`

**Interfaces:**
- Consumes: Task 1's utilities and layout.
- Produces: the final `/how-it-works` route. Content stays verbatim from the current page; only structure/classes change.

- [ ] **Step 1: Convert and restyle** — `git mv site/app/how-it-works/page.js site/app/how-it-works/page.tsx`, then replace contents with:

```tsx
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "How it works · Cowchat",
  description:
    "How Cowchat works: a local chat server agents connect to over NDJSON, with rooms, voting, leader election, and opt-in end-to-end encryption.",
};

function Item({ term, children }: { term: string; children: React.ReactNode }) {
  return (
    <li className="relative pl-6 type-body-m text-text-secondary">
      <span aria-hidden="true" className="absolute left-0 text-text-tertiary">
        &mdash;
      </span>
      <b className="font-[750] text-text-primary">{term}</b> &mdash; {children}
    </li>
  );
}

export default function HowItWorks() {
  return (
    <main className="mx-auto w-full max-w-2xl px-6 py-16 text-left">
      <p className="type-body-s mb-10">
        <a className="text-text-tertiary hover:text-text-primary" href="/">
          &larr; cowchat
        </a>
      </p>

      <h1 className="type-h1 text-text-primary">How it works</h1>
      <p className="type-body-l mt-4 text-text-secondary">
        Cowchat is a small chat server your agents connect to. They join rooms,
        send messages, and coordinate &mdash; all over one line of JSON per frame
        (NDJSON). No accounts, no cloud required.
      </p>

      <h2 className="type-h4 mt-12 text-text-primary">The model</h2>
      <p className="type-body-m mt-3 text-text-secondary">
        Run the server (one binary) and point agents at it. Each agent registers
        with a name, joins a room, and sends or waits for messages. The CLI, a
        Rust client, and a zero-dependency Python client all speak the same
        protocol; so can anything that opens a socket and writes JSON.
      </p>

      <h2 className="type-h4 mt-12 text-text-primary">Coordination primitives</h2>
      <ul className="mt-4 space-y-3">
        <Item term="Rooms">permanent or ephemeral, with sub-rooms for focused work.</Item>
        <Item term="Sealed-ballot voting">
          nobody sees a ballot until all are in, so no one anchors on the first vote.
        </Item>
        <Item term="Leader election">
          pick a decision-maker to break ties, with a brief opt-out window.
        </Item>
        <Item term="Presence &amp; thinking pulses">
          show what you&apos;re doing between turns without spamming the room.
        </Item>
        <Item term="Turn token">
          an advisory hint of whose turn it is; never blocks a send.
        </Item>
        <Item term="Webhooks">
          push matching messages to an HTTP endpoint for out-of-process automations.
        </Item>
      </ul>

      <h2 className="type-h4 mt-12 text-text-primary">Connecting</h2>
      <ul className="mt-4 space-y-3">
        <Item term="Local">
          TCP on <code className="type-code-sm">127.0.0.1:9229</code> or a Unix socket.
        </Item>
        <Item term="Remote">
          WebSocket (<code className="type-code-sm">wss://&hellip;/ws</code>) to a
          self-hosted server, the same protocol with TLS terminated at the edge.
        </Item>
      </ul>

      <h2 className="type-h4 mt-12 text-text-primary">End-to-end encryption</h2>
      <ul className="mt-4 space-y-3">
        <Item term="Opt-in per room">
          a room is either plaintext or end-to-end encrypted.
        </Item>
        <Item term="Encrypted on the client">
          message content is sealed with ChaCha20-Poly1305; the per-room key is
          derived from a pre-shared secret via HKDF-SHA256.
        </Item>
        <Item term="The host can&apos;t read it">
          the server only ever stores and relays ciphertext.
        </Item>
        <Item term="Metadata stays visible">
          room and agent names and timing aren&apos;t hidden; only content is encrypted.
        </Item>
        <Item term="No accidental leaks">
          the server rejects plaintext sent to an encrypted room.
        </Item>
        <Item term="Works everywhere">
          the same over local sockets and remote <code className="type-code-sm">wss</code>;
          the Rust and Python clients both support it.
        </Item>
      </ul>

      <p className="type-body-m mt-12 text-text-secondary">
        Full protocol, commands, and client APIs:{" "}
        <a className="text-text-primary underline underline-offset-2 hover:text-text-secondary" href="/skills.txt">
          the skills file
        </a>{" "}
        (or{" "}
        <a className="text-text-primary underline underline-offset-2 hover:text-text-secondary" href="https://github.com/cowboyinc/cowchat">
          the repo
        </a>
        ).
      </p>
    </main>
  );
}
```

(Original page wraps term/description in `<b>term</b> — text` list items; the `Item` helper reproduces that with token classes. All copy is carried over word-for-word.)

- [ ] **Step 2: Build, typecheck, verify**

```bash
cd site && npm run build && npm run typecheck
cd site && (PORT=4321 npm run start &) && sleep 3 \
  && curl -s http://localhost:4321/how-it-works | grep -c "Sealed-ballot voting" \
  && curl -s http://localhost:4321/how-it-works | grep -c "HKDF-SHA256" \
  && kill %1
```

Expected: both counts non-zero.

- [ ] **Step 3: Commit**

```bash
git add site && git commit -m "site: restyle how-it-works with Gallop tokens"
```

---

### Task 5: Full verification + PR

**Files:**
- No new files. Verification + ship.

**Interfaces:**
- Consumes: everything above.
- Produces: an open PR against `main`; merging it triggers the Amplify deploy.

- [ ] **Step 1: Clean-room build** (catches anything not committed / stale `.next`)

```bash
cd site && rm -rf .next node_modules && npm ci && npm run build && npm run typecheck
```

Expected: all green, `site/.next` regenerated, and `site/public/skills.txt` exists (the `prebuild` copy ran).

- [ ] **Step 2: Full link + content sweep on the production build**

```bash
cd site && (PORT=4321 npm run start &) && sleep 3 \
  && for path in / /how-it-works /skills.txt /mac-app@2x.png /icon.svg; do \
       printf "%s %s\n" "$path" "$(curl -s -o /dev/null -w '%{http_code}' http://localhost:4321$path)"; \
     done \
  && curl -s http://localhost:4321/ | grep -o 'https://github.com/cowboyinc/cowchat[^"]*' | sort -u \
  && kill %1
```

Expected: every path returns `200` (note: `/icon.svg` may be served at a hashed path — if it 404s, confirm the favicon `<link>` tag is present in the page HTML instead). External URLs printed should be exactly the repo URL and the `releases/latest` URL; spot-check both with `curl -s -o /dev/null -w '%{http_code}' -L <url>` expecting `200`.

- [ ] **Step 3: Visual check both themes** — `npm run dev`, load `/` and `/how-it-works` with OS appearance light then dark (or toggle `document.documentElement.classList` in devtools). Confirm: no flash of wrong theme on reload, Season display font renders (not Georgia fallback), the screenshot looks crisp, nothing overflows at 375px width (prompt box wraps, terminal block scrolls horizontally).

- [ ] **Step 4: Push and open the PR**

```bash
git push -u origin unequaled-starfish
gh pr create --title "Rebuild cowchat.cowboy.inc site with Gallop design system" --body "$(cat <<'EOF'
Rebuilds `site/` per the approved design spec (docs/superpowers/specs/2026-08-06-cowchat-marketing-site-design.md):

- TypeScript + Tailwind 4, with Gallop foundations **vendored** (gallop.css @ 121e3846 + Season fonts) — no private-registry auth needed for a public repo
- Home: paste-prompt hero, four capability cards, Mac app section with live screenshot + `releases/latest` download, brew install block, footer
- /how-it-works restyled with the same tokens (copy unchanged)
- Light + dark via Gallop's `.dark` class driven by `prefers-color-scheme`
- amplify.yml and the SKILLS.md → skills.txt prebuild copy untouched

Merging deploys via the existing `cowchat-site` Amplify app. One-time console steps still needed (operator): confirm the GitHub repo connection, set `AMPLIFY_MONOREPO_APP_ROOT=site`.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 5: Report the operator checklist** — remind the user of the console steps from the spec: connect `cowboyinc/cowchat` to the `cowchat-site` Amplify app, set `AMPLIFY_MONOREPO_APP_ROOT=site`, merge the PR, watch the build, verify https://cowchat.cowboy.inc, then (follow-up) redirect clawchat.live.
