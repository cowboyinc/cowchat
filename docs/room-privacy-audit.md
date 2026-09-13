# Room flow logging and persistence audit

2026-09-13. Bounded review requested in `dashboard-impl` message 1673. Baselines:
Cowchat `6f025f6` (code `09e74f4`), gateway `6afd166`, harness `8a53482`, in the
three worktrees recorded in [the cohort inventory](dashboard-implementation-cohort.md).
The changes described below are a local review cut, not production clearance.

## Findings and changes

1. **Callback credentials could be copied into durable retry errors and logs.**
   `cowchat-server/src/webhooks.rs::process_delivery` formatted a complete
   `reqwest::Error`. A failed callback request can include its URL path/query in
   that string. The worker both saves it in `subscription_deliveries.last_error`
   and logs it on retry/exhaustion. Replace that diagnostic with fixed timeout,
   connection-failed or request-failed categories. Numeric non-success HTTP
   statuses remain; response bodies are not inspected. Retry/ack behavior is
   unchanged. The fix covers the shared worker, including legacy subscriptions.
2. **Gateway proxy diagnostics could disclose runner endpoint credentials.**
   `gateway-server/src/lib.rs::proxy_to_runner` logged a rejected raw endpoint
   (potentially userinfo/query), and logged full target URLs on request failure
   and response-size rejection. Log the public actor plus static reason instead.
   Also replace the raw reqwest error forwarded through `build_streaming_response`
   with a static body-stream error, so framework error reporting cannot expose
   its transport error chain. Status codes, headers, response streaming and the
   size cap remain unchanged. This is generic proxy behavior, not Telegram logic.

No existing retry rows are scrubbed by this change. Previously persisted URL
credentials may remain in the database, WAL or backups. No production database
or operational secret was inspected or modified. This review does not establish
that a real credential has leaked. Any confirmed exposure needs its own bounded
rotation/remediation decision; silently deleting retry history is not this fix.
For an affected deployment, rotate exposed callback tokens and clear obsolete
`last_error` values through the approved rollout; retained backups/logs need
their own handling. This review performs none of those operational actions.

## Sink inventory for the actual seated-room path

Paths below are relative to each repository's `crates/` directory unless stated
otherwise. Public identifiers and routing metadata are not message plaintext,
but still reveal participants, activity and relationships.

