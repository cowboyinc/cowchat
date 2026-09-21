# Local actor wake receiver

`cowchat actor-host` receives signed wake notifications and starts a configured program only when the room has work. Cowchat keeps the input, claim, reply, and completion in SQLite. There is no separate local inbox or reply database.

This is a local execution adapter. It does not verify Cowboy controller authority, charge an actor balance, or replace the hosted execution provider. `agent_id` here is the ordinary Cowchat connection identity, not a verified Cowboy identity.

## Run it

Start a local Cowchat server with private webhook destinations enabled (needed for the loopback receiver). Use a separate test database if an existing server is already running. Set the same `COWCHAT_ROOM_KEY` on participants and create an encrypted room.

Set `COWCHAT_WAKE_SECRET` to a secret shared only by this receiver and Cowchat. Then run:

```sh
cowchat --tcp 127.0.0.1:9229 --name FizzBuzz --agent-id fizzbuzz-actor \
  actor-host your-room --listen 127.0.0.1:9230 --mode addressed -- \
  python3 examples/python/fizzbuzz_actor.py
```

The receiver prints one JSON ready record containing the room and subscription IDs. The program gets one `ActorWork` JSON value on stdin, including decrypted `input.content`, and writes its reply as UTF-8 text on stdout. The example replies to an integer with FizzBuzz. A real adapter can invoke the harness or another configured inference provider. The executable and arguments come exclusively from local configuration; wakes and messages cannot choose a command.

With `addressed`, send a structural mention containing `fizzbuzz-actor`. The Rust client accepts that as the fourth argument of `send_message`. `always` handles all non-self chat messages; `listen` never starts the program. Thinking and system events do not start inference.

Restart with the same actor ID, room, listen address, mode, and wake secret. Enrollment is idempotent for that exact configuration. Changing it requires deleting the old subscription and enrolling again. The receiver checks queued work on startup, so a missed wake cannot strand an unclaimed message.

## Recovery behavior

- The webhook includes only room/subscription IDs. It is authenticated with Standard Webhooks HMAC and a five-minute timestamp window; the receiver also checks its configured room and subscription.
- Only the oldest unfinished input can be claimed. Claims expire after five minutes if a worker dies before storing a reply.
- If a reply was already stored, claim returns it immediately, even during the previous claim window. The client completes the work without invoking the program again.
- If the worker dies before append, inference can repeat. Stable reply IDs prevent a second visible reply. This does not make external actions exactly-once; the program must handle its own action idempotency.
- `process_actor_work` accepts a caller-owned async execution callback. It prepares one encrypted reply and checks server-validated completion after an append conflict.
- The local host retries program execution up to three times, then records `failed` and logs the work ID before continuing. The count is in memory; a host restart may repeat attempts. Transport/decryption errors leave work pending. An operator can also explicitly complete claimed work as `skipped` or `failed` through the client API. Such completion is ordered and recorded, not a silent cursor jump.

The local receiver binds only a fixed loopback address. A child runs for at most 240 seconds and may return at most 1 MiB. No child remains running between inputs. The receiver itself remains available for HTTP wakes; this is not evidence of hosted free dormancy or compute billing. It supplies only the current input to the program; conversation context and actor state belong to the chosen adapter/runtime.

## Verification

```sh
cargo test --locked -p cowchat-cli --test actor_host
cargo test --locked -p cowchat-server actor_work
```

The process test requires Python 3. It starts the real CLI receiver, uses three participants in an encrypted room, kills the receiver with SIGKILL, queues another input while it is absent, and verifies one reply after restarting it. Server tests separately cover database reopen, claim expiry, wrong identity, oldest-first completion, persistent history, and recovery between reply append and acknowledgment.
