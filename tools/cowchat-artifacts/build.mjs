import { build } from 'esbuild';
import { compile } from '@tailwindcss/node';
import { readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
const root = dirname(fileURLToPath(import.meta.url));
export async function buildRenderer() {
  const output = resolve(root, '../../crates/cowchat-server/web');
  const result = await compile(await readFile(resolve(root, 'tailwind.css'), 'utf8'), { base: root, onDependency() {} });
  const source = await readFile(resolve(root, 'node_modules/@cowboyinc/gallop-components/dist/index.mjs'), 'utf8');
  const candidates = [...source.matchAll(/["'`]([^"'`]+)["'`]/g)].flatMap(match => match[1].split(/\s+/));
  await writeFile(resolve(root, '.gallop.generated.css'), result.build(candidates));
  await mkdir(output, { recursive: true });
  await build({absWorkingDir:root, entryPoints:['client.jsx', '.gallop.generated.css'], bundle:true, outdir:resolve(root, 'dist'),
    minify:true, format:'iife', platform:'browser', target:'es2022', define:{'process.env.NODE_ENV':'"production"'},
    loader:{'.woff2':'dataurl', '.woff':'dataurl', '.ttf':'dataurl', '.wasm':'file'}, logLevel:'warning'});
  const js = (await readFile(resolve(root, 'dist/client.js'), 'utf8')).replace(/[ \t]+$/gm, '');
  const css = `${await readFile(resolve(root, 'dist/.gallop.generated.css'), 'utf8')}\n${await readFile(resolve(root, 'dist/client.css'), 'utf8')}`;
  await writeFile(resolve(output, 'artifact.js'), js);
  await writeFile(resolve(output, 'artifact.css'), css);
  await writeFile(resolve(output, 'artifact.html'), `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex,nofollow"><meta name="referrer" content="no-referrer"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src data:; img-src data:; connect-src 'self'; base-uri 'none'; form-action 'none'"><title>Cowchat artifact</title><link rel="stylesheet" href="../artifact.css"></head><body><div id="artifact"></div><script src="../artifact.js"></script></body></html>`);
  await rm(resolve(root, '.gallop.generated.css'));
}
if (process.argv[1] === fileURLToPath(import.meta.url)) await buildRenderer();
