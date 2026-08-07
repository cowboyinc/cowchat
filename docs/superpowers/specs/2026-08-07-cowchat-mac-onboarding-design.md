# Cowchat Mac onboarding redesign

**Date:** 2026-08-07
**Status:** Approved in sections by Patrick 2026-08-07; adversarial spec review applied (3 lenses, 30 findings)
**Scope:** `apps/CowchatMac`, plus two site changes (`PROMPT` string and the step-2 lead-in copy in `site/app/page.tsx`)

## Problem

The current first-run chain shows two different copy-prompt walls back to back:
the "Howdy… Welcome to Cowchat!" screen carries a generic prompt (no room name)
behind a copy-consent sheet, completion force-opens the Create Room sheet, and
the new room lands on a second full-window prompt screen (`RoomSetupView`) with
a different, room-specific prompt. A user meets a wall of prompt text before
they have any model of the product, and if they paste the generic prompt the
bot has no room to join and improvises.

Two structural bugs compound it:

- The setup-screen state persists across launches and launch-time room
  selection prefers it (`ChatStore.connect()`), so a user whose room never got
  a collaborator reopens the app onto the full-window "Paste this prompt"
  screen on every launch.
- Connect and server-spawn failures surface only as the sidebar footer status
  label (failure detail hidden in a hover tooltip); the `ContentView` alert
  fires only on `errorMessage`, which the connect path never sets. During
  onboarding even the footer is invisible.

Direction (Patrick): no forced onboarding. One explanatory splash, then the
app guides through its own empty states. Matches the wireframe intent
(Figma `YAjrzlTuDU1q6R79Jpmzil`: welcome `33:6189`, room empty state `34:650`).

## Glossary: the two setup mechanisms

The current code has two parallel room-ID sets whose names differ between the
in-memory property and the persisted UserDefaults key. This spec always names
both forms:

| In-memory (`ChatStore`) | Persisted key | Purpose today | Fate |
|---|---|---|---|
| `setupRoomIDs` | `CowchatMac.pendingSetupRoomIDs` | rooms awaiting first collaborator; drives the 2s readiness poll and the ready toast | **Kept** |
| `roomSetupScreenIDs` | `CowchatMac.pendingSetupScreenRoomIDs` | rooms whose full-window `RoomSetupView` takeover still shows; drives the launch-selection preference | **Deleted** |

Room-name matches in this spec ("lobby", "General") are always
case-insensitive on the app side. (The server's name-uniqueness check is
case-sensitive; see §7 for the one hazard that creates.)

## Goals

1. First launch explains before it asks; one click from splash to a live app.
2. Exactly one connect prompt concept — room-specific — surfaced where the
   user is looking, with live feedback while waiting for the first agent.
3. The reopen-onto-setup trap is gone.
4. A site-first user's bot and an app-first user's bot converge on the same
   public "General" room.
5. Connection failures are visible and carry their own fix guidance.

## Non-goals

- Multi-room membership / per-room presence attribution (spec'd out of the
  Dash-alignment work; separate project).
- Dark theme; vote/election UI surfacing; site screenshot recapture;
  clawchat.live redirect; changes to `skills.txt` publishing.
- Server-side changes. None are needed: the default room is app-created.
- Guided tours, checklists, coach marks.

## Design

### 1. Splash (rebuild of `CowchatOnboardingView`)

One full-window screen, shown once. Version-gate machinery is unchanged:
`CowchatOnboarding.currentVersion` stays **1**, `completedVersionKey` and
`migrateExistingUser` untouched, Settings → "Show onboarding again" still
resets the version to 0. Existing users who completed v1 (or were migrated)
never see the new splash.

Centered column:

- App icon (`CowchatAppDelegate.applicationIcon()`), same hero treatment.
- H4 (Season Mix): **"Howdy… Welcome to Cowchat!"**
- Body (settled copy, unchanged): *"Cowchat is a small chat server your
  agents connect to. They join rooms, send messages, and collaborate in real
  time."*
- Three-step row — the same GallopIcon trio the connect state uses
  (list → arrow → sparkles), with captions:
  1. **Copy the prompt from your first room**
  2. **Paste it into an AI chatbot**
  3. **Watch your agents work together live**
- Primary capsule (`CapsulePillButtonStyle`): **"Get started"** →
  `onComplete` sets the completed version. Nothing else: the auto-presented
  Create Room sheet is removed.

