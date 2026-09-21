# Cowchat service redesign

Status: implementation in progress on `jw/cowchat-v2-simple`. This is the agreed direction and a delivery checklist, not a claim that hosted actors or clients already work.

## Product contract

A human, local agents, and Cowboy actors share a fast, persistent room. A participant sends once, gets a committed message ID and sequence, and reconnects by replaying after its cursor. An actor may keep a process and connection warm, or sleep without compute charges and wake for new work. Closing the browser must not stop the actor.

Builder conversations remain Cowboy Labs project conversations in Postgres. Cowchat room content lives in the room service's storage, encrypted for participants; the dashboard must not copy it into builder transcripts, Postgres, logs, or telemetry. An authorized actor runtime needs plaintext to reason about a message; room encryption does not conceal that plaintext from the runtime executing it.

The user requires the full experience: dashboard, native clients, Telegram, Cowboy actors, execution funding, and recovery. A local demo is the first checkpoint, not acceptance of the entire replacement.

Decisions come from `cowchat-situation.md`, `cowchat-situation-supporting-material.md`, `answers-cowchat-redesign.md`, the Patrick conversation of September 20, and review in the exact `dashboard-cowchat-v2` room. The later answers supersede the old specification where they differ.

## Keep the architecture small

One Cowchat service owns rooms, member lists, ordered history, and durable actor subscriptions in SQLite. Keep its existing TCP/Unix/WebSocket transports and asynchronous webhook delivery. Reuse crypto primitives where they reduce work. No CBQS or CBSS dependency in the message path. No transaction on every message. No Signal fork.

Local operation needs no identity authority or authentication ceremony. Shared room keys remain a valid simple local configuration. Hosted access uses authenticated participants and server-enforced membership. Cowboy actors carry stronger, visibly distinct attribution: enrollment binds their signing key to their Cowboy actor identity, checked against controller authority at enrollment and reconnect. Local display names must never masquerade as verified Cowboy identity.

Chain state should be as small as possible: authenticate the service/actor binding and refer to the room service. Ownership and membership live in Cowchat. The exact chain anchoring transaction must be selected against current chain interfaces before calling hosted support complete. Do not add a chain-backed roster or native execution proof to ordinary chat.

For hosted encrypted rooms, the invite identifies the service, room, and owner public key. The owner signs the current member list. The server stores that list; clients verify its signature. The owner distributes the room key to members, using existing HPKE wrapping if individual key delivery is needed. On removal, the owner creates and distributes a new room key. A key ID on ciphertext selects the decryption key for history. No roster hash chain, compare-and-swap protocol, per-message generation admission, forward secrecy, or background identity-renewal lease in v1. Revocation takes effect on reconnect/rekey; old recipients cannot be made to forget old plaintext.

Actor messages are signed by a key authorized to speak for the actor. This is attribution, not proof of a particular execution. Human/local signatures are optional. Hosted clients must verify attribution rather than trust an unsigned badge from the server. TLS authenticates the endpoint; no extra server identity handshake.

Federation remains a requirement for a later version. The first implementation uses one owner-selected service per room; it does not claim replicated authority or availability across services.

## Durable messages and actor work

The append transaction allocates the room sequence and commits the message, retry receipt, and matching subscription deliveries together. A retry with the same message ID and content returns the original result. Changed content under that ID conflicts. Broadcasts happen after commit; replay is authoritative.

Use the existing `subscription_deliveries` row as the actor work item. Do not introduce a second queue. Actor subscription state contains the actor ID, wake mode, and processing cursor. Modes are ALWAYS, ADDRESSED, and LISTEN; self messages and thinking/system events never start inference. Addressing uses participant IDs, not display-name comparison. A subscription begins at the current room tip, so creating it does not unexpectedly spend inference on old history.

A webhook wake carries only the room and subscription identifiers. It carries no message content, keys, code, or instructions. Its destination and signing secret are configured at enrollment. Existing destination/SSRF protections apply. A successful HTTP delivery means the receiver was notified, not that it processed the message. Keep retrying/coalescing notifications until processing completes.

Warm and cold workers use the same claim operation. Only the oldest unfinished item is claimable; a claim expires after five minutes to recover a crashed worker. This permits duplicate inference after a long pause. It does not promise exactly-once execution or make arbitrary external side effects safe to repeat. An actor performing external actions must use that destination's idempotency support or its own application recovery policy.

Each item has a stable reply message ID. Within one attempt the worker reuses the prepared encrypted reply; generating a different ciphertext under the same ID is a conflict. The claim returns any already-persisted reply so recovery skips inference after append. A crash before append may repeat inference. There is no separate local durable outbox. A completion acknowledgment checks that the reply belongs to this actor, room, and input. Explicit skipped/failed outcomes let a poison message stop blocking the actor. A processing cursor never jumps over earlier unfinished work. Actor-room history and reply receipts persist until explicit deletion, including through long sleeps.

The actor funds its execution. Do not add message billing, sender sponsorship, or a combined transport/execution grant. Room transport/storage charges are separate. Runtime authorization and spending limits remain required when invoking the Cowboy execution provider; a room invitation is not a permission to spend arbitrary funds.

## Implementation and verification

Preserved old work lives on the original branches/PRs and in `.backups/cowchat-v2-20260921/cowchat-before-reset.bundle` in the sibling workspace. Start from Cowchat main `659b2b7`; the only imported implementation change so far is durable append `3fc2e33`, cherry-picked as `85ce4e6`. Do not merge the old room-session/CBQS/CBSS branch stack wholesale.

1. **Cowchat delivery foundation.** Reuse durable append; implement persistent actor subscriptions, content-free wakes, oldest-first claims, explicit completion, and restart recovery. Test transaction rollback, repeated send, wrong actor/room, claim expiry, and interrupted reply/ack.
2. **Participant identity and encryption.** Keep local no-auth/shared-key use working. Add the small hosted member-list/key-distribution path and signed actor attribution with enrollment/reconnect authority checks. Reuse existing primitives, not chain-heavy certificate policies. Test wrong signer, wrong room, removal/rekey, and encrypted replay.
3. **Actor adapter.** Own Cowchat-specific transport and wake logic in Cowchat or the harness, never the generic Runner. Fetch/decrypt input only inside the authorized runtime, execute through the generic harness, persist/retry the reply, then acknowledge. Support warm and cold entry through the same code. Validate actual funding and controller checks at the provider boundary.
4. **Clients and integration.** Wire dashboard rooms directly to Cowchat, keep builder chat separate, complete native encrypted send/read, and connect Telegram through an explicit authorized participant. Verify no room content reaches Labs transcript storage or telemetry.
5. **End-to-end acceptance.** Exercise a mixed human/local-agent/Cowboy-actor room, browser closed, warm and cold response, service/runtime kill-and-restart, encrypted history, key changes, exhausted funds, duplicate delivery, and real client surfaces. Record measured message and cold-start latency. Tests of local code are not evidence of hosted deployment or real billing.

First checkpoint: local encrypted room with three participants; send/replay; a sleeping actor starts and replies; pending work survives process termination and restart. Stop adding machinery to that checkpoint once these behaviors pass. Continue the remaining product work above before declaring the replacement ready.
