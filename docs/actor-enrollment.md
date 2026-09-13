# Actor enrollment proof boundary

Actor certificate matching, finalized-control proof fetching, and
`POST /rooms/{room_id}/actors` are implemented. The endpoint accepts the same four
base64 certificate/signature fields as owner enrollment, plus signed-request
headers under the actor's delegated key. It verifies room-owner membership and
request possession before fetching any proof. The final transaction independently
rechecks room state, proof freshness, certificates, and request signature, consumes
the nonce, installs credentials, and advances the local proof rollback floor.
An exact retry with a fresh request nonce preserves the existing installation.

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
production network checkpoint is included in this slice. `serve --http ...
--actor-proof-rpc <origin>` enables the endpoint's proof authority in such a build;
without that runtime option the actor enrollment endpoint returns unavailable.

Before shipping: embed independently verified production checkpoint bytes and
their digest at build time, configure the intended proof RPC, and verify that the
resulting proof identifies the intended chain instance and a fresh finalized tip.

The configured RPC is a courier, not the trust anchor. Requests have a ten-second
timeout, refuse redirects, and bound the response by the protocol proof limit.
The authenticated target timestamp must be at most sixty seconds old and no more
than five seconds in the future. This is a bounded freshness policy, not an
assertion that no control update can occur after the read. Returned verified
control values have private fields. The enrollment transaction rechecks freshness
and room generations and rejects proof height regression, a conflicting hash/root
at the same height, changed chain instance, actor authorization regression, or
a changed certificate commitment without advancing actor authorization. The
height/hash/root/generation/commitment floor is local SQLite state. A newly proven
higher actor authorization generation invalidates older actor credentials across
rooms on the same chain. This does not yet provide passive chain-revocation
subscriptions; a later enrollment/refresh is what supplies the new proof.

History and initial subscription backfill exclude records from key generations
before membership's `from_gen`. History advances its cursor past excluded records
so a bounded page can reach the authorized range. No room key is stored here;
key delivery must also enforce that floor when it is connected.

```sh
cargo test --offline --locked -p cowchat-crypto --test actor_membership
cargo test --offline --locked -p cowchat-server --lib actor_control_fetch
cargo test --offline --locked -p cowchat-server --lib actor_http_enrollment
```

The second test fetches an actual threshold-signed finality and QMDB inclusion
proof over local HTTP and rejects the wrong actor, altered proof, stale/future
timestamp, and malformed control value. Its chain and checkpoint are synthetic
test fixtures. It does not prove Canyon configuration, a live deployed actor,
secret acquisition, or runtime execution. The enrollment HTTP test uses distinct
owner and actor keys and the real proof verifier, injects installation failure to
check complete rollback, exercises exact retry and preflight/install state races,
rejects an older proof than the local floor, and appends/reads/decrypts an
authenticated actor record while excluding an older key generation. The actor
record is written by the test client; wake-triggered actor execution through a
generic gateway remains to connect. All proof reads belong to enrollment;
room append, history, and wake delivery continue against local service state.
