# Cowchat transport recovery contract

Reviewed design contract, 2026-09-13. This defines the next implementation boundary;
it does not add a CBQS adapter, archive client or new production authority.
Chad's current constraint is controlling: room message and wake flows do not
write consensus state. Track R and its execution ledger remain frozen.

## Source baseline and the problem

Inspected source: Cowchat `6a3b381530b0eb6e91959b5862c6761477f2e7b7`,
CBQS `cd48864c8c75f6487905fd507172e29ade97f92d`, and CBFS
`7c39d3037008748cbdfe91b81a76ab050606f6ca`. The workspace
`cbqs-handoff.md` was consulted first; its four repeated v1 sections are historical.
The checked-out v2 source supplies the API contract below. This is not evidence
of which build Canyon runs.

Today `Store::append_seated_record` verifies the request and signed envelope,
checks current credential/key generations and consumes the request nonce in an
immediate SQLite transaction. `Store::append_on` adds message identity, sequence,
deduplication receipt and matching wake obligations in that transaction.
History reads the same SQLite store. There is no storage adapter trait yet.
Relevant files are `store/seated.rs`, `store.rs`, and `store/seated/history.rs`
under `crates/cowchat-server/src/`.

CBQS v2 `SessionV2::request` accepts `Append { lane_id, payload }` and returns
`Appended { sequence }`. An ambiguous append retried after a disconnect can
produce another physical record: v2 deliberately does not deduplicate it
(`cbqsd/src/conformance_v2.rs`, `AfterWriteBeforeAck`). A provider-signed
`GetCheckpoint` response is verified by `SessionV2` before it is returned.
Subscription delivery and checkpoint/record verification are separate operations;
merely decoding a delivered frame does not authenticate its payload.

Putting an asynchronous append behind the current SQLite function signature
would break its atomicity. A generic `append/replay/subscribe/ack/health` trait
must not conceal that break or claim remote commit on a local transaction.

## Authority stays above transport

The room service remains the authenticated admission and publication authority.
Actors and the harness use its signed HTTP surface; they do not hold CBQS sockets.
The transport implementation owns its broker session, not the room's decryption
key. It stores and moves the existing signed, encrypted envelope unchanged.

For the first adapter, the service is the sole broker append principal for a
room stream. Room participants do not receive unrestricted raw append grants.
Raw CBQS deliveries are transport evidence, not published room history. A future
direct-client transport would need the same verifiable publication and fencing
rules; distributing stream grants alone would bypass them. This service-mediated
shape is the reviewed first-adapter choice, not an already implemented restriction.

CBQS grants are signed by the stream admin and bind instance, stream,
authorization generation, holder key, verbs, lane/group scope and validity
interval. Lane creation requires the relevant admin verb/scope; a lane is not an
independent encryption boundary. Use least-privilege, bounded grants for a
pre-provisioned binding. No room operation here creates a registry entry, bumps a
registry generation, funds escrow or changes CBQS consensus code. Room revocation
must be enforced by the room authority without a per-message registry write.
How a deployment obtains and rotates its initial transport grants is a separate
provisioning gate; it does not authorize room-key release.

## Identity and positions

An intent binds chain instance, room, transport generation, client message UUID,
authenticated signing principal, canonical header bytes, encrypted body and
signature. Compare the complete authenticated envelope representation, not JSON
whitespace, mutable display names, or unverified caller-supplied digests. Define
and vector-test that encoding before persisting a new digest format. Existing
message-ID conflicts must not become weaker under the adapter.

Keep three positions distinct:

| Position | Meaning |
| --- | --- |
| Admission order | Durable local ordering of accepted intents within a room |
| Broker sequence | Physical transport position; retries can consume several |
| Room position | One position assigned to one published logical message |

Public room cursors retain room and transport generation. They never silently
switch to physical broker sequences. A repeated authenticated envelope maps to
its original room position and original wake obligation; it adds no visible
record. Different authenticated bytes under an existing identity conflict.
Unauthenticated records cannot reserve, poison or advance that identity.

Resolve admission order serially for the first implementation. Later intents
cannot overtake an earlier intent awaiting publication or cancellation; an
already-published intent waiting for archive completion does not hold this order.
A canceled intent receives no room
position. This deliberately favors a clear recovery rule; bounded batching can
be designed later without changing identity or publication semantics.

