# River archive latency probe

This explicit, ignored test measures actual authenticated Cowchat TCP client
acknowledgements using the local real CBQS broker fixture and **River's real
chain-backed CBFS**. It separately measures the control-volume promotion CAS.
It does not deploy Cowchat, install a custom broker on River, or claim deployed
CBQS authority, cross-host failover, or production latency.

The probe uses the existing pinned CBFS CLI library for private-volume
registration, owner authentication, key wrapping, relay discovery, and manifest
Stage/Finalize. There is no standalone/Sled authority option. It registers two
new private owner-key volumes with random run-specific names, initializes their
archive/control records only after successful registration, and keeps the
control volume separate from the archive volume. A new worker holder and a
matching signed local broker fence follow each measured allocation.

## Prerequisites

- A dedicated disposable River box with healthy validator, registered reachable
  CBFS relays, and the required CBSS committee. Run River `doctor` first.
- A funded disposable owner with a committed account DKG, working CBFS delegated
  authentication, registered RAS delegation, and the owner wallet discoverable
  by the existing CBFS CLI. Use local credential paths; do not paste keys into
  chat or put them in the result artifact.
- A trusted finalized checkpoint pinned independently by the River operator.
  The pinned CBFS CLI requests proof v1; the node must still serve that wire.
- Network access to the RPC, write relayer, and advertised relay QUIC addresses.
  Run the probe inside River's network if those addresses cannot be reached
  from the laptop. No SSH tunnel can carry QUIC by itself.

Supply an explicit JSON configuration, with absolute paths:

```json
{
  "cbfs_state_dir": "/absolute/private/river-cbfs",
  "rpc_url": "http://RIVER_RPC:4000",
  "trusted_checkpoint_file": "/absolute/private/river-checkpoint.bin",
  "erasure_k": 2,
  "erasure_m": 1,
  "initial_reserve_wei": "REPLACE_WITH_OPERATOR_SIZED_DECIMAL_WEI",
  "samples": 30,
  "promotions": 10,
  "output_file": "/absolute/results/cowchat-river-run.ndjson"
}
```

`initial_reserve_wei` is funded into **each** volume; 1 CBY = 1,000,000,000 wei.
Size funding from the actual River costs. The output must not already exist.
The run creates paid chain state; it is never part of ordinary automated tests.

```sh
COWCHAT_RIVER_PERF_CONFIG=/absolute/private/river-perf.json \
  cargo test --locked --release -p cowchat-server --features river-perf \
  river_archive_perf --lib -- --ignored --nocapture --test-threads=1
```

No `COWCHAT_TEST_CBFS_NODE` is needed: this probe does not launch local storage
nodes. The SDK test-support feature isolates local pending journals only; all
storage traffic and authority are the real River services.

## Interpreting the output

The private NDJSON file records a `started` row, setup phases, every successful
promotion/send sample, and a `complete` row only after all checks pass. A returned
error records `failed`; a crash may leave only the last phase. Neither is a
completed distribution. Do not report success-only percentiles from either.

- `control_cas_ms`: the real private control-record update through verified
  readback. It excludes the following local CBQS fence/session admission, which
  is reported separately as `local_fence_session_ms`.
- `archive_gated_client_ack_ms`: one pre-encrypted 1 KiB message, timed from
  client send through the actual archived success reply. This includes local
  journal, local broker, replay, River archive publication, projection and reply.
  Encryption/preparation and the follow-up reader check are outside the timer.
- Every sample is checked through a second authenticated client's decrypted
  history. An exact same-ID/ciphertext retry must return the original receipt;
  final history must contain no duplicates.

The workload is sequential, one message per batch, with no discarded warmup.
Percentiles use nearest rank and always include `n`; a 30-sample p99 is its
maximum, not a well-established tail estimate. The real client's current
10-second request timeout is retained: crossing it fails the run rather than
quietly raising the ceiling or removing the sample. This is not a saturation
benchmark or a throughput claim.

Record the River service pins, doctor result, host location and artifact beside
any reported numbers. Never combine the old local CBQS latency figures with
these samples and call the sum measured end-to-end latency.

The two named volumes remain on the disposable River box for inspection. Collect
the artifact, then coordinate teardown with the River owner or retain its
explicit TTL. Do not use this probe against a shared network.
