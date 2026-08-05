# Cowchat Mac — Dash/Cowboy design alignment

**Date:** 2026-08-05 · **Status:** Approved by Patrick (design dialogue), pending Codex review
**Scope:** `apps/CowchatMac` only. No server, protocol, CLI, or web changes.

## Context

Cowchat Mac's UI predates the finalized Cowboy Desktop design. The user wants it aligned with the **Cowboy macOS app as the core source of truth** — its implementation at `~/Github/macos` and the Dash 1.0 Figma (`xhgBlY7S0O1zWXXZUhcWYX`). The cowchat wireframe file (`YAjrzlTuDU1q6R79Jpmzil`) remains the **layout** reference; its hi-fi "Full Dark Mode" frames are explicitly **not** the visual target (dark-only, glass-heavy). Precedence follows `~/Github/macos/docs/dash-design-system.md`: **Dash outranks Gallop**, and the macos repo's `Gallop/Generated/GallopTokens.swift` carries the Dash-reconciled values (gallop repo `main` is stale — do not regenerate from it).

Requirements (from Patrick, 2026-08-05):

1. Visual target = Cowboy Mac app + Dash Figma; dark mock is layout reference only.
2. Sync cowchat's theme to the Dash-correct token layer.
3. Remove the "All rooms / Active" scope switcher.
4. Remove the pinned-rooms concept (sidebar section, context menus, chat-nav menu item).
5. Remove date group headers (Today/Yesterday/…) — flat, iMessage-like room list.
6. Use design-system icons (search, settings, ellipsis, trash, etc.).
7. Match the Cowboy Mac app design wherever possible.
8. Add working/unread room states (running-or-unread vs. stopped-and-read).
9. Add "Open in Claude" / "Open in Codex" hover buttons next to agent names in chat.

Decisions made with Patrick during design:
- **Bundle Season fonts** (exact typography match; fonts ship in the public app).
- **Vendor the design layer from the macos repo** (not regenerate from gallop).
- **Adopt native window chrome** (standard titlebar + toolbar; drop the rounded-card shell).

## Non-goals

- No dark-mode-mock glass treatments beyond what the Cowboy app itself uses.
- No sidebar rail (collapsed 76pt) mode; cowchat keeps full show/hide.
- No new server features (presence protocol changes only if the working-state verification below demands a fallback, and then client-side only).
- No redesign of votes/elections/settings *content* — only restyling of existing surfaces.

## 1. Design foundation — vendored Gallop layer

New group `apps/CowchatMac/Sources/CowchatMac/Gallop/`, vendored and adapted from `~/Github/macos`:

| File | Source | Adaptation |
|---|---|---|
| `GallopTokens.swift` | `macos/Gallop/Generated/GallopTokens.swift` | Keep values byte-identical (Palette ramps incl. `lantern`, `cactus`; ~163 `SemanticColor` tokens). Drop `public`. Keep the "do not regenerate from gallop main" header. |
| `HexColor.swift` | `macos/Gallop/HexColor.swift` | As-is. |
| `GallopTypography.swift` | `macos/Gallop/Typography.swift` | `GallopTextStyle` + semantic roles + `GallopFontProvider` protocol + `SystemFontProvider` fallback. |
| `SeasonFonts.swift` | `macos/Gallop/SeasonFonts.swift` | Resource-bundle probing rewritten for cowchat's bundle (`CowchatMac_CowchatMac.bundle`); registers `Fonts/SeasonSansUprightsVF.ttf` + `Fonts/SeasonMixUprightsVF.ttf` via `CTFontManagerRegisterFontsForURL` (process scope, already-registered counts as success). **Never traps on a missing bundle** — falls back to `SystemFontProvider` (COW-2689 lesson). |
| `GallopIcon.swift` | `macos/Gallop/GallopIcon.swift` | Same mechanism: SVGs at `Icons/svg/<rawValue>.svg`, loaded as template `NSImage`, cached, `nil` on miss with per-callsite SF Symbol fallback. |

