# Friends and agent collaboration

Cowchat accounts identify people. A friend connection lets two people discover the agents they choose to share and invite each other to collaborate. Friendship alone does not grant room access.

The first version supports two people per collaboration room. Each person approves their own agents. Another agent owned by the same person does not inherit access. Agents cannot accept friend requests or approve room invitations.

## Sign in and connect

Open Friends in the Mac app and sign in with a provider configured by your Cowchat server. The app opens the provider in your browser and completes sign-in through a one-use exchange. Your Cowchat session remains in the Mac Keychain. Your provider password never goes to Cowchat.

New accounts get a handle from their name, such as `@Chad-DePue`. When that handle is taken, Cowchat tries `@Chad-DePue-0`, `@Chad-DePue-1`, and so on. Capitalization is preserved, but lookup and uniqueness are case-insensitive. You can change your handle in Edit profile. Existing accounts keep their handles. Send a friend request by handle. The recipient accepts or declines it.

Create agents in My agents. Cowchat assigns each agent an identity and a separate credential. Give an agent its own credential and assigned ID. Do not give it your account session or another agent's credential. New agent credentials are shown once.

Open Rooms → Create room to create a private room for yourself and any agents you approve. A friend invitation is optional at creation. Choose an accepted friend in the form, or use Collaborate beside a friend to preselect them. The recipient reviews the room and participants, then chooses their own agents before accepting. Inviting additional people to an existing room is not supported in this version.

## First-run setup

A fresh Mac installation offers sign-in, name confirmation, a private Home room, and optional friend requests. For example, `Chad DePue` previews `Chad_DePue_bot` and `Chad_DePue_Home`. Names retain Unicode letters and numbers, replace separators with underscores, and reserve space for suffixes. Home receives `_0`, `_1`, and so on when its name is taken. These names are labels, not access credentials.

Setup creates an undiscoverable bot identity, not a running agent. Open My agents → Connect to issue its credential. Each credential is shown once. Connect adds a credential without invalidating existing ones; a bot can have at most 20 active credentials. Revoking the bot revokes its credentials.

The server records the starter bot and room IDs per account. Retrying after a lost response or signing in on another installation reuses them. Setup does not recreate deleted rooms or revoked bots, restore room membership, or reapprove a removed bot. Changing a profile name later does not rename existing resources.

Friend requests and copied invitations share only the account handle and public Cowchat website. They do not grant Home membership or bot access. SMS invitations remain outside this flow. Choose Local rooms to finish without signing in; local rooms and shared Global connections remain independent. Existing installations keep their completion state and can replay setup from Settings.

The server must advertise `setup_version: 1` before the wizard can provision resources. Deploy the server before the Mac app. Older servers show an update message with Retry and the Local escape; the Mac never falls back to separate bot and room creation calls. The server initializes an additive `social_setup` table. `GET /social/v1/setup` returns `{"setup": null}` or the recorded IDs, names, currently available bot and accessible room, and `bot_admitted`. Human-only `POST /social/v1/setup` accepts `display_name` and commits the profile, resources, owner membership, admission, and setup row in one transaction. Human-owned active bots accept `POST /social/v1/agents/:id/credentials`; agents and other accounts cannot issue their credentials.

## Room access

New collaboration rooms are private and unencrypted. Accepted participants can read earlier room history. Review both facts before accepting. Existing encrypted and local rooms retain their existing behavior.

New collaboration rooms do not connect to Slack in this version. Existing global-room Slack connections retain their existing behavior.

Removing a friend stops new invitations. It does not remove either person from a room they already accepted. Leave the room or remove an agent from its participant list to change existing room access.

Revoking an agent stops its credential and room participation. It cannot retract messages or files already received. Artifact links retain their existing sharing behavior. Anyone with an unlisted artifact URL can read that artifact until its owner rotates or removes the link.

Room approval does not start an agent process, give it tools, or authorize paid execution. Existing wake mechanisms and execution settings still apply.

## Sign-in identities

Cowchat binds each provider's immutable subject to a Cowchat account. Email equality does not merge accounts. Linking another provider requires an existing human session and successful authentication with that provider.

Old shared API keys are not proof of who owns an account or agent. Signing in does not claim old agents or rooms. The Mac keeps the existing shared global connection running alongside the signed-in account connection. The sidebar groups shared and private rooms under Global; Local rooms stay on this Mac. New Room keeps the Local/Global choice and, when both global connections exist, offers shared or private access.

Cowboy wallet sign-in is outside this first version. Cowchat's existing Cowboy-hosted session and protocol paths are unchanged.

## Verification and deployment

The implementation is tested locally with disposable accounts and rooms. Test provider endpoints are test-only. Production sign-in requires registered provider applications and exact callback URLs configured on the server.

Use a separate local database for development. Do not test by changing a shared engineering room or its Slack binding.

For provider application setup, callback URLs, session expiry, and the native callback contract check, see [Social sign-in configuration](friends-server.md).

The server must support room creation without `friend_id` before deploying the Mac solo-room form. Omit the field for a solo room; its response contains `invitation: null`. Existing requests with a friend retain their invitation response. No database migration is required.
