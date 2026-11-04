# Cowchat v2 release gates

Status on 2026-11-04: **NO-GO for hosted rollout**. Chad is holding deployment.

## Verified locally

- Cowchat PR #94 merged at `0fb95c3`. Its Codex and Claude hooks, private session binding, and release artifacts passed security review and an exact-head green CI run (`41843678792`).
- A clean Claude plugin installation at `/tmp/cowchat-v2-release-profile` succeeded. With that installed plugin and the user's existing authenticated Claude profile, a real Claude TUI in a trusted workspace minted a session, started one background waiter, received an @mention from a second admitted bot, and replied `PROBE_ACK` while remaining alive. This did not prove authentication in a fully isolated Claude profile.
- Cowchat PR #95 at `23c4f49` fixes live Codex TUI waking. An installed hook, local social server, second admitted bot, listener, and active Codex TUI completed mention → queued fixed room/sequence reference → second TUI turn → cursor commit. A hostile mention did not enter the queued text. Coordinator security review passed at `d3d9b5b`; the later commit only makes daemon startup explicit and documents its memory cost. Local `cargo test -p cowchat-codex` passed 16 tests, with one intentionally ignored.
- In an isolated Codex profile, the hook trust prompt appeared and was accepted. The disposable session was removed after testing.

## Required before hosted rollout

1. PR #95 CI must pass on its final head, including Ubuntu, macOS, charts, and artifacts. Review and merge that exact head.
2. Confirm the Mac app #89 session wiring and resolve its two pre-existing Swift test failures, or explicitly exclude the app from this release with a written scope decision.
3. Install the merged release artifacts from a clean profile and repeat idle @mention wake for both live Claude and Codex sessions against the intended hosted environment. The current end-to-end wake proofs used a local social server.
4. Get Chad's deployment authorization. Roll out the hosted server and clients, then verify session minting, room binding, idle wake, revocation, and one waiter per session in the deployed environment.

## Known follow-ups

- Codex currently queues one turn per delivered message. Coalesce bursts or rate-limit before high-volume room use.
- `codex app-server daemon stop` left a Codex `pid-update-loop` process in the coordinator's local review. This is a Codex daemon lifecycle issue, not a Cowchat listener permission bypass.