## Durable state transitions

These are proposed application-store states, not consensus records. No network
call occurs while the SQLite transaction is held.

1. **Admit.** Under the current room authority transaction, verify the signed
   request/envelope, current rights and generations; claim the request nonce;
   insert an immutable intent, archive work and matching wake obligations together. Record
   admission order, writer epoch and the authorization revision being used.
   The obligations exist but are not dispatchable. Replays return the existing
   intent/result; changed authenticated bytes conflict. An outbox failure rolls
   back nonce, intent and admission order together.
2. **Submit or reconcile.** The active transport worker reads committed intents.
   It never reconstructs fresh ciphertext for a retry. Record evidence of an
   authenticated matching physical record and verified checkpoint coverage.
   After an ambiguous response, reconcile from a retained verified cursor. A
   repeated physical append is tolerable only because logical publication
   deduplicates it. Lack of a retained range is an explicit recovery gap, not
   proof that the first append failed.
3. **Publish.** In a new local authority transaction, recheck current writer
   epoch, binding and authorization revision and validate the recorded transport
   evidence. Assign the next room position, freeze the result and make the
   original wake obligations dispatchable atomically. Publication makes the
   message visible in room history; it does not wait for archive completion and
   is not yet the final sent acknowledgment.
4. **Archive independently.** Persist the encrypted record and its ordered recovery metadata
   to the independent archive. Mark archive completion only after that archive's
   own durable commit and verified read/recovery evidence. A local intent row,
   a broker acknowledgment, an upload request or a checksum alone is not that
   proof. This archived object contains the immutable intent identity, admission
   order, relevant authority revisions and transport binding. An archived intent
   is not itself a publication receipt and cannot establish visible history on
   its own. This worker runs asynchronously; archive lag does not block
   publication or wake dispatch.
5. **Deliver.** Existing per-seat durable delivery/retry/ack semantics apply to
   the published logical record. Transport-consumer acknowledgment, producer
   acknowledgment and the receiving actor's wake acknowledgment are distinct.
   Consumer processing/output remains at least once; this contract guarantees
   one visible room record, not exactly-once billing or provider side effects.
6. **Acknowledge sent.** The producer's final sent acknowledgment is the stronger
   durability claim: it requires both publication and verified independent
   archive completion. Archive stalls leave the producer pending-not-sent while
   the published message and its wakes continue normally. Neither a retry nor
   later archive completion publishes or wakes the same message again.

An implementation will need a bounded pending response or internal wait when
remote durability is incomplete. This draft does not change the existing HTTP
response schema or retrofit today's successful SQLite append as archive-backed.
The exact pending/status API is a follow-up integration decision.

## Fencing and revocation

The local authoritative store serializes writer epoch changes, membership/key
revisions and publication. A worker can perform network I/O only for its issued
epoch and must recheck that epoch at publication. Lease timeout alone is not a
fence: every publication transaction must reject an obsolete epoch.

If revocation or a binding change wins before publication, an old intent cannot
become visible afterward. Cancel it, retain its identity/tombstone and physical
evidence, and suppress its wake. Do not automatically adopt it under a new
writer or ask the new writer to sign different bytes under the same ID. An
explicit recovery rule for adoption would require separate review.

A stale worker might still complete a broker append or archive upload. That is
an orphaned encrypted transport record, not a published message. The prohibition
is on post-fence room visibility and wake dispatch; this design cannot claim it
erases bytes from a broker that accepted an in-flight request. A raw consumer
that ignores publication authority does not satisfy the contract.

Multiple processes must share the same authoritative durable epoch/revision
store. Independent writable database copies are not supported. Restoring an
older copy cannot safely resume writes from its own stale epoch: recovery must
establish a current non-regressing fence and revocation floor, or remain paused.
No external restore authority is invented here. Demonstrating that fence is a
launch gate, not something a process-local mutex proves.

## Independent archive and no-consensus constraint

Both hosted SQLite and CBQS seated rooms require an independent archive under
the agreed product requirement. The local log is not its own archive. Local
key-only rooms retain their separate exemption.

The inspected CBFS SDK's `replay_pending_diff` invokes
`ManifestRegistry::commit_manifest_v2` (`sdk/src/commit.rs`); the Cowboy hook
posts `/cbfs/v1/manifests/commit-v2` (`hooks/src/cowboy.rs`). That normal manifest
commit path is not established as a no-consensus operation. This draft does not
authorize using it per message, batching consensus writes as an assumed
exception, or treating shard uploads as a committed archive instead.

