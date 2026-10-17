# Live room artifact renderer

The host loads `/a/{public_id}/snapshot` and subscribes to `/a/{public_id}/events`.
An SSE `revision` event invalidates the snapshot. Every connection, including
reconnection, fetches the latest complete snapshot. A permanently closed SSE
connection is recreated with backoff from 500 ms up to 10 seconds; “Live”
requires an open stream as well as a successful snapshot. The host supplies each JSON
data property as an OpenUI `$name` binding. The layout does not need to change.
The renderer remounts on revision changes to clear removed bindings; document
scroll position is retained. A `revoked` SSE event immediately clears the page and title, aborts pending
requests and stops reconnection. A snapshot 404/410, including a check after
SSE failure, also clears the page.

## Authoring

`examples/overview.openui` and `examples/data.json` are a complete example.
Arguments are positional in the order below. Optional arguments may be omitted.
All strings are escaped React text. No HTML, scripts, actions, forms, tool calls,
images, or query functions are registered. Links permit absolute HTTP(S) URLs
only and open with `noopener noreferrer`. Markdown skips HTML and images.

| Component | Arguments |
| --- | --- |
| `Stack` | children (array, max 200), optional direction (`"column"` or `"row"`) |
| `Card` | children (array, max 200) |
| `Heading` | text |
| `Text` | text |
| `Markdown` | text, supporting ordinary Markdown without raw HTML/images |
| `Metric` | label, scalar value (string/number/boolean/null), optional detail |
| `Badge` | text (Gallop Chip rendered disabled/read-only) |
| `Link` | text, absolute HTTP(S) href |
| `Table` | columns, rows, optional caption |

A table column is `{key:"name",label:"Name"}` (1–20 columns; keys max 100 chars).
Rows may be an array or a keyed object of generic row objects (max 1000 rows).
Keyed rows render in sorted key order, using the map key as their stable React
identity. For incremental updates, bind `Table(columns, $prs)` to data such as
`{"prs":{"core-618":{"name":"Review storage","status":"Queued"}}}`. A merge
patch `{"prs":{"core-618":{"status":"Done"}}}` changes just that entry;
`{"prs":{"core-618":null}}` removes its row. Each cell is a
scalar or `{text:"Report",href:"https://example.com"}` or
`{text:"Ready",badge:true}`. Missing cells display an em dash; null/missing rows
display “No data”. Text values are limited to 16,000 characters by the component
schemas. The server additionally bounds the entire view and data document.

Chart definitions are reused directly from the existing Cowchat offline chart
renderer's pinned OpenUI library: `BarChart`, `LineChart`, `AreaChart`,
`RadarChart`, `HorizontalBarChart`, `Series`, `PieChart`, `RadialChart`,
`SingleStackedBarChart`, `Slice`, `ScatterChart`, `ScatterSeries`, and `Point`.
For example:

```
root = Stack([Card([Heading("Revenue"), BarChart($revenue.labels, [Series("Revenue", $revenue.values)], "grouped")])])
```

with data `{"revenue":{"labels":["April","May"],"values":[28,42]}}`.
Publish the layout once; merge-patch data as needed. JSON Merge Patch replaces
arrays and removes properties with null. Deleted top-level keys do not retain
their previous values in the renderer.

## Viewer resource limits

Before OpenUI can lower or evaluate a view, an AST-only preflight bounds its
work and serializes only the approved root expression, inlining its named
references and dropping unused definitions. It permits the components
above, named references, literals, arrays/objects, `$data` bindings, property
access, literal indexes, and negative numeric literals. It requires a `root`
that resolves to `Stack`. Cycles, unknown references, state declarations,
operators, runtime builtins (including `@Each`), queries, mutations and actions
are rejected. Data must remain JSON values; OpenUI internal AST/element/action
object shapes and prototype-related keys are reserved.

Limits: 64 KiB source, 8,192 tokens, 128 statements, 32 levels of bracket and
expanded expression/reference depth, 256 expanded components, and 20,000
expanded weighted value nodes. Repeated references count each occurrence;
bound data and text length contribute to the same budget. Tables additionally
count rows × columns; charts conservatively count first-array × second-array
length toward a shared 20,000 render-work budget. Data is independently limited
to 512 KiB, 10,000 nodes and 16 levels. Over-budget or unsupported views display
a small view error without mounting OpenUI. Existing examples and incremental
table/chart bindings remain supported; use fewer blocks or smaller datasets
when a view exceeds these limits.

## Build and verify

```
pnpm install --frozen-lockfile
pnpm run build
pnpm test
```

Gallop components and foundations 0.7.0 are vendored as MIT-licensed published
package archives under `vendor/`, referenced with repository-relative file pins.
No private registry credentials are required. See `vendor/README.md` for source,
archive integrity and reproducible refresh instructions. Other dependencies pin
OpenUI lang-core/react-lang 1.0.1 and react-ui 0.18.1 and React 19.2.3. There are no absolute
local-path dependencies. Set `OPENUI_TELEMETRY_DISABLED=1` during installation to
disable the upstream OpenUI install telemetry; runtime observability is disabled
in the renderer.

Build outputs are `crates/cowchat-server/web/artifact.{html,js,css}` and are
embedded by the server. Gallop's Tailwind utilities are compiled from the actual
published component bundle. The existing vendored Season fonts are embedded as
data URLs. Runtime uses same-origin static assets and SSE/snapshot only, with
observability disabled and a restrictive CSP. No CDN is needed.

The browser test starts a disposable HTTP/SSE fixture and verifies a table and
chart change with the same layout, reconnect retrieves missed data, and removed
bindings disappear. It also checks Gallop's compiled Chip styles, zero external
requests, and browser errors. If Playwright's default Chromium is unavailable,
set `COWCHAT_CHART_BROWSER_PATH` to a compatible Chromium executable.
Screenshots are written to `/tmp/cowchat-artifact-before.png` and
`/tmp/cowchat-artifact-after.png`.

Data reserves the keys `__proto__`, `prototype`, and `constructor` at every
level. Objects cannot contain a `steps` array, `type: "element"`, a truthy
`__reactive`, or a `k` matching an OpenUI AST kind (`Comp`, `Str`, `Num`, `Bool`,
`Null`, `Arr`, `Obj`, `Ref`, `Ph`, `StateRef`, `RuntimeRef`, `BinOp`, `UnaryOp`,
`Ternary`, `Member`, `Index`, `Assign`). Use names such as `stages` for a list of
workflow steps. The server rejects these shapes before changing the artifact.