| Boundary / sink | What reaches it and limits |
|---|---|
| Cowchat `cowchat-server/src/store/seated.rs`, `store/seated/` SQLite writes | Encrypted `cow1` message bodies, authenticated headers/signatures and signer-seat metadata; public certificates, signatures, membership context, proof floors, nonce and operation receipts. HPKE delivery stores signed scope and ciphertext, never an unwrapped room key. These conclusions apply to seated rooms; legacy room storage is not an encrypted-room guarantee. |
| Subscription configuration and delivery journal | Callback URL and raw webhook HMAC secret are intentionally stored in `subscriptions`. Durable wake bodies contain immutable message/dispatch pointers, not message bodies. Retry status stores the fixed diagnostics above. This database is sensitive configuration, not secret-free storage. |
| SQLite files and sidecars | `Store::open` creates/repairs owner-only database permissions and hardens existing WAL/SHM files; `auth.rs` creates/hardens the containing directory on Unix. This protects filesystem access, not encryption at rest, copied backups, process memory or arbitrary deployment tooling. |
| Cowchat service/worker logs and HTTP errors | Seated handlers return status codes for internal failures; the worker logs IDs, attempt/result categories and database failures. Current bound SQL does not interpolate private values into application log text. Callback URL errors were the concrete duplication found above. Background proof refresh handles public proof data and fixed fetch errors, not identity private keys. This is source review, not evidence about third-party production log agents. |
| API-key CLI / startup | Startup reports paths and bind configuration, not key values. Explicit administrative `auth show`/rotation commands print the API key by design; they are secret-output commands outside the room request path. Shell capture of those commands remains sensitive. |
| Gateway request context and proxy | Headers/query and raw wake body are forwarded transiently through the existing route-context/body mechanism. The audited proxy does not write them to a journal or log. The fixture wake is a pointer notification; provider message text is not passed through it. Arbitrary runner response bodies are still relayed by design. Optional payment routes and other gateway targets are outside this audit. |
| Harness `harness-serve/src/cowchat_wake.rs` | Binding stores opaque key handles and public seat/routing data. Raw webhook verification precedes lease/read; only an authenticated encrypted trigger is decrypted for the runtime callback. Seed, room secret and plaintext buffers owned by this handler use `Zeroizing`; errors become status codes. There are no logging, session, event, memory or filesystem sinks in this handler, and the binding/lease have no derived `Debug`. |
| Harness runtime boundary | The concrete production `CowchatWakeRuntime` is still absent. The trait receives plaintext and can copy/persist it: the handler cannot enforce privacy inside an arbitrary provider. Current room handling bypasses the normal agent route/session pipeline. This does not clear future model calls, Cattle Guard, transcript/run-event persistence, memory extraction or a key provider. |
| Harness/room/gateway fixture stdout, stderr and files | Public fixture seeds only. Harness execution JSONL contains message/dispatch IDs and an execution event; it ignores the trigger text. Its after-append crash marker is empty. Room fixture output reports IDs/counts/verification flags; owner-context file is public certificate context. SQLite and reports contain ciphertext/pointer metadata. The driver captures stderr and includes its tail on failure, so a future fixture must not print private inputs there. |
| Fixture argv and environment | Bind/origin URLs and artifact paths are arguments; fixture key bytes are constants, not argv/environment inputs. The driver inherits the parent environment and is not a general environment scrubber. Release-checkpoint build inputs are public trust anchors, not room keys. No operational environment or key file contents were dumped for this review. |
| Process termination / host diagnostics | Local recovery uses service SIGKILL and harness exit 99 after append. Application artifacts are covered above. Zeroization on ordinary drop does not establish protection from OS core dumps, swap, debuggers, abrupt termination, provider copies or crash-report uploads. Host production policy remains a deployment concern. |

## Regression evidence

- Real worker test
  `seated_worker_transport_failure_does_not_persist_callback_url_credentials`:
  loopback callback with path/query canaries fails HTTP parsing; an actual reqwest
  diagnostic demonstrably contains the token, while the worker persists exactly
  `transport request failed`. It preserves the URL in its intended configuration
  field and retains retry behavior. No real credential is involved.
- Actual proxy test
  `runner_proxy_diagnostics_never_echo_endpoint_or_request_credentials`:
  invalid credential-bearing endpoint, failed request, oversized response and
  truncated body. A tracing subscriber must capture each expected warning;
  assertions reject URL/request/header/body canaries in captured diagnostics and
  inspect the streaming error chain. Static 503/502 responses and the body error
  remain observable.
- Cowchat full server battery: 151 library, 71 integration and six binary tests;
  strict all-target Clippy on Rust 1.93. Gateway's 14 focused proxy tests pass.
  Formatting and diff checks pass in both edited worktrees. Gateway all-target
  Clippy passes with `--no-deps` and command-line allowances for
  `result_large_err`, `useless_conversion`, `too_many_arguments` and `useless_vec`.
  This is not an unqualified strict pass: without allowances, existing payment
  test helpers in `mpp.rs`/`x402.rs` and a `payment_state.rs` vector are lint errors;
  dependency-inclusive strict checking also fails on an existing unused
  `gateway-cbfs` field. These source locations are unchanged at the baseline;
  no unrelated lint cleanup or source suppression was added.

The earlier three-process recovery proof remains the flow evidence; it was not
rerun just for diagnostic changes. Its known-plaintext artifact scan is not a
general key/logging audit. These changes add no consensus state, per-message
chain admission, new key provider, deployment or external-provider call. The
remaining production privacy boundary is the future runtime/tool/session and
door integration described in the cohort inventory, not an assertion that the
whole dashboard is now private.
