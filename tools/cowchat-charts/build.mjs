import { build } from 'esbuild';
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('.', import.meta.url));
export async function buildRenderer() {
  await mkdir(`${root}dist`, { recursive: true });
  await build({ absWorkingDir: root, entryPoints: ['client.jsx'], bundle: true,
    outfile: 'dist/client.js', minify: true, format: 'iife', platform: 'browser', target: 'es2022',
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: { '.woff2': 'file', '.woff': 'file', '.ttf': 'file', '.wasm': 'file' },
    logLevel: 'warning' });
  await writeFile(`${root}dist/index.html`, `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src data:; font-src 'self'; connect-src 'none'; base-uri 'none'; form-action 'none'"><title>Cowchat chart</title><link rel="stylesheet" href="client.css"><div id="chart"></div><script src="client.js"></script>`);
}
if (process.argv[1] === fileURLToPath(import.meta.url)) await buildRenderer();
