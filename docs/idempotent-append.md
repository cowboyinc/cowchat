# Idempotent append and transactional webhook outbox

This slice extends the existing legacy `send_message` frame. It does not expose
the signed v3 HTTP append endpoint or authorize a v3 seat.

A caller may provide `message_id` (1–128 UTF-8 bytes, no control characters).
Omitting it preserves the old behavior: the server generates a UUID for each
send. A retry uses the same ID and the same decoded append projection: room,
authenticated agent ID, content, reply reference, metadata, and mentions. JSON
object key order is normalized by the current sorted-map serde_json decoder;
array order remains significant. Display name is excluded so a rename does not
break a retry. Encrypted clients must persist and retry the same ciphertext,
not encrypt again. The shipped convenience client still omits IDs; durable
client send queues and the signed v3 adapter are later work.

The first append commits all of these in one immediate SQLite transaction:

- The message and its next room sequence.
- A retry receipt containing a SHA-256 projection digest and original response
  scalars: sender display name, timestamp, sequence, room and sender IDs.
- One delivery row for each matching active subscription, with filters evaluated
  against subscription state in that transaction.

A matching retry returns the original response, reconstructing content, reply,
and metadata from the verified retry input. It does not allocate another
sequence, broadcast, emit mentions, change the turn, count another message, or
enqueue another delivery. A different projection or sender under a retained ID
returns `message_conflict`. Room membership is required even for retries; a
read-only receipt check before quota admission allows an already-accepted retry
when the sender has exhausted its message quota. The transaction repeats the
check so concurrent connections cannot create duplicate effects.

Existing history rows without a receipt fail closed on ID reuse; their original
mentions cannot be reconstructed. No guessed receipts are backfilled.

## Retention and delivery

Receipts hold no content, metadata or reply reference. Tier retention sweeps
prune them after the history retention window plus seven days, and only after
the corresponding history row is gone. For current hosted tiers that is a
minimum dedupe horizon of 21 or 97 days from append. A retry after both receipt
and history are gone may append anew. Local rooms without retention retain their
receipts with the room. Explicit room destruction removes them.

`subscription_deliveries` is the outbox. Handlers only notify the existing worker
after commit. Startup and periodic scans find pending work even if the process
exits before notification. HTTP remains at-least-once; a lost response can cause
redelivery and receivers must deduplicate downstream effects.

Pending deliveries pin their message rows against tier retention. They become
`abandoned` after the existing six-attempt retry budget, an inactive subscription,
a rejected destination, a missing message, or their 24-hour enqueue deadline.
Both worker scans and retention sweeps enforce the deadline, so downtime cannot
hold retention indefinitely after the next sweep. Terminal rows release the
history pin; later pending sequences can proceed past an abandoned row. Consumers
must backfill by sequence to recover a gap, subject to history availability.
Abandoned rows carry only routing IDs and diagnostics and are pruned seven days
after their deadline. Successful rows are deleted; explicit subscription deletion
cascades its queue. Existing pending rows get a 24-hour deadline when upgraded.

The migration is additive: `message_appends`, plus delivery `status` and
`deadline_at`. An older binary must not serve new appends after upgrade: it does
not create receipts or atomic outbox obligations. This is a coordinated binary
upgrade, not mixed-version write support.

## Validation and remaining scope

Tests exercise cross-connection duplicate races; changed sender, room, content,
reply, metadata and mentions; enqueue failure rollback; restart before notify
through a real local HTTP receiver and signature check; handler broadcasts,
mentions, turn and quota effects; retention bounds; exhausted delivery retries;
and upgrade of an old delivery schema. Run `cargo test --workspace`, strict
Clippy on server/client/core, and `cargo fmt --all -- --check`.

The future signed v3 path must bind its receipt to exact verified sealed-record
bytes. This legacy projection must not be reused as v3 signature or byte-equality
validation. Durable signed mentions, seat/certificate authorization, request
nonce claims, CBQS/CBFS adapters, room lifetime policy, and durable client retry
queues are not implemented by this slice. Legacy live socket broadcasts remain
best effort; a disconnected consumer recovers committed history by sequence.
