# Actor enrollment proof boundary

The actor certificate matcher and finalized-control proof fetcher are implemented.
The HTTP actor enrollment endpoint and transactional installation are not connected
yet. This document describes the implemented boundary and the remaining work.

The matcher checks two separate authorities: the actor controller signs the actor's
identity certificate, and the room owner signs its membership certificate. The
identity certificate must match the commitment and authorization generation in
the actor's authenticated control record. Room membership and encryption-key
generations are separate. Membership does not authorize paid invocation.

`actor_proof::ActorProofAuthority` fetches `POST /proof/finalized-state` from a
service-configured RPC, requests exactly the actor and `__cowchat/control/v1`, and
calls the existing `cowboy-protocol-light-client` verifier. The proof must contain
the exact key under the actor's address. Its canonical CBOR value has three fields:
`controller` (20-byte byte string), `certificate_commitment` (32-byte byte string),
and `authorization_generation` (unsigned integer). This is newly specified actor
application data; its deployment has not been established.

The release build may supply `COWCHAT_RELEASE_CHECKPOINT_PATH` and
`COWCHAT_RELEASE_CHECKPOINT_SHA256` together. The build embeds the checkpoint bytes
and expected digest into the binary. The loader validates the digest and canonical
checkpoint. No runtime path or HTTP-provided checkpoint can change that anchor.
Without these build inputs, the authority constructor refuses enrollment. No
production network checkpoint is included in this slice.

The configured RPC is a courier, not the trust anchor. Requests have a ten-second
timeout, refuse redirects, and bound the response by the protocol proof limit.
The authenticated target timestamp must be at most sixty seconds old and no more
than five seconds in the future. This is a bounded freshness policy, not an
assertion that no control update can occur after the read. Returned verified
control values have private fields; the enrollment transaction must recheck
freshness, room generations, and a durable local height/hash/root rollback floor
before installing credentials. History must enforce membership's `from_gen`.
Those installation checks remain to be wired.

```sh
cargo test --offline --locked -p cowchat-crypto --test actor_membership
cargo test --offline --locked -p cowchat-server --lib actor_control_fetch
```

The second test fetches an actual threshold-signed finality and QMDB inclusion
proof over local HTTP and rejects the wrong actor, altered proof, stale/future
timestamp, and malformed control value. Its chain and checkpoint are synthetic
test fixtures. It does not prove Canyon configuration, a live deployed actor,
secret acquisition, or an actor reply. All proof reads belong to enrollment;
room append, history, and wake delivery continue against local service state.