Deleted with this rebuild: `collaborationPrompt`, the copy-consent sheet
(`isCopyExplanationPresented`), `hasCopiedPrompt`, and the
Skip-for-now/Continue button duality. All custom controls keep
`.macAccessibleAction`.

⌘N / New Room behavior is unchanged: still disabled while the splash shows;
rooms created afterward land in the §3 state machine.

Connection startup keeps running behind the splash exactly as today. Failure
visibility is handled by §5 — the splash itself shows no connection state,
but it is one click deep, so the user reaches the surfaces that do.

### 2. Default room "General"

Auto-creation runs **only for fresh installs** — installs where
`migrateExistingUser` did NOT stamp the user (i.e., the user actually saw the
splash). Migrated existing users never get an unrequested room; for them the
guard flag is simply set on first evaluation. This keeps the promise that
existing users only get better empty states, including long-standing
lobby-only users.

Trigger: first successful connect **on the Local profile** once the initial
room list has loaded. Evaluate this ordered chain exactly once per launch:

1. Guard flag `CowchatMac.didCreateDefaultRoom` (a single **unscoped**
   UserDefaults key — only the Local profile ever consults it) is set →
   do nothing.
2. User is a migrated existing user → set the flag, done.
3. A visible room named "General" exists → select it, set the flag.
4. Any visible room other than the seeded `lobby` exists (CLI-first user) →
   set the flag; normal launch selection applies (§4).
5. Otherwise → create a **public, permanent** room named **"General"**,
   description **"Where your agents meet and work together"** (distinct from
   the lobby's seeded description; final copy per Patrick's review), select
   it, set the flag.

Failure handling for step 5:

- `RoomNameTaken` (a room named General exists but is not visible to the app
  — e.g. a keyless bot created it private before first app launch): **set the
  flag anyway** and fall back to normal selection. Retrying can never
  succeed, so no retry-every-launch loop. The site prompt's
  "create it as a public room" clause (§6) makes this case rare.
- Transient failure (server/network error): leave the flag unset; the next
  launch retries once. No in-session retry loop.

The room is never resurrected: deleting General later does nothing (flag
stays set). The lobby dashboard's New Room card is the route back.

A created General enters `setupRoomIDs` like any created room, so the 2s
readiness poll and the ready toast work unchanged. If "Get started" is
clicked before the connection is up, the main window shows the §5 states and
General is created and selected when the connect lands.

### 3. Room empty state (replaces the `RoomSetupView` takeover)

`RoomSetupView`, `roomSetupScreenIDs`, and `completeRoomSetup` are deleted.
What a selected, open, non-lobby room renders is **purely derived** from live
signals — connection status, currently connected members other than self,
loaded messages. `markSetupRoomReadyIfNeeded` no longer gates rendering; it
only removes rooms from `setupRoomIDs` (ending their polling) and drives the
ready toast.

Decision table, evaluated in order. "Loading" means an in-flight history
fetch on an established connection only:

| Condition (in order) | Renders |
|---|---|
| Room named `lobby` | Lobby dashboard (unchanged) |
| Not connected (connecting/offline/failed) and no messages loaded this session | **Connect state** with the matching status-line variant |
| History fetch in flight | Existing loading treatment (unchanged) |
| Loaded messages exist | Normal chat (unchanged) |
| No messages, no other members connected | **Connect state** ("Connected" variant) |
| No messages, other members present | Quiet-room state (unchanged from PR #3: "This room is quiet" / "Bring an agent in with the connect prompt, or open the composer and say hello.") |

Consequences to implement knowingly:

- A room with server-side history renders the connect state while
  disconnected (history can't load without a connection; `select()` clears
  messages). Honest, and the Offline status line explains why.
- An agent that joins and leaves before any message returns the room to the
  connect state. Correct: nobody is there and nothing was said.
- **Member-truth refresh:** while a room renders the connect state it
  participates in the 2s member poll regardless of `setupRoomIDs`
  membership, and members are refetched on reconnect. This closes the
  current hole where a failed `listAgents` during `select()` leaves
  `roomMembers` empty and the UI would claim "waiting for your first agent"
  next to a live agent.

**Connect state**, inside normal chat chrome (sidebar visible, header shows
room name; header caption **"No agents here yet"**):

- Icon row: list → arrow → sparkles (as today).
- H5: **"Paste this prompt into an AI chatbot"**.
- Prompt card: `store.connectPrompt(for: room)` (`ChatStore.connectPromptText`
  remains the single source of truth) with **Copy → Copied**. Copy writes to
  the pasteboard immediately — no consent sheet.
- Live status line with a subtly pulsing dot, keyed off connection status:
  - Connected: **"Connected — waiting for your first agent…"**
  - Connecting: **"Connecting…"**
  - Offline/failed: **"Offline — reconnect before pasting."** (existing
    reconnect affordances unchanged)
- Slow-join reassurance: after ~60s continuously in the Connected variant,
  a secondary tertiary line appears: **"First join can take a few minutes
  while your agent installs cowchat."** Optional (implementer judgment, only
  if the signal is already available from the lobby's available-agents
  source): when an agent is online on the server but not in this room, show
  **"An agent is online but hasn't joined this room — check the room name in
  your paste."**
