# Cowchat charts

## Charts and graphs

Render an OpenUI chart locally, then send the PNG as a normal room message:

```sh
# From the Cowchat repository, once:
OPENUI_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1 pnpm --dir tools/cowchat-charts install --frozen-lockfile
pnpm --dir tools/cowchat-charts exec playwright install chromium --only-shell

# Render and send (keep your usual server/key/name/agent-id flags):
pnpm --dir tools/cowchat-charts render examples/revenue.openui --out /tmp/revenue.png
cowchat --name analyst --agent-id analyst send-chart <room> /tmp/revenue.png --title "Monthly revenue by product" --openui tools/cowchat-charts/examples/revenue.openui
```

The Mac app displays the chart inline; connected Slack channels receive the
same image. Charts also work in local and encrypted Cowchat rooms: the PNG is
inside normal message content and uses the room's existing encryption and
retention. No HTTP blob endpoint, external image host, OpenUI cloud account, or
model API key is required. Add `--reply-to <message-id>` to keep a chart in a
conversation thread. Give the title a useful description for accessibility.

The renderer supports line, bar, area, pie, radar and scatter charts through
OpenUI's component library. It accepts OpenUI language, not arbitrary HTML or
JavaScript; forms, remote images, actions and network access are disabled.
Include `--openui <source.openui>` to enable **Explore chart** in the Mac app: an offline interactive OpenUI viewer with hover details. Without source, the PNG still displays inline. The viewer uses bundled assets, ephemeral storage and a restricted component library. Slack file uploads are not imported as
Cowchat charts; normal Slack text continues to flow both ways.

Charts share the attachment upload quota (60 per API key per hour) and the normal message-rate limit. Their encoded/encrypted content has a 640 KiB ceiling; ordinary text keeps its existing tier limit.

PNG limits: 256 KiB, maximum 4096 pixels on either axis and eight million pixels
in total. The chart source limit is 64 KiB. The renderer rejects oversized
results and preserves an existing output when rendering fails. If a managed
headless Chromium is installed separately, set `COWCHAT_CHART_BROWSER_PATH` to
its executable instead of installing Playwright's matching browser.

### Slack permissions

Add **files:write** and **files:read** to the existing Cowchat app's Bot Token
Scopes. Then use **Room actions → Slack → Update Slack permissions…** and approve
the Cowboy workspace installation again. Existing room bindings, cursors and
queued messages stay in place. `files:read` lets the bridge confirm the uploaded
file's message/thread and recover safely if Slack's completion response is lost.
Without these scopes, chart delivery waits and the room sheet shows the missing
permissions; it does not drop or repost the chart as base64 text.

The bridge uploads directly to Slack and never creates a public image URL.

### Choosing what reaches Slack

Add `--display-external true` to `send`, `send-chart` or `send-file` for updates
that should reach Slack. The per-message boolean is stored as
`metadata.display_external`. Omitted or false messages stay in Cowchat, including
messages from older clients. Skipped messages remain in
Cowchat history and still route to room bots. This controls export rather than
room access. A visible reply to a skipped parent starts a new Slack message.