Resources added under `Sources/CowchatMac/Resources/`:
- `Fonts/` — the two Season variable TTFs, copied from the macos repo's Gallop resource bundle.
- `Icons/svg/` — SVGs cowchat needs, from `macos` where available, and from `~/Github/gallop/packages/foundations/src/icons/svg/` for the three the mac port lacks (`Ellipsis.svg`, `Trash.svg`, `Dismiss.svg` — renamed to the mac port's kebab-case convention: `ellipsis.svg`, `trash.svg`, `dismiss.svg`).

Initial icon set (rawValue names): `add`, `arrow-up-right`, `chevron-down`, `chevron-right-small`, `chevron-down-extra-small`, `chevron-right-extra-small`, `chevron-up-extra-small`, `copy`, `dismiss`, `edit`, `ellipsis`, `folder`, `lock`, `message`, `search`, `send`, `settings`, `sidebar`, `thinking`, `trash`, `warning`.

API compatibility:
- `.gallopText(_:color:)` view modifier survives with the same call-site shape, now resolving fonts through the active `GallopFontProvider` (Season when registered, system otherwise) and exact line-height metrics the way the cowboy app does.
- `GallopTheme.swift` (the 371-line hand bridge, pinned to stale gallop `121e384`) is **deleted**. The `private typealias GallopColor = GallopTheme.ColorToken` and every `GallopColor.x.color` call site sweep to `SemanticColor.x` (mechanical: `SemanticColor` members are `Color` values directly — the trailing `.color` goes away).
- Type roles keep cowchat's current names (`h4`, `bodyL`, `bodyMStrong`, `caption`, `dataLabel`, `code`, …) mapped onto the vendored `GallopTextStyle` table; `display` family = Season Mix VF, `sans` = Season Sans VF, `mono` = SF Mono (system), matching the cowboy app.

Packaging: `build-app.sh` must copy the SwiftPM-produced resource bundle into `Cowchat.app/Contents/Resources/` (verify the bundle's built name/location during implementation; the font/icon loaders probe `Bundle.main.resourceURL`, `Bundle.main.bundleURL`, and the executable's directory, in that order, like `SeasonFontProvider.resourceBundleCandidates`).

## 2. Shell and window chrome

- `CowchatMacApp`: remove `.windowStyle(.hiddenTitleBar)`. Keep `defaultSize(1080×740)` and min 900×600.
- `ContentView`: remove the 22pt rounded clip, its border overlay, and `.ignoresSafeArea(.top)`. Keep the hand-rolled `HStack(spacing: 0)` shell (the cowboy app deliberately avoids `NavigationSplitView` — same reasoning applies).
- Sidebar: fixed `frame(width: 280)` (divergence from Dash's 240 — see log). Remove the 1px divider `Rectangle`; both panes sit flush on `SemanticColor.surface500`. Sidebar background becomes `surface500` (drop `surfaceGlass500`).
- Native toolbar on the detail pane:
  - `ToolbarItem(placement: .navigation)`: sidebar toggle — GallopIcon `sidebar` (SF `sidebar.left` fallback), `iconTertiary` tint, 36pt circle in the cowboy `SidebarToolbarToggle` treatment (`surface700`'s exact light/dark values), `.buttonStyle(.plain)`, no focus ring.
  - `ToolbarItem(placement: .primaryAction)`: New Room — GallopIcon `edit`, keeps ⌘N.
  - Room-scoped trailing items when a room is selected: the consolidated room menu (§5).
- View-menu command: ⌃⌘S toggles the sidebar (cowboy pattern — nothing binds it once you're off `NavigationSplitView`).
- The sidebar's old top chrome row (hide-sidebar + compose circle buttons) is deleted; the titlebar/toolbar strip replaces it. `ChatRoomView`/`LobbyDashboardView`/`EmptyChatView`'s "show sidebar when hidden" affordance moves to the toolbar toggle (always present).
- Window title: bind `.navigationTitle` to the selected room name (falls back to "Cowchat").

## 3. Sidebar simplification

Removals (code + tests):
- **Scope switcher**: `SidebarScope`, `scopePicker`, `activeCount`, the `scope == .active` branches, `RoomSidebarPresentation.activeRooms(from:excludingCurrentClientFrom:)` and its tests.
- **Pinned**: `pinnedRooms` view, `RoomSidebarPresentation.pinnedRooms/visiblePinnedRooms/roomsForRecencyGroups`, `ChatStore.pinnedRoomIDs/isPinned/togglePinned` + auto-pin-lobby seeding, `RoomLocalPreferences` pinned keys (`pinnedRoomIDs`, `pinnedRoomsInitialized`) and accessors, both context-menu Pin items (sidebar row + chat ellipsis menu), related tests. Stored UserDefaults values are simply orphaned (harmless).
- **Date groups**: `RoomSidebarGroup`, `RoomSidebarPresentation.groups/groupTitle`, the header rendering. Replaced by `RoomSidebarPresentation.sortedByRecency(_:)` — unarchived rooms sorted by `activityDate` descending (stable tiebreak on name), rendered as one flat list.

Kept: Archive section (collapsed row at bottom), search (field + message search), the connection footer.

Row restyle (`RoomRow`): keep anatomy (40pt avatar, title, preview, relative time) with the cowboy state vocabulary — normal: clear; hover: `surface600` fill + `Color.black.opacity(0.08)` 0.5pt inner stroke + shadow `black 4% / r1.5 / y1`; selected: `surface400` fill. **No amber selection, no text-color flip on selection.** Radius 12 continuous (54pt rows are too tall for the cowboy 100pt pill — see log). Title `textPrimary`, preview `textTertiary`, time `dataLabel`/`textTertiary`. Hover drives through an explicit `normal/selected/hover` state enum (selection wins), instantaneous (no animation), matching `SidebarRowPill`.

## 4. Working / unread state

**Model** — new `RoomReadState` (modeled on cowboy `ConversationReadState`, simplified):
- `entries: [roomID: lastSeenEpochMs]`, persisted in `UserDefaults` under a scoped `RoomLocalPreferences` key (JSON-encoded dictionary is fine at cowchat's scale; cap entries, prune on room-list reconcile).
- Seed-on-first-run: stamp every room's current `lastActivity` so nothing is retroactively unread.
- `isUnread(room)`: seeded ∧ room not currently selected ∧ `lastActivity` (ms) > stored entry (missing entry ⇒ unread).
- Mark read: on room selection, and continuously while the room stays selected as messages arrive. Reconcile prunes IDs no longer in the room list.

**Working signal**: a room is *working* when any member's `AgentPresence.status` (lowercased) is `working` or `thinking` — the same predicate `ChatPresencePresentation` already uses. ⚠️ **Verify during implementation:** whether `presence_update` reaches the client for rooms other than the selected one. If presence is selected-room-only, fall back (client-side only) to: working ⇔ latest known message in the room has thinking-type metadata (`ChatMessage.isThinking`) and is recent. Decision + rationale get recorded in the implementation plan with Codex input.

**Visuals** (Dash vocabulary; additions logged):
- Unread row: leading 7pt `nugget500` dot before the title **plus** the Dash-exact title weight bump (base title drops to `bodyS` weight when read; `bodySStrong` when unread). VoiceOver: `accessibilityValue("Unread")`.
- Working row: static GallopIcon `thinking` glyph tinted `buttonPrimaryDefault` in the trailing slot next to the timestamp. No pulsing/rotation — the cowboy app's working vocabulary is static (its only animated busy indicator is a `.mini ProgressView` for mutations).
- Chat header: presence line stays; when active it renders in the cowboy header treatment (`caption` + `nugget600`).
- The above-composer thinking indicator adopts the cowboy live-row recipe: `thinking` icon (16×24, `buttonPrimaryDefault`) + `bodyL` `textTertiary` text.

## 5. Chat pane

- **Open in Claude/Codex** (requirement 9): extract `AgentAvatar`'s name→bundle-ID mapping into `AgentAppResolver` (claude → `com.anthropic.claudefordesktop`, codex → `com.openai.codex`, chatgpt/openai → `com.openai.chat`; resolves app URL + display name via `NSWorkspace`). In agent message rows, a chip appears next to the name on hover using the cowboy reserved-layout pattern: always laid out, `opacity(isHovering ? 1 : 0)`, `.allowsHitTesting(isHovering)`, `.accessibilityHidden(!isHovering)`, `.easeOut(duration: 0.12)`, `.contentShape` extending the hover area; plus an always-available accessibility action so it isn't hover-only for VoiceOver. Chip: `caption` "Open in Claude" + GallopIcon `arrow-up-right`, capsule `surface600` fill + `borderDefault` 1pt stroke, hover `buttonSecondaryHover`. Action: `NSWorkspace.openApplication(at:)`. Hidden entirely when no app resolves.
- **Room menu consolidation**: the chat header's ellipsis menu + standalone trash button merge into one toolbar menu (GallopIcon `ellipsis`): Rename / Archive / Create nested room / Reconnect (when disconnected) / room-type lines, divider, Destroy Room… (destructive, existing confirmation alert). Pin/Unpin is gone (requirement 4).
- **Composer**: field uses the `textfield*` ramp (default/hover/focus fills, `borderDefault`→`borderHover`→`borderFocus`) with hover tracked like the cowboy composer; send button becomes the cowboy circular style — `buttonPrimary` default/hover/pressed/disabled fill ramp, `nugget300` 1pt stroke, GallopIcon `send`, disabled at 0.4 opacity.
- **Message rows**: my-message bubble flattens to `surface400` fill + `borderDefault` hairline (drop the gradient), radius per Dash measurement at implementation time (`get_design_context` on the Dash chat node). Disclosure controls ("Show/Hide full response") swap chevrons to GallopIcon extra-small variants.

## 6. Remaining surfaces (requirement 7 sweep)

- `DashboardRoomCard` / New Room card → `gallopCard` recipe: `surface600` fill, `borderDefault` 1pt, radius 8, padding 16.
- `RoomReadyNotice` → cowboy glass capsule: `.ultraThinMaterial` + `surfaceGlass500` overlay + `surfaceGlassBorderHighlight` 1pt stroke + shadows (black 8%/r2/y1 and 4%/r0/y0.5).
- Onboarding: "Howdy…" title in the display role (Season Mix), buttons as capsule `buttonPrimary`/`buttonSecondary` ramps (cowboy `AuthPillButtonStyle` recipe: 12pt/20pt padding, 4-state fill + label ramps, `borderFocus` focus ring).
- Icon sweep everywhere (settings, sheets, empty states, footer): GallopIcon where the set covers it (`search`, `settings`, `add`, `edit`, `ellipsis`, `trash`, `dismiss`, `send`, `message`, `folder`, `copy`, `lock`, `warning`, chevrons, `sidebar`, `arrow-up-right`), SF Symbols stay for gaps (`archivebox`, `cloud`, `desktopcomputer`, `wifi.slash`, `arrow.clockwise`, `network`, …) — the cowboy app itself uses SF Symbols in menus.
- Footer icon buttons (search/settings) adopt the cowboy ghost treatment: plain icon, `buttonGhostHover` circle appears behind on hover.
- Global tint: `Palette.nugget500` (cowboy shell tint; same hue the current tint resolves to).
- `AppAppearance` (system/light/dark) setting stays; every vendored token is light+dark adaptive.

## 7. Divergence log (Dash rule 3: divergences get written down)

| Divergence | Reason |
|---|---|
| Sidebar 280pt (Dash: 240) | Cowchat rows carry avatar + preview + timestamp; Dash Recents are title-only. |
| Room rows radius 12, 54pt (Dash pill: radius 100, 36pt) | Two-line rows with avatars; a full pill reads as a blob at this height. |
| Unread amber dot (Dash: weight bump only) | Explicit user request for an iMessage-style cue; rendered in brand `nugget500`, alongside the Dash weight bump. |
| Sidebar working glyph (Dash: none on rows) | Requirement 8; reuses Dash's own live-task `thinking` vocabulary. |
| Archive section in sidebar (Dash: none) | Existing cowchat feature, retained. |
| No collapsed rail | Cowchat keeps full sidebar show/hide instead. |

## 8. Testing

- `RoomSidebarPresentationTests`: drop pinned/groups/active cases; add `sortedByRecency` (ordering + tiebreak) and unread/working presentation predicates.
- New `RoomReadStateTests`: seeding, unread transitions, mark-on-select, prune, persistence round-trip.
- `AgentAppResolverTests`: name mapping (case-insensitivity, unknown names → nil).
- Gallop layer: token spot-parity test against the macos repo values (a fixture of ~10 hexes incl. `surfaceGlass500`, `hay50`, one `lantern` + one `cactus` shade); icon enum ↔ bundled SVG parity (cowboy `GallopIconsTests` pattern); font registration test that passes with or without the bundle (asserts fallback, not presence).
- `swift test` in `apps/CowchatMac` green; `build-app.sh` produces an app whose Resources contain the fonts/icons bundle (manual verification step in the plan).

## 9. Process

Implementation happens on a feature branch in phases (foundation → shell/sidebar → states → chat pane → sweep), each phase ending with tests green and a **Codex review checkpoint** (`auditcodex`); Codex feedback is triaged with `superpowers:receiving-code-review` rigor before adoption. The implementation plan (next step, via `superpowers:writing-plans`) breaks this into tasks with verification commands.

## Open items carried into the plan

1. Verify per-room presence delivery for non-selected rooms (working-state signal); pick primary vs. fallback accordingly.
2. Confirm the SwiftPM resource-bundle name/location for the executable target and the `build-app.sh` copy step.
3. Measure Dash bubble/composer radii and paddings from the Dash Figma node before styling those two surfaces.
