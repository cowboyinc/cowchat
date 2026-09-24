# Hosted owner-stream startup

Build the production entrypoint with:

```sh
cargo build --release --locked -p cowchat-server --features hosted-bootstrap
```

`hosted-init` and `hosted-serve` use real chain proofs, a registered CBQS
provider/stream and private CBFS volumes. This feature includes no test broker
or standalone storage fixture. Ordinary `serve` remains local mode.

## Prerequisites

The operator provisions a funded owner, a CBQS account and active stream with
its Ed25519 admin public key, and a registered provider advertising a TLS
WebSocket endpoint. The owner must also have existing Cowboy CBFS CLI credentials,
DKG access, an owner delegation permitting mount-capable ReadWrite attachments,
and registered storage relays. Use the same wallet as the stream's proved owner.
Supply an independently trusted checkpoint file; do not treat an arbitrary RPC
checkpoint candidate as a trust root.

The node exports candidate bytes at `GET /cbqs/trusted-checkpoint-candidate`
(`application/octet-stream`). During provisioning, obtain them from an
independently pinned validator, check them against the deployment's genesis and
committee, and pin the approved file's BLAKE3 digest. Install that file before
starting Cowchat or CBQS. Do not fetch and trust a new candidate from the proof
RPC at each restart. A second URL to the same node does not establish independence.
The canonical checkpoint contains the chain-instance ID; its source is
`keccak256("cbqs/chain-instance/v2" || genesis_beacon[32] || chain_id_u64_BE)`.
It is distinct from the numeric chain ID and the network name. Keep the approved
anchor within 65,536 blocks of the head for the CBFS client's v1 proof horizon.

The node must serve finalized proof bundle v2 for the stream and provider under
system actor `0x17`, and bundle v1 for the existing CBFS client. The broker must
support the pinned v2 protocol, policy fencing and keyed lanes. Registration or
a green health check alone does not establish this compatibility.

The CBQS broker also requires `COWBOY_EXPECTED_RUNTIME_FINGERPRINT`, containing
the independently reviewed release manifest's activation fingerprint (`sha256:`
plus 64 lowercase hex characters). Provision it before starting the broker;
do not adopt the first node response as the expectation. The node must publish
matching typed `/chain-info.activation` metadata. Missing, mismatched or expired
agreement denies new sessions and mutations, including keyed lane allocation,
while already-admitted obligations can still settle. This is separate from the
trusted checkpoint and does not prove codec compatibility.

Create an existing private API-key file and a private file containing the
64-character hex Ed25519 admin seed. Neither key goes in command-line arguments
or configuration JSON. Keep these files owned by the worker user, mode `0600`,
with no symbolic or hard links. Hosted startup does not print either key.

Example configuration (replace every placeholder with actual provisioned data):

```json
{
  "worker_dir": "/srv/cowchat/worker-a",
  "cbfs_state_dir": "/srv/cowchat/cbfs-credentials",
  "rpc_url": "https://node.example",
  "trusted_checkpoint_file": "/srv/cowchat/checkpoint.bin",
  "owner_address": "0x<20-byte owner hex>",
  "chain_instance_id": "0x<32-byte instance hex>",
  "stream_id": "0x<32-byte stream hex>",
  "provider_address": "0x<20-byte provider hex>",
  "admin_key_file": "/srv/cowchat/admin.seed",
  "broker_url": "wss://broker.example/ws",
  "broker_pin": null,
  "archive_volume": "cowchat-archive",
  "control_volume": "cowchat-control",
  "api_key_file": "/srv/cowchat/api.key",
  "http_addr": "127.0.0.1:19440",
  "http_origins": ["https://dashboard.example"],
  "public_ws_url": "wss://chat.example/ws",
  "max_rooms_per_wallet": 100,
  "max_pending_rooms_per_wallet": 4
}
```

All filesystem paths must be absolute. The worker directory is created `0700`
under an existing parent, or checked for private ownership if it exists. One
process holds its lock throughout initialization/recovery/serving. Each worker
needs its own directory, intent journal and CBFS pending-commit/path-tag state;
the executable sets the SDK environment before starting any runtime threads.
Keep the worker path short enough for its Unix socket (less than 100 bytes with
`/server.sock`). Configuration rejects unknown fields.

By default the broker uses checked DNS/public IP routing. For a private River
route, set `broker_pin` to an object containing `address` (numeric `IP:port`) and
`certificate_der_file` (absolute path to its TLS root certificate). The SDK
retains TLS certificate/SNI validation for the exact advertised `broker_url`.
There is no plaintext or unchecked WebSocket option.

## First initialization

```sh
cowchat-server hosted-init --config /srv/cowchat/hosted.json \
  --reserve-wei <positive-reserve-per-volume> --erasure-k 2 --erasure-m 1
```

