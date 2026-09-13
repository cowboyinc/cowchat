# Recovering a room reply

`SeatedHttpClient::find_reply_with_signer(trigger, signer)` derives the existing
stable reply ID from the room, chain, seat and trigger, then performs one signed
exact-message history read. A restarted caller needs no candidate plaintext,
room decryption key, stored ciphertext or new append. The installed signer checks
current room authority at the HTTP signing point, as it does for other seated
reads. Callers must also fence response consumption with their current lease;
the client holds no authority guard while waiting for HTTP.

A returned `AuthenticatedReply` has private fields and exposes only the message
ID and positive service position. The client verifies the encrypted record's
signature and its exact room, chain, seat, role, certificate, key generation,
message class, trigger and stable reply ID. Conflict reconciliation after an
append uses the same record validation. Only the currently installed certificate
and key generation are accepted; historical-key recovery is not implemented.

An empty successful history page returns `None`. Malformed pages, invalid
signatures, wrong reply identity and refused or failed reads return errors.
Neither absence nor an error authorizes another model invocation. A coordinator
must combine the authenticated reply with the configured journal's accounting
facts and current room authority before completing a run. The service position
is not an independently verified archive receipt, and this lookup does not
finish accounting or establish final SENT.

For an authorized new execution from a durable reference,
`prepare_reply_for_trigger` seals using the installed seat and the same stable
reply identity. It requires a canonical nonzero trigger UUID and verifies the
resulting signature against the installed seat key. The existing `prepare_reply`
entry retains its verified-wake room/generation checks and delegates sealing to
this method. Neither method grants access or permission to run a model: the host
must first refetch and authenticate the trigger, obtain its paid permit, and
carry the original current lease through sealing and submission. Recovery of an
already dispatched attempt still uses lookup, not a replacement candidate.

Tests cover a real service read before append and after a lost append
acknowledgement with a recreated client. Adversarial HTTP tests cover malformed
pages, a forged signature, a validly signed different reply, duplicate rows,
invalid positions, denied reads and signing-time revocation. No live service,
consensus, provider, production key release or accounting completion is involved.