Before a production archive adapter, identify and review an independent durable
CBFS recovery path compatible with Chad's constraint, or obtain his explicit
decision on the requirement conflict. A fixture archive can exercise ordering
and failure handling while that gate remains open. It cannot certify real CBFS
durability. An unavailable/unproven archive leaves archival progress and the
producer's final sent acknowledgment pending; publication and wakes continue
on their own transport and authority evidence. It must never silently convert
local-only durability into an archived or final-sent claim. This split follows
the coordinator's clarified publication-versus-acknowledgment contract; it does
not authorize adoption of the unresolved CBFS commit path.

For pruned broker history, use a verified independent archive that covers the
missing interval and joins the current transport anchor, together with the
authoritative publication mapping. The mapping is committed at publication;
archiving an earlier intent does not make that later mapping independently
recoverable. A launch restore protocol must preserve both mapping and revocation
floor or fail closed. This draft does not claim archive-only disaster recovery.
Without the required evidence return
`HistoryGap` with the unavailable interval; never jump the cursor and call the
history complete. The reported CBQS hydration-drop fix `6a1d48d` did not resolve
in the inspected checkout and is not claimed as included. Deployment checking
must verify its actual descendant before relying on it. The reported escrow
unit gotcha is another provisioning checklist item; no escrow command runs in
this work.

## Executable proof plan

Start with the actual SQLite implementation, not a model that simply repeats the
desired transitions. Add a test-only conformance driver beside the existing
seated store tests using real `Store::open` databases, signed fixture envelopes,
existing subscription APIs and authority mutation helpers. It must exercise
authenticated append/retry/conflict, rejected signatures, injected outbox
failure, close/reopen after commit, and revocation-before-append. Assert message
identity, room tip, nonce rollback and exact durable obligation count. Keep
plain legacy append tests separate from signed seated admission proof.

Reuse `seated_append_auth_nonce_idempotency_and_revocation_share_one_transaction`
for its existing signed authentication/retry/conflict checks. The additional
coverage is the persistent signed path: close/reopen after commit and an injected
outbox failure proving the signed request nonce also rolls back across reopen.
Existing legacy reopen/outbox tests are useful precedent, but do not substitute
for those seated caller tests. Do not duplicate already covered assertions merely
to increase the count or describe fixture execution as process-kill evidence.

Then use an explicit fault-injected transport/archive model for the new async
states. Enumerate cuts before/after intent commit, broker acceptance before lost
response, replay with duplicate physical sequences, archive commit before lost
response, revocation/epoch change during I/O, and publication before lost client
ack. Assert no final sent result/history/wake before publication, no final sent
acknowledgment without archive evidence, publication and wakes progressing while
archive is stalled, no obsolete-epoch publication and no duplicate logical record.
Test malformed/forged evidence, changed authenticated bytes, missing
history, and restore with a stale epoch as negative cases.

The common scenario names and observable invariants become the adapter
conformance suite. Current SQLite can pass the admission/dedupe/wake/reopen
subset; it presently lacks independent archive gating and asynchronous writer
epochs. Those gaps must be explicitly reported, not hidden as passing mocks or
zero-test filters. The future implementation must run these scenarios through
real CBQS and CBFS callers before being called conformant.

## First proof and remaining file boundaries

The first executable proof is
`crates/cowchat-server/src/store/seated/transport_contract_tests.rs` and its
test-module declaration in `store/seated/tests.rs`, reusing the existing private
fixture helpers. Both new persistent signed-path tests passed; the complete
server library passed 153 tests, and all-targets strict Clippy and formatting
passed on Rust 1.93 with locked offline dependencies. Logs are
`/tmp/cowchat-transport-contract-{tests,server-lib,clippy}.log`.
This adds no public production trait, dependencies or migrations.
The exact async model files and the eventual durable schema/network integration
will be proposed separately after agreement on publication, archive and fencing.

Service-only broker access, conservative cancellation across authority changes
and the non-regressing restore gate are reviewed design requirements. The pending
response shape and the no-consensus independent archive path remain open;
implementing a concrete restore fence still requires review and proof.
Production key custody, initial grant
provisioning and deployment remain outside this draft's authority.
