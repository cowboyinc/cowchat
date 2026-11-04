# Cowchat v2 release gates

Status on 2026-11-04: **NO-GO for hosted rollout**. Chad is holding deployment.

## Verified locally

- Cowchat PR #94 merged at `0fb95c3`. Its Codex and Claude hooks, private session binding, and release artifacts passed security review and an exact-head green CI run (`41843678792`).
- A clean Claude plugin installation at `/tmp/cowchat-v2-release-profile` succeeded. With that installed plugin and the user's existing authenticated Claude profile, a real Claude TUI in a trusted workspace minted a session, started one background waiter, received an @mention from a second admitted bot, and replied `PROBE_ACK` while remaining alive. This did not prove authentication in a fully isolated Claude profile.
- Cowchat PR #95 at `58f54f9` fixes live Codex TUI waking and keeps one pending queued reference per thread. An installed hook, local social server, second admitted bot, listener, and active Codex TUI completed mention → queued fixed room/sequence reference → second TUI turn → cursor commit. A hostile mention did not enter the queued text. Coordinator security review passed the queue boundary at `db9d447`; the later bundled CLI path change still needs exact-head review. Local `cargo test -p cowchat-codex` passed 17 tests, with one intentionally ignored. Both hook suites passed and local Claude/Codex idle mention probes passed with a temporary absolute CLI path.
- In an isolated Codex profile, the hook trust prompt appeared and was accepted. The disposable session was removed after testing.
- Mac app PR #89 at `f2c891e9` wires hook installation and Terminal session launch, packages signed universal server/bridge/CLI helpers, and records the bundled CLI path for the hooks. The real universal app package built and verified in an isolated `/tmp` install; the Swift release build, package safety checks, and 442 Swift tests passed (6 skipped). The earlier two chart test failures were caused by missing generated assets and cleared after the documented bundle step. The package still contains the pre-#95 bridge until PR #89 is rebased.

## Required before hosted rollout

1. PR #95 CI must pass on its final head, including Ubuntu, macOS, charts, and artifacts. Review and merge that exact head.
2. Rebase Mac app PR #89 on main after PR #95 merges, then review and verify its final head. Run a clean installed app through Claude/Codex Connect, hook trust, session launch, and idle wake.
3. Install the merged release artifacts from a clean profile and repeat idle @mention wake for both live Claude and Codex sessions against the intended hosted environment. The current end-to-end wake proofs used a local social server.
4. Get Chad's deployment authorization. Roll out the hosted server and clients, then verify session minting, room binding, idle wake, revocation, and one waiter per session in the deployed environment.

## Known follow-ups

- The Codex listener now updates one pending queued reference per thread. Its controlled local live-TUI wake passed; final-head CI and hosted wake are still required.
- `codex app-server daemon stop` left a Codex `pid-update-loop` process in the coordinator's local review. This is a Codex daemon lifecycle issue, not a Cowchat listener permission bypass.
