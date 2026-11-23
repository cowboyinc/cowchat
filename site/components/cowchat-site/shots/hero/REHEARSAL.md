# Rehearsal: shot 1, the hero

A recreation of a session's room in the Cowchat Mac app as the Mac prototype's ninth round (P9) draws it (`cowchat-mac-proto`, `native/CowchatMac/Sources/CowchatMac/ContentView.swift`, the room header and `MessageFeedRow`; `MessageAttachmentViews.swift`, `PullRequestRow`; `RoomConnectState.swift`, `ThinkingPulsePill`), compared against P9's render `08-scene-session-working.png` at 1080 by 740. Built from Gallop's tokens and icons with stageshow's method; nothing from the skill's kit.

Gold: `hero-demo` and `ai-chat`. Kept: one thing happening, a poster frame that tells the story alone, the loop that runs only in view, and messages that arrive by opening their height before their words fade in. Changed: no cursor and no typing in a composer, since the app's room has no composer in view.

Beats: the poster (the session's report with its pull request, the teammate's question, the session's answer with the new line counts) holds 2.6s; the two newer messages leave; the teammate's question arrives at 4.0s; the session thinks under the question at 5.3s and the room's caption turns to "cart-total active" in the accent; the answer arrives at 7.6s and holds; the loop starts again at 10.4s.

Captured: 1440 and 390, light and dark, with reduced motion (the poster), and the beats with motion allowed at both widths. A Playwright probe recorded the order poster, quiet, ask, thinking, poster; scrolling the hero away put it back on the poster and stopped it, scrolling back started it again, and switching to reduced motion mid-loop showed the poster and held it.

Film: stageshow's `film.mjs` finds a stage by the kit's `.ss-stage` and clock hooks, which this shot does not have, so its gate cannot read it. Frames came from element screenshots of `[data-cs-shot="1"]`, and the surface guard (`tools/snap/cowchat-site.mjs`) checks the poster's content and crop.

- First film: at 1440 the pane stopped well above the frame's bottom, and at 390 the answer's pull request row was cut off at the bottom, because the shot's wrapper had no height. The wrapper fills the frame now, and the guard fails if the answer's row ends below the frame.
- At 390 the pull request row took three lines. It takes the app's own two-line form now: the title wraps beside the name and the status goes under it, 24 in.
- The dev server served a stale server render after an edit to the parts; it was restarted before the frames above.

Review (one Codex fresh-eye round, with review.md, verify.md and the frames):

- The opening beat leaves the lower half empty at 1440. It stands: the room fills as the conversation arrives, and the frame was cut from 560 to 480 so the poster fills it.
- Thinking sat above the teammate's question, where the app anchors the pill under the session's own last message. It was kept at first. On the lead's read it looked as if the session began thinking before it was asked, so the pill now hangs 10 under the question (Departure from the app).
- The presentation rim and the pane's own border nested at the corners. Fixed: each shot is cropped to the pane, so the frame's edge is the pane's edge.
- The fresh review after the lead's pass (Fable and Codex, with every still): a reset to the poster from another step opened the question and the answer together, each cut off while its height grew, and a message's words began to fade in while its height was still opening. Fixed: a reset lands on the poster with transitions off for a frame, and the words fade in once the height has opened.
- Agent avatars were circles and the body text read light. Fixed: a message's disc is `AgentAvatar`'s rounded square, the text is the app's weight 550 with each style's letter spacing, and the words are Body L in Text/Secondary as `ExpandableMessageText` draws them.

Departure from the app: the app hangs a session's thinking pill under that session's latest message, which here is above the teammate's question. The shot hangs it under the question, so the order reads as the story: asked, thinking, answered.

Assumptions: the question and the answer both say "now"; the caption names the session alone where the app may add its owner's handle; the session's disc draws the Claude icon, as the Join sheet draws the same session, where P9's room draws the session name's initial because `AgentAvatar` resolves an app icon only from a name that contains "claude" or "codex".

Unresolved: none.

Not checked: frames at 50ms through a transition, since `film.mjs` could not run; a hidden tab (the code stops on `visibilitychange`, but no frame shows it); Safari and real phones; touch.