- Tertiary link: **"Not connecting?"** (§5).

**Bridge to the second agent:** when a room flips from the connect state to
live chat, show a one-time transient hint (per room, session-scoped, glass
style of `RoomReadyNotice`): **"Add more agents anytime — Copy connect prompt
in the ⋯ menu."** The ready toast still fires when the user is looking
elsewhere. This pays off the splash's plural promise; without it the flow
dead-ends at one agent.

The lobby dashboard stops excluding `setupRoomIDs` rooms from its grid — that
filter existed to support the takeover flow this spec deletes, and it would
otherwise hide General from the dashboard indefinitely while agentless.

### 4. Launch selection + state cleanup

- Initial selection in `connect()` becomes: room named `lobby` → first room.
  The setup-screen preference is removed. (§2 step 5 overrides this on the
  one launch where General is created: General is selected.)
- One-time cleanup at `ChatStore` init: delete the
  `CowchatMac.pendingSetupScreenRoomIDs` key (unscoped and the current
  profile's scoped variant). `RoomLocalPreferences` drops the property.
- Cheap guard against a reachable trap: `createRoom` and `renameRoom` reject
  names case-insensitively equal to "lobby" (the server's case-sensitive
  uniqueness would otherwise allow a second room named "Lobby", which the
  app's case-insensitive special-case then treats as a second un-renamable
  dashboard).

### 5. Failure visibility + "Not connecting?" guidance

The stale-brew-server failure (an old `cowchat-server` squatting port 9229 —
the app connects to whatever answers and never spawns its bundled helper)
manifests as a *failed connection with zero rooms*. Guidance must therefore
live on the failure surfaces, not only inside a room's connect state:

- **Latched server-spawn failures set `store.errorMessage`** so the existing
  `ContentView` alert actually fires (small behavior change; today the
  connect path only sets footer status). Ordinary retrying connect failures
  keep the footer treatment.
- **Connection-failed empty state:** when the connection is failed and no
  room can be shown (fresh user, empty room list), the main pane's
  `EmptyChatView` gains a failed variant: status line, Reconnect button, and
  the troubleshooting content below, inline.
- **"Not connecting?" popover** (~360pt), opened from the tertiary link in
  the room connect state and from the failed empty state. Three items, one
  line each, commands copyable:
  1. **Older server on port 9229.** "If you've installed cowchat with
     Homebrew before, an older server may be answering. Run
     `brew upgrade cowchat`, then quit and reopen Cowchat."
  2. **Private rooms need the key.** "Agents must connect with the key at
     `~/.cowchat/auth.key` to see a private room — or make the room public."
  3. **Your agent needs internet.** "Your agent fetches the Cowchat skill
     from cowchat.cowboy.inc — it needs internet even though Cowchat itself
     is local."

No further troubleshooting UI. Final phrasing subject to Patrick's live copy
review.

### 6. One prompt everywhere (site change)

`site/app/page.tsx` `PROMPT` becomes the room-naming variant with the
chaining sentence kept (bot #1 still recruits bot #2 — approved 2026-08-07)
and a create-if-missing clause so a site-first bot creates General **public**
(otherwise a keyless bot's private General is invisible to the app, blocks
the name, and re-creates the exact silent failure §2 exists to kill). The
string below is copy-exact including curly quotes:

> You're going to collaborate with another AI chatbot in real time over
> Cowchat. Read the Cowchat skill, connect to the local server, join the
> exact room “General” (create it as a public room if it doesn't exist),
> start listening right away (don't wait for me to confirm), and give me a
> prompt I can paste into the other bot. https://cowchat.cowboy.inc/skills.txt

Step-2 lead-in copy becomes (draft, Patrick reviews): *"Paste this into your
first chatbot. It reads the skills file, joins your General room, and prints
the prompt for your second agent:"*

The in-app prompt is unchanged and chain-free (you're standing next to the
Copy button; the §3 bridge hint covers recruiting more agents). With
`collaborationPrompt` deleted, the app and the site each carry exactly one
prompt, and both name the room.

## Copy summary (all final copy subject to Patrick's live review)

| Surface | Copy | Status |
|---|---|---|
| Splash H4 | Howdy… Welcome to Cowchat! | settled |
| Splash body | Cowchat is a small chat server your agents connect to. They join rooms, send messages, and collaborate in real time. | settled |
| Splash steps | Copy the prompt from your first room · Paste it into an AI chatbot · Watch your agents work together live | new |
| Splash button | Get started | new |
| General description | Where your agents meet and work together | new |
| Connect-state header caption | No agents here yet | new |
| Connect-state H5 | Paste this prompt into an AI chatbot | changed ("one" → "an") |
| Prompt card buttons | Copy → Copied | settled |
| Status line | Connected — waiting for your first agent… / Connecting… / Offline — reconnect before pasting. | new |
| Slow-join line | First join can take a few minutes while your agent installs cowchat. | new |
| Escape hatch link | Not connecting? | new |
| Popover items | see §5 (three items) | new |
| Second-agent hint | Add more agents anytime — Copy connect prompt in the ⋯ menu. | new |
| Quiet room | This room is quiet / Bring an agent in with the connect prompt, or open the composer and say hello. | pre-approved (PR #3), unchanged |
| Site PROMPT | see §6 blockquote | new |
| Site step-2 lead-in | Paste this into your first chatbot. It reads the skills file, joins your General room, and prints the prompt for your second agent: | draft |

## Error handling

- Server spawn failure: latched failures now set `errorMessage` → alert
  (§5); ordinary connect failures keep footer status + §5 surfaces.
- General creation failures: per §2 (RoomNameTaken sets the flag; transient
  errors retry once next launch).
- Offline at "Get started": §3 table row 2 / §5 failed empty state; General
  deferred to connect.
- Settings "Show onboarding again": replays just the splash; the §2 guard
  flag prevents a second auto-create.

## Testing

Unit (in `CowchatMacTests`):

- Default-room chain (§2): creates once; selects existing visible "General";
  skips for migrated existing users (including the lobby-only existing
  user); skips when other rooms exist; RoomNameTaken sets the flag; never
  recreates after deletion; Local-profile-only.
- Launch selection: no preference for setup rooms (regression for the reopen
  trap).
- §3 decision table, including: disconnected room with server-side history →
  connect state; agent joined-then-left before messages → connect state
  returns; failed `listAgents` then reconnect → members refetched.
- One-time `pendingSetupScreenRoomIDs` cleanup.
- "lobby" name rejection in `createRoom`/`renameRoom`.
- Onboarding gate/migration tests updated for the rebuilt splash (consent
  sheet and `collaborationPrompt` assertions removed; migration semantics
  unchanged and still locked).
- Rewrite/delete tests that reference deleted APIs: `RoomTransitionTests`'
  setup-screen persistence and `completeRoomSetup` flows,
  `RoomLocalPreferencesTests`' `pendingSetupScreenRoomIDs` round-trip.
- `ConnectPromptTests` unchanged for the app prompt.

Manual (dev loop, per memory: external workspace server, Python driver
agent, window-ID screenshots):

- Fresh-HOME first run: splash → Get started → General connect state →
  paste prompt into a real agent → watch the in-place flip to live chat
  (no toast) and the one-time second-agent hint.
- Repeat with the lobby selected while the agent joins → verify the ready
  toast fires.
- Relaunch with an agentless created room: verify landing on lobby, with the
  room visible in the dashboard grid.
- Site `PROMPT` string cross-check against §6 (the site has no test
  harness).
- Stale-server simulation: occupy 9229 with an incompatible server → verify
  the failed empty state shows the troubleshooting content.
