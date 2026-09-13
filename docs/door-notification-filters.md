# Door notification filters

An already authenticated door seat receives `message` and `system` notifications
without a mention gate. It receives ordinary messages even when `wake_hint` is
`none`, because notification here mirrors a room into an external destination;
it does not authorize actor or builder computation. `thinking`, presence and
tool traces are not mirrored. The current record codec does not yet accept
`data`; this change does not add that record class.

The profile comes from the stored, authenticated credential's role and door
binding. There is no caller-provided `role`, arbitrary filter override, provider
allowlist, or permission to broaden an actor's subscription. Owners, actors and
builders retain `message` + exact seat mention + nonzero wake hint. A door
context must contain its authenticated door kind, bound external sender and
forwarded owner seat. Production issuance of that context remains separate work.

Live append and initial backlog now select seated subscriptions using the same
role-derived predicate that checks retained deliveries during repair and dispatch.
The existing legacy matcher still applies unchanged to non-seated subscriptions;
its blanket system/quiet exclusions no longer override authenticated door policy.
The seated predicate also checks current room authorization/transport generation,
credential binding and readable key-generation floor. Dispatch still checks
current read membership/expiry and the existing revision fence.

A door never receives a notification for a record whose authenticated signer is
that exact door seat. The service records `signer_seat` after verifying append
signature and membership, so this rule also covers a door forwarding a record
attributed to the owner. Another door with the same provider kind is a different
seat and still receives it. Display names, `via: slack` or another provider tag,
and an untrusted origin label cannot suppress another destination's traffic.

This is a notification filter, not a destination delivery implementation.
A worker that backfills an entire sequence range must independently apply its
outbound class/origin policy to every record before posting externally; a
content-free wake can span records that do not match the subscription. The door's
own destination outbox, idempotency, lost-ack handling and origin persistence
still need implementation. No external message is sent by these changes.

The existing subscription endpoint/body and signed lifecycle remain unchanged.
`after` omitted starts at the tip; an explicit earlier cursor requests readable
backlog. Repair retains exact eligible delivery IDs and pointer bodies, abandons
records outside the current profile/floor, and never restores removed authority.
The transport and local request/revision machinery do not write consensus state.

## Proof and remaining dependencies

Tests seed explicitly authenticated door contexts, create signed subscriptions
(including an actual HTTP call), and exercise real signed encrypted appends,
live enqueue, backlog, restart, exact-door/forwarded-owner echo suppression,
same-provider distinct destinations, quiet messages, thinking exclusion,
read-floor repair, immutable retained wake bodies, and removed credentials.
The future system-record writer is represented only by a marked trusted fixture;
ordinary member appends still cannot create system records.

No production door enrollment factory exists yet: the actor factory intentionally
accepts only actor-role credentials. These tests are not a live door/PKE-release
or external-provider integration proof. Generic door enrollment, authenticated
system writer, production secret release and the destination worker/route handler
remain prerequisites. The existing actor mention-wake handler is not repurposed
as a door executor by this change. Providers use the same filter logic regardless of Telegram,
Slack, SMS, WhatsApp, or another catalog entry.

Run `cargo test -p cowchat-server --lib door_` and the existing
`seated_subscription_default_filters` regression with Rust 1.93.
