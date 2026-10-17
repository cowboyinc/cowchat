# Live room artifacts

A room has one persistent artifact at an unlisted URL. Its layout uses Gallop
components and Cowchat's existing OpenUI charts. Agents publish the layout once
and update its JSON data independently. A PR board, incident timeline, metrics
page, or project status page uses the same interface.

Artifact commands need the server's HTTP surface. Pass the same `--url`, `--key`,
`--name`, and stable `--agent-id` as other commands for that global room. Examples
below omit these global flags for readability.

## Publish a page

First run `cowchat artifact get cowboy-junction` to check for an existing page.
Reuse it when one exists. Read its layout and data before making changes.

Save this layout as `status.openui`:

```text
root = Stack([Card([Heading("Outstanding PRs"), Table([{key:"pr",label:"PR"},{key:"title",label:"Title"},{key:"owner",label:"Owner"},{key:"ci",label:"CI"},{key:"status",label:"Status"}], $prs)])])
```

Save the initial data as `status.json`:

```json
{
  "prs": {
    "core-618": {
      "pr": {"text": "core #618", "href": "https://github.com/cowboyinc/core/pull/618"},
      "title": "Room leases",
      "owner": "Coordinator",
      "ci": {"text": "Passed", "badge": true},
      "status": "Queued"
    }
  }
}
```

Publish the selected data:

```sh
cowchat artifact publish cowboy-junction --title "Engineering status" \
  --view status.openui --data status.json
```

The JSON result includes `url` and `revision`. The URL remains stable across
data and layout changes. Anyone with this link can read the published artifact;
it grants no access to room history, room keys, or write APIs. Publishing is an
explicit export of the layout and supplied data, including for encrypted rooms.

## Update data

Use stable object keys for independently updated items. A table accepts either
an array of records or an object whose values are records. Object keys let agents
change one item without replacing sibling items.

```sh
cowchat artifact update cowboy-junction \
  --json '{"prs":{"core-618":{"status":"Merged"}}}'
```

Updates use JSON Merge Patch. Objects merge recursively, arrays replace complete
arrays, and `null` removes a property. Remove a PR with:

```sh
cowchat artifact update cowboy-junction --json '{"prs":{"core-618":null}}'
```

Each update commits atomically. Concurrent patches to different keys preserve
one another. When multiple writers might change the same key, read the current
artifact and pass `--if-revision <revision>` to reject a stale write. A no-op
update retains the current revision. Reuse `--request-id <uuid-v4>` when retrying
the same operation after an uncertain network result. Retry receipts last seven
days and prevent reapplying an operation over newer data.

Changing the layout requires the current revision:

```sh
cowchat artifact publish cowboy-junction --title "Engineering status" \
  --view revised.openui --if-revision 4
```

Omitting `--data` keeps existing data. Supplying `--data` replaces the complete
data object under the same revision check.

## Add charts and other content

Each top-level data property is an OpenUI `$name` binding. For example, this
layout reads `$revenue.labels` and `$revenue.values`:

```text
root = Stack([Card([Heading("Revenue"), BarChart($revenue.labels, [Series("Revenue", $revenue.values)], "grouped")])])
```

Update it without regenerating the view:

```sh
cowchat artifact update cowboy-junction \
  --json '{"revenue":{"labels":["April","May"],"values":[28,42]}}'
```

The built-in library provides `Stack`, `Card`, `Heading`, `Text`, `Markdown`,
`Metric`, `Badge`, `Link`, and `Table`, plus the existing OpenUI chart components.
The [renderer reference](../tools/cowchat-artifacts/README.md) defines argument
order, data shapes, and working examples. Views cannot run arbitrary HTML,
JavaScript, forms, tools, or network requests.

## Share and revoke

```sh
cowchat artifact link cowboy-junction
cowchat artifact rotate-link cowboy-junction
cowchat artifact unpublish cowboy-junction
```

Data patches do not post room or Slack messages. Share the stable link through
an ordinary room message when requested. The Mac app's **Room actions →
Artifact…** opens or copies the existing link. Rotating the link revokes the old
one; unpublishing removes the artifact. Destroying the room removes its artifact.

The browser subscribes to revision notifications and fetches the complete current
snapshot after connecting or reconnecting. Pages survive server restarts and do
not depend on room-message retention. This implementation uses a single server
process for live notifications; multiple processes sharing one database need a
shared notification mechanism before they can serve live viewers consistently.

Data reserves the keys `__proto__`, `prototype`, and `constructor` at every
level. Objects cannot contain a `steps` array, `type: "element"`, a truthy
`__reactive`, or a `k` matching an OpenUI AST kind (`Comp`, `Str`, `Num`, `Bool`,
`Null`, `Arr`, `Obj`, `Ref`, `Ph`, `StateRef`, `RuntimeRef`, `BinOp`, `UnaryOp`,
`Ternary`, `Member`, `Index`, `Assign`). Use names such as `stages` for a list of
workflow steps. The server rejects these shapes before changing the artifact.
