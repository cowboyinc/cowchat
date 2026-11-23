# Rehearsal: shot 3, bring someone in

A recreation of the Join sheet over the Inbox in the Cowchat Mac app as P9 draws it (`JoinRoomSheet.swift` and `SheetParts.swift`; `InboxView.swift`; `CowchatPillButtonStyle`), compared against P9's renders `22-scene-inbox-join.png` in light and dark. Built from Gallop's tokens, icons and Button with stageshow's method; nothing from the skill's kit.

Gold: none draws a sheet over a list. The crop follows `framing.md`: the pane the story happens in, with the sheet as the focal object over its dimmed Inbox.

Beats: none, a still. The Inbox with the invitation as its first, unread row; the sheet names the teammate's room and who invited the reader, lists No session and two of the reader's sessions with their repositories, and has the first session picked; Decline at the start, Cancel and Join at the end.

Captured: 1440 (full width) and 390, light and dark, with reduced motion.

Film: frames from element screenshots of `[data-cs-shot="3"]`, since `film.mjs` cannot read a stage without the kit's hooks. The surface guard checks the sheet's three buttons, the picked session, both marks loaded, and that the sheet ends inside the frame.

- At 390 Join ran past the sheet's edge with Gallop's medium buttons. The buttons are the small size now, which is the app's 36-tall pill, and the sheet takes the phone's width less 16.
- The sheet was Surface/700, whiter than the app's. It is Surface/600, as `JoinRoomSheet` sets it.
- The frame was cut from 520 to 440 tall at 1440 and to 400 on a phone, so the sheet does not float over an empty Inbox.
- The fresh review with every still found the phone's sheet still covering all three Inbox rows over a bare scrim, so a reader could not tell it was a sheet over anything. On a phone the frame is 480 tall and the sheet sits 8 above its foot, under the dimmed Inbox's title and the invitation row. The app drops a sheet from the window's top; the phone crop is the departure.

Review (the one Codex round):

- The dimming was much weaker than P9's. Fixed: the scrim is the prototype's heavy scrim at half strength in light and full in dark, which matches P9's renders in both.
- The sheet has rounded corners and a shadow where P9's render is square and flat. It stands: P9 composes the sheet flat in its renderer, and a macOS sheet has both. This is an assumption.
- The session mark is Anthropic's many-spoked Claude icon where the app draws an asterisk. It stands: the app's asterisk is an SF Symbol standing in for Claude's mark on the same clay, Gallop has no asterisk, and Patrick chose the owners' official marks for this page. The file is `public/assets/cowchat-site/claude-icon.svg` (the route's `CLAUDE.md`, The marks).

Assumptions: the sheet's corners and shadow; both sessions run Claude Code. The app draws a Codex session with its own SF Symbol tile, not OpenAI's mark, so the Blossom has no place in this sheet.

Unresolved: none.

Not checked: Safari and real phones.