The explicit reserve is funded for **each** of two new private owner-key volumes.
Choose an erasure layout supported by the available relays. Both volume names
must be unused in the configured CBFS state. Only freshly created volumes are
initialized; a zero root on an existing volume is never accepted as genesis.
A partial initialization preserves created volumes and fails; inspect them and
choose new names for another attempt. Initialization does not acquire a writer
epoch or open a listener.

## Serve and promote

```sh
cowchat-server hosted-serve --config /srv/cowchat/hosted.json --expected-epoch 0
```

`0` is the expected control epoch only immediately after first initialization.
Startup verifies finalized stream/provider authority and the local admin key,
opens the private control volume, and attempts exactly one expected-epoch CAS
with a fresh worker identity and stable claim ID. It obtains a matching signed
broker fence acknowledgement, then opens the archive and recovers history and
local pending intents before constructing the server. Losing or ambiguous
promotion fails closed; it does not automatically compete at a newer epoch.
On a later start the operator must supply the observed current control epoch.
Do not blindly increment the argument after an error: a previous attempt may
have committed its claim even if its reply was lost.

The authenticated primary API key is bound to this one owner. HTTP/WebSocket
and the private Unix socket require authentication. There is no TCP listener,
public key signup, open-auth mode or SQLite room-history fallback. The SQLite
files hold local identity bookkeeping and pending intents, not hosted history.
The chat-first surface supports private encrypted room creation, send, history,
list, join and leave; unsupported coordination/REST paths fail closed.

## Credential renewal and remaining live gate

`grant_ttl_seconds` controls each CBQS grant's lifetime (default 86400 seconds,
range 600 through 86400). There is no process-lifetime cap. The hosted owner
renews at half the remaining credential lifetime (about every 12 hours by
default), opening a fresh checked/pinned session at the same writer epoch
without the writer lock, then swapping it in without fencing. Replay cursors
use fresh subscription IDs. CBFS control and archive owner tokens renew using
the access mode each volume was opened with.

The runtime retires reads and writes 30 seconds before the earliest grant,
owner token, or wallet-signed delegation expiry. Transient renewal failures
(broker unreachable, timeout, token mint error) are logged and retried with
bounded backoff; the horizon still expires independently of blocked writes or
renewal I/O. A broker refusal that no retry can clear (stream not active,
authorization generation stale, invalid grant, policy epoch stale) or a
rejected swap retires the worker at once. Every 15 seconds the owner also
makes one read on its current session, so a fenced or revoked writer retires
within that interval even when idle. In each case hosted-serve exits non-zero. The 30-day
wallet-signed CBFS delegation cannot renew in process: an error is logged
within its final 24 hours. Arrange operator renewal and a controlled restart
before that ceiling. A restart still requires explicit promotion and recovery;
a service manager must not guess the next epoch. Credential refresh adds no
persistent state and does not change intent staging, commit, or crash replay.

Local boundary tests do not establish River E2E. The live acceptance run must
use this executable against finalized chain authority, real CBQS and CBFS:
create an encrypted room, join from another authenticated client, send and read
history, reconnect/retry the identical prepared ciphertext, then recover on a
new worker directory and verify the acknowledged history. Client ACK and
participant visibility must continue to follow durable CBFS publication.

The opt-in acceptance client starts no server or storage fixture:

```sh
COWCHAT_HOSTED_E2E_MODE=create \
COWCHAT_HOSTED_E2E_URL=ws://127.0.0.1:19440/ws \
COWCHAT_HOSTED_E2E_KEY_FILE=/srv/cowchat/api.key \
COWCHAT_HOSTED_E2E_STATE=/srv/cowchat/acceptance.json \
cargo test --release --locked -p cowchat-server --features hosted-bootstrap \
  --test hosted_live -- --ignored --nocapture
```

The `create` phase creates a new private state file containing the exact room,
encrypted payload, sender identity and room secret before mutations. Preserve
that file; it contains test credentials and must not be published. A locked
second client checks ciphertext history, while the sender checks decrypted
receipts, stable retries and reconnects. The normal 10-second client request
timeout is unchanged; a slow archive fails this run rather than silently raising
the threshold.

After a successful `create` run, stop the first worker and start a separately
configured worker with a **fresh worker directory**, the same owner/volume
handles and an explicit observed control epoch. Rerun the command with
`COWCHAT_HOSTED_E2E_MODE=recover`, preserving the state file and using the new
endpoint. This phase reads the expected message before retrying any operation,
then confirms the same receipt and one-message history. Retain successful logs
from both phases and the server bootstrap logs with the concrete node/broker
builds. A failed create run followed by a passing recovery run is not proof that
an acknowledged message survived. These two phases prove a fresh-worker restart
only for the topology actually run; use distinct hosts to claim cross-host
failover. Neither phase establishes broker-host HA.
