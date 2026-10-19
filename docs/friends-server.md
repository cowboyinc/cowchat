# Social sign-in configuration

Social accounts run only in the standalone SQLite server with authentication enabled. Cowboy broker mode and `--no-auth` do not enable social APIs. Existing local keys and room grants keep their existing behavior.

Configure the server process with an HTTPS origin and at least one provider.

```text
COWCHAT_SOCIAL_PUBLIC_ORIGIN=https://chat.cowchat.cowboy.inc
COWCHAT_GITHUB_CLIENT_ID=<GitHub OAuth app client ID>
COWCHAT_GITHUB_CLIENT_SECRET=<GitHub OAuth app secret>
COWCHAT_GOOGLE_CLIENT_ID=<Google web OAuth client ID>
COWCHAT_GOOGLE_CLIENT_SECRET=<Google web OAuth secret>
```

Register the exact redirect URLs below with each provider. The origin is configured by the operator. Incoming Host and forwarded headers cannot change OAuth redirects.

```text
https://chat.cowchat.cowboy.inc/auth/social/github/callback
https://chat.cowchat.cowboy.inc/auth/social/google/callback
```

Keep secrets in the service environment with restrictive permissions. Do not commit them. The server never stores provider access tokens. Google access tokens are checked against tokeninfo for the configured client audience and positive lifetime before userinfo is accepted. Provider HTTP redirects are disabled.

The native app opens an ephemeral loopback listener before starting sign-in. Start supplies its port and a SHA-256 PKCE challenge. The provider callback redirects a single-use completion code to `http://127.0.0.1:<port>/cowchat/oauth`. Exchange requires that code and the original verifier. A remote initiator who sends their authorization URL to another person cannot poll out the other person's credential. Linking identities commits only after the native exchange succeeds. Email addresses never merge accounts.

Device sessions expire after 30 days. Agent credentials expire after 365 days. Explicit logout and agent revocation invalidate active delivery and reconnect authorization. All social bearer credentials have the reserved `csoc_` prefix and are stored as SHA-256 digests. Invalid or revoked credentials cannot fall back to legacy key, local, public-room, or `--no-auth` access.

Friendship does not grant room access. Each v1 collaboration room has exactly its creator and one invited friend. The invited person chooses their own agents. Agent access is assigned to an immutable server-created ID and requires both owner membership and that exact room admission. Another device or sibling agent cannot assume that ID. Additional people and Slack connections are unavailable in these rooms for v1. Existing approved rooms survive removing a friendship. Remove an agent admission or leave the room to end its access.

Social credentials cannot create legacy rooms, redeem or mint legacy invites, or create webhook and actor subscriptions. Room metadata, messages, history, blobs, artifacts, live delivery, and reconnect delivery use managed-room authority before legacy visibility. Account-owned agent credentials cannot approve friendship or room invitations.

OAuth attempts expire after ten minutes. Pending attempts are capped at 20 per source and 1000 globally. The existing proxy-aware session limiter also limits sign-in starts. Each account can have up to 100 active agents, 100 pending outbound friend requests, and 100 owned collaboration rooms. These limits bound the initial implementation; deployment still needs normal upstream HTTP rate limiting.

## Check the native callback contract

On macOS, run `scripts/test-social-contract.sh` from the repository root. It runs the real HTTP OAuth tests, captures their server-generated callback URL, and passes that URL through the Mac's production callback parser. The test also rejects a callback for another request. All provider accounts and credentials in this check are disposable local fixtures.
