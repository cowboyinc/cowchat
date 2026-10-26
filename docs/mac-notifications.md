# Mac notifications

Cowchat can notify you about incoming friend requests, accepted outgoing friend requests and incoming room invitations. Click an alert to open Friends or Invitations. Approval stays inside Cowchat.

Open **Preferences → Notifications** to choose event types, request macOS permission or send a test notification. If macOS blocks notifications, open its notification settings from this page and allow Cowchat. Focus modes and macOS notification preferences control how alerts appear.

Cowchat checks your account every 30 seconds while the app runs, including when the Friends panel is closed. Closing the last Cowchat window quits the app, so alerts stop until it runs again. This uses the existing social endpoints and does not require a server update or push service.

The first successful account sync establishes a silent baseline. After that, seen event IDs persist separately for each server and account. New events since the previous run can notify on the next launch. Repeated polls and reconnects do not repeat seen alerts. Events observed while notifications or an event type are disabled are recorded without replaying when enabled. Signing out clears delivered notifications and prevents stale responses or clicks from opening another account's data.

Alerts contain the sender's handle and, for invitations, the room name. They do not include agent credentials or room messages. macOS controls lock-screen previews. Room messages do not generate notifications in this version.
