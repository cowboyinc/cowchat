# Rehearsal: shot 4, team sessions

A recreation of the Team page in the Cowchat Mac app as P9 draws it (`TeamView.swift`: `PageHeader`, `TeamColumnLabels`, the person head and `TeamRow`; `ContentView.swift`, `SessionWorkingMark`), compared against P9's render `14-scene-team.png`. Built from Gallop's tokens, icons and Spinner with stageshow's method; nothing from the skill's kit.

Gold: none draws a list page of this kind. The crop follows `framing.md`: the page's header and its first two people, with the rest of the page let run off the bottom.

Beats: none, a still. The title and branch count with the team switch and the search, the column labels, then two people with three branches each: the repository, the pull request with passing checks where there is one, and the state, two of them Working with the turning mark.

Captured: 1440 (the shot beside its words, about 680 wide) and 390, light and dark, with reduced motion.

Film: frames from element screenshots of `[data-cs-shot="4"]`, since `film.mjs` cannot read a stage without the kit's hooks. The surface guard checks six branches with two Working and that no row runs past the frame's sides.

- The first film had the app's column widths wrong, Repo and Idle in the wrong colours and the labels ending past their words. The row now has the app's 40 height and its columns (Repo 120, Pull request 88, State 80, 16 between, 40 after), the repository and an idle state are Text/Tertiary, and a working state is Text/Secondary. The labels were later moved off the app's form (Departures).
- The Working mark was Gallop's spinner in its nugget colour at 16. It is the same spinner in Icon/Tertiary at its own 12, as `SessionWorkingMark` draws it.
- "Pull request" wrapped onto two lines over its 66 of room. Labels keep one line.

Review (the one Codex round):

- On a phone the crop showed five of the six branches under "6 branches". Fixed: the phone frame is 470 tall and shows all six.
- The nested rim at the corners: fixed for every shot (shot 1's file).

Departures from the app:

- A Mac window has no phone width, so at 390 the Team page leaves out its team switch, its search and its Repo column, and the branch names start at the page's edge under each person.
- The app ends "Pull request" and "State" where their words end, over values that also end on the right, so "Pull request" starts left of the numbers and "State" stops short of the Working mark. The labels also sit 4 over the first person's head, which reads tight (`TeamColumnLabels` and `TeamView.swift`, the list). The shot starts the Pull request and State columns on the left with each label over its values' first letter, and puts the labels 8 over the first person. Repo keeps the app's form, ending on the right with its label over it. This is an app bug to fix in the same way.

Assumptions: none beyond the sample.

Unresolved: none.

Not checked: Safari and real phones.
