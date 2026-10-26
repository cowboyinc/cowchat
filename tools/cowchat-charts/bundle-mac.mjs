import { buildRenderer } from './build.mjs';
import { mkdir, copyFile } from 'node:fs/promises';
await buildRenderer();
const destination = new URL('../../apps/CowchatMac/Sources/CowchatMac/Bundled/Charts/', import.meta.url);
await mkdir(destination, { recursive: true });
for (const file of ['index.html', 'client.js', 'client.css']) {
  await copyFile(new URL(`dist/${file}`, import.meta.url), new URL(file, destination));
}
