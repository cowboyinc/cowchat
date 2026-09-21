# Telegram bridge

`cowchat telegram-bridge` maps one operator-selected numeric Telegram chat ID to
one encrypted Cowchat room. Telegram is the first adapter on the shared
`bridge.rs` room encryption, pending-append recovery and outbound replay core.
The adapter owns Telegram polling, update filtering, offset handling and text
chunking. The core accepts normalized events without requiring a polling API;
no webhook receiver or other provider adapter is implemented. It polls Telegram and the room directly. It does not
use Labs builder APIs, Postgres, gateway routes, CBSS, or a Cowboy execution job.
The bridge has its own stable Cowchat participant ID; Telegram sender IDs are
labels inside encrypted message content, not verified Cowboy identities.

The bridge and Telegram receive plaintext. Choosing this mapping authorizes
forwarding every ordinary room message posted after the bridge's initial start
to that Telegram chat, including actor replies. Cowchat's service and the local
cursor file receive ciphertext. This is not end-to-end encryption against
Telegram or the machine running the bridge. Ensure all intended room participants
understand the configured bridge before enabling it.

## Configure and run

Use a dedicated Telegram bot with no existing webhook and one bridge process.
`getUpdates` consumes the bot's whole update stream: updates outside the configured
chat are ignored and acknowledged, so this bot must not serve unrelated mappings
or applications. The implementation never deletes an existing webhook for you.
Telegram may restrict which group messages the bot can receive; configure its
permissions/privacy mode for the intended chat.

Supply `TELEGRAM_BOT_TOKEN` and `COWCHAT_ROOM_KEY` through your usual protected
environment. Do not put the bot token in command arguments, logs, or room messages.
Use the existing Cowchat access-key configuration for a shared service. Create
an encrypted room and arrange access before running the bridge.

```sh
cowchat --url wss://your-room-service/ws \
  --name Telegram --agent-id telegram-project-room \
  telegram-bridge ROOM_UUID \
  --chat-id=-1001234567890 \
  --state-file /your/private/state/telegram-project-room.json \
  --mention YOUR_ACTOR_ID
```

The parent state directory must exist. `--mention` is optional and repeatable;
these fixed actor IDs are addressed by incoming Telegram text. Telegram text
cannot change the destination room, access credentials, recipient chat, or actor
routing. Bots, non-text updates, edited messages, and other chat IDs are ignored.
Only text is supported; attachments, reactions, topics and rich formatting are
not implemented. Remote Telegram requests use the fixed HTTPS Bot API endpoint,
with redirects disabled. This command provides no custom API endpoint option.

## Recovery

On first start, the room cursor begins at its current tip: earlier room history
is not exported. Pending Telegram updates may still arrive. Preserve the state
file and reuse the same mapping/identity when restarting. The file contains a
configuration digest, Telegram offset, room sequence and at most one encrypted
pending append. It contains no plaintext transcript, bot token or room key. A
sidecar OS lock prevents two processes from using the same file; writes use a
synced temporary file and atomic rename.

Before appending an inbound message, the bridge persists its ciphertext and
stable message ID. A restart retries those exact bytes. Only a confirmed append
advances the Telegram cursor. An ID conflict remains an error: it is not treated
as proof that this bridge's message was stored. Retargeting a state file to a
different endpoint, room, chat, bot identity, Cowchat identity/access key or actor
routing is refused. A wrong room key is rejected against recent encrypted history
when such history exists. Empty rooms cannot provide that validation.

Outbound delivery is at least once. The room cursor advances only after Telegram
accepts all chunks of a message. A crash after Telegram accepts a message but
before the cursor is saved can duplicate it; a retry of a partially sent long
message can duplicate earlier chunks. Telegram has no client-supplied deduplication
key for `sendMessage`. Rate-limit responses back off without advancing the room
cursor. A decryption error pauses forwarding and reports the blocked sequence to Telegram
once, without sending ciphertext or advancing the room cursor. Restore the correct
room key and restart with the same state file. If a particular message is invalid,
restart with `--skip-room-seq N` to skip only that unreadable sequence. The bridge
sends a visible gap notice before advancing; it never skips readable messages or
other sequences. A skip notice can duplicate after an interrupted checkpoint, just
like other outbound messages. The original ciphertext remains in Cowchat history.
Remove the skip flag after recovery. This explicit override also permits startup
when no recent message can be decrypted, but all other unreadable sequences still
block. Do not use it as a substitute for restoring the correct key. Operators should retain the state file: deleting it changes
the replay boundary and can cause duplicates or skipped outbound history.

Telegram retains unconsumed updates for at most 24 hours. Its API supports
long-poll offsets and a 4096-character text-message limit; this bridge splits text
conservatively and disables link previews. See the official
[Bot API](https://core.telegram.org/bots/api#getupdates) and
[sendMessage reference](https://core.telegram.org/bots/api#sendmessage).

## Verification status

Local tests use a real Cowchat server and a fake Telegram HTTP API. They verify
ciphertext-only inbound storage, sender/chat filtering, configured actor mentions,
exact-byte recovery after append-before-checkpoint failure, 429 handling without
cursor loss, outbound target restriction, Unicode chunking, file locking, and
refusal to acknowledge a message ID occupied by another participant, and explicit
single-message poison recovery without silently skipping wrong-key messages. No real bot
or Telegram chat has been connected by this implementation task. Real Telegram
acceptance still needs the intended bot, chat mapping and participant authorization.
