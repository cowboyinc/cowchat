# Hosted room key epochs and durable cutover

The follow-up writer now supports an internal `CommitKeyEpoch` command in its
existing owner stream, on control lane zero. The record binds the transition ID,
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

## Activation boundary still unfinished

There is no public command or route to perform this cutover. The internal
command is not a certificate verifier. Before submitting it, the owner
publication coordinator must authenticate the owner policy and current root,
complete the all-holder setup checks, publish stable custody/grants/policy with
expected-root CAS, confirm the published root and complete all-holder fencing
and admission. It must serialize that sequence with this writer and reconcile
old accepted intents first. That coordinator is not implemented yet; these
changes do not claim an end-to-end safe room rotation or permission to activate
the product.

The next integration must also record preparation in shared durable state before
publishing a new control policy. A marker only in the worker's local journal is
insufficient: another host can take over without that file. Reuse the owner
stream/archive for a prepared transition, retain the exact signed policy,
wrapped custody and grants, and block fresh room sends while it is pending.
Recovery must reconcile that same preparation before completing the cutover.
This preparation command and coordinator are planned, not implemented here.

The pinned SDK's general `Volume::commit` can rebase a pending mutation after a
conflict. The publication integration must explicitly enforce the attested
expected root at the actual commit boundary; it must not silently publish the
prepared policy onto a newer root. An initial root comparison alone is not
enough. Preserve pending evidence on an indeterminate commit and independently
confirm the published policy/root before the proxy barrier.

## Validation

At protocol `1e25ac7`, CBFS SDK `e6e8233`, and the isolated CBQS child `eb2ba17`:

- Reducer tests exercise ordered rotation, old receipts versus fresh old sends,
  epoch relabelling, predecessor mismatch, stable IDs, lane checks and replay.
- Real broker and subprocess CBFS-node tests hold the archive commit, prove the
  old epoch remains visible, cancel the write, require retired reads, recover
  the same transition, and check message receipts across the boundary.
- An authenticated TCP caller test rejects omitted/noncanonical/stale epochs,
  preserves labels through receipts/history, and sends/decrypts a real raw-key
  contextual message. It also proves the local password decoder is not used.
- The feature-enabled daemon library passes 158 tests (one existing ignored
  process helper); default workspace tests and strict all-target workspace
  Clippy pass. These are local author-run tests, not the consolidated live gate.

The preserved transport PR branches are unchanged. Final Homestead C1
height-based workload-authority reconciliation remains outstanding; see
`COWCHAT_ROOM_KEYS.md` on the isolated CBQS child branch.
