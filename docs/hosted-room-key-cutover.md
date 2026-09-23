# Hosted room key epochs and durable cutover

The follow-up writer supports internal `PrepareKeyEpoch` and `CommitKeyEpoch`
commands in its existing owner stream, on control lane zero. Preparation retains
the exact signed setup/policy bytes, encrypted custody and grants before any
control-volume publication. It pauses fresh sends for that room while preserving
existing receipts and service to other rooms. Commit must match the prepared
transition exactly and clears preparation only after archive commit. The record binds the transition ID,
expected previous policy hash, new policy/key epochs, policy hash and control
root. Its command ID is the lowercase hex transition ID, so uncertain retries
cannot silently switch bodies. Initial key/policy epochs are zero; later
transitions must extend the exact current policy and strictly advance both.

The existing runtime writes the command through its local pending-intent journal,
CBQS owner log and private CBFS archive. It changes the shared projection only
after archive commit. Cancellation or an indeterminate commit retires the writer;
recovery reconciles and completes the same durable intent before accepting work.
No control-volume activation flag is written, which avoids changing the root
that the completed proxy fence attested.

Preparation now also retains the predecessor's signed policy for successors.
This is necessary to reconstruct and validate the pending publication after its
new policy has replaced the mutable control record. Older experimental records
that omit these bytes remain paused on replay; the publication verifier must
recover the predecessor or fail closed. Omitting this optional field preserves
the earlier record encoding and receipt digest; it does not grant permission to
skip predecessor verification.

Each hosted send and returned message has an optional `key_epoch`, encoded as
canonical decimal u64 text for clients. Rooms without an activated key retain
the existing unlabelled behavior. Once activated, new messages must name the
current epoch. Receipt lookup comes first: an already accepted old-epoch message
keeps its original receipt, while a new old-epoch message is rejected. Changing
the epoch or ciphertext under an existing message ID is a conflict. The epoch
is included in the command digest. Legacy unlabelled command digests and wire
serialization stay unchanged.

`CowchatClient::prepare_room_key_message` uses the existing raw-key contextual
codec and returns a payload to persist before sending. The local password/HKDF
decoder never attempts to open an epoch-labelled message. The local server
rejects labelled sends; its SQLite schema and history remain unchanged. The
legacy actor-work helper refuses this profile until its room-key consumer is
wired. CLI/Swift/browser room-key UX and discovery remain follow-up work.

## Hosted room-key creation

The opt-in `room-keys` feature adds the production prepare, attest, activate,
open, and rotation frames to `hosted-serve`. Dashboard and native clients keep
member signing keys and plaintext room keys locally. Cowchat stores only the
signed public setup, policy, grants and encrypted custody, then publishes,
confirms, fences every holder, and commits the exact epoch through the owner
log and archive. A room may contain up to 32 members; that bound keeps every
all-epoch grant matrix through the full removal lifecycle in one durable CBQS
record.

The earlier `hosted-room-demo` command duplicated this client flow around one
prepared JSON file and could no longer create a room after writer claim moved
the control root. It has been removed; clients use the hosted frames directly.

Preparation is shared durable state, not a marker only in a worker's local
journal. A replacement worker with an empty journal recovers it from the owner
archive/log and keeps the room paused until matching cutover. Preparation only
checks structural replay invariants; it does not authenticate its opaque signed
bytes or prove that encrypted custody contains the intended key. A stored setup
request is not a fresh attestation. The coordinator must re-establish current
setup permission and preserve the exact prepared custody, policy and grants.
There is no automatic abort, rebase or roster replacement while pending.

The serialized preparation is bounded to 240 KiB, leaving space in a 256 KiB
CBQS envelope. Large membership/history sets that exceed that bound must be
rejected before publication; records are never truncated. This is not a claim
that every protocol-maximum roster fits the current hosted transport.

The pinned SDK now offers `Volume::commit_at_root`, which rejects a different
base or pending mutation and never rebases onto an unrelated root. Use it for
publication: general `Volume::commit` still supports automatic rebase. The
registry must enforce actual predecessor CAS, and the coordinator must retain
pending evidence and independently confirm the published policy/root before
the proxy barrier. The CBSS publication library and hosted room-key coordinator
both use that path.

A worker takeover itself changes the control volume through writer allocation.
An old setup request/root therefore cannot automatically authorize a fresh
publication after takeover. If the exact prepared policy has already landed,
recovery must authenticate it before fencing; otherwise it needs a fresh
owner-signed setup request and ALL-holder attestations for the current root,
while retaining the original transition/policy/custody/grants. This is explicit
renewed authorization, not an SDK rebase or a stored-ACK bypass. Until that
renewal/reconciliation is wired, the prepared room remains paused.

## Validation

At protocol PR #149 @ `98c393e`, CBFS SDK `0711f7e`, CBQS child `00bdc82`, and CBSS room release `6865839`:

- Reducer tests exercise ordered rotation, old receipts versus fresh old sends,
  epoch relabelling, predecessor mismatch, stable IDs, lane checks and replay.
  Preparation checks cover replacement conflicts, bounds, malformed bytes and
  cutovers that disagree with prepared fields.
- Real broker and subprocess CBFS-node tests hold the archive commit, prove the
  old epoch remains visible, cancel the write, require retired reads, recover
  the same transition, and check message receipts across the boundary. A second
  worker with an empty local journal recovers exact preparation, keeps fresh
  sends paused, returns old receipts and serves another room through cutover.
- An authenticated TCP caller test rejects omitted/noncanonical/stale epochs,
  preserves labels through receipts/history, and sends/decrypts a real raw-key
  contextual message. It also proves the local password decoder is not used.
- The feature-enabled daemon library passes 164 tests (one existing ignored
  process helper); default workspace tests and strict all-target workspace
  Clippy pass. These are local author-run tests, not the consolidated live gate.

Root-authorized SDK journals now live in a separate namespace per approved
predecessor. Ordinary `WriterRegistry::open` recovery cannot submit them through
its generic registry and bypass fresh setup authorization. A real subprocess-node
regression leaves a room publication pending, opens/promotes the actual writer,
and verifies that the room objects remain unpublished and journal bytes remain
unchanged. Takeover can advance the control root; a new publication still needs
fresh owner setup for that root and preserves the older pending evidence.

## Remaining proof

Local checks exercise the room-key, publication, archive and hosted-message
components. The complete flow still needs a live run against a validator,
finalized-proof couriers, private CBFS volumes, CBQS broker and CBSS committee
before it is a live end-to-end PASS. That consolidated gate must include room
creation, rotation, recovery and connector clients.

The preserved transport PR branches are unchanged. Final Homestead C1
height-based workload-authority reconciliation remains outstanding; see
`COWCHAT_ROOM_KEYS.md` on the isolated CBQS child branch.
