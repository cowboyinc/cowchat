# Local OpenUI chart renderer

See [Cowchat chart usage](../../docs/charts.md) for installation, examples,
limits, encryption, and Slack setup. The renderer uses the OpenUI components
from the `pnpx @openuidev/cli@latest create` scaffold, without the scaffold's
Next.js agent backend or cloud inference/storage integrations.

`pnpm build` bundles the pinned React/OpenUI renderer. `pnpm render` builds it
and runs it in a fresh headless Chromium with no external network access.
`pnpm test` covers bar/line output, limits, and rejected remote image components.

Example OpenUI language:

```text
root = Stack([title, chart])
title = TextContent("Monthly revenue", "large-heavy")
chart = BarChart(labels, [s1, s2], "grouped")
labels = ["January", "February", "March"]
s1 = Series("Product A", [120, 150, 180])
s2 = Series("Product B", [90, 110, 140])
```

## Portable message format

`cowchat send-chart` writes `cowchat-chart:v1` followed by a newline and a JSON
object with `title` and `png` (standard base64), plus optional `openui` source (64 KiB maximum). That string
is the normal message content, encrypted by the existing client when the room
is encrypted. `send-chart` also sets metadata type `chart` to select the media allowance; recognition relies on the
validated content so old metadata decoders do not discard images.

The shared `test-fixtures/chart-v1.txt` vector covers Rust and Swift decoding.
