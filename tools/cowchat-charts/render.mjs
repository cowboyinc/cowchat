import { chromium } from 'playwright';
import { createServer } from 'node:http';
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { buildRenderer } from './build.mjs';

export function validateSource(source) {
  if (Buffer.byteLength(source) > 65536) throw new Error('Chart source exceeds 64 KiB');
  if (!source.trim()) throw new Error('Chart source is empty');
}

export async function renderChart(source, output) {
  validateSource(source);
  await buildRenderer();
  const root = fileURLToPath(new URL('./dist/', import.meta.url));
  const server = createServer(async (request, response) => {
    const file = request.url === '/' ? 'index.html' : request.url.slice(1);
    if (!/^[a-zA-Z0-9_.-]+$/.test(file)) { response.writeHead(404).end(); return; }
    try {
      const body = await readFile(root + file);
      response.writeHead(200, {
        'Content-Type': file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html',
        'Content-Security-Policy': "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src data: blob:; connect-src 'none'; frame-src 'none'; object-src 'none'",
        'Cache-Control': 'no-store',
      }).end(body);
    } catch { response.writeHead(404).end(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  let browser;
  let deadline;
  try {
    browser = await chromium.launch({ headless: true, executablePath: process.env.COWCHAT_CHART_BROWSER_PATH });
    deadline = setTimeout(() => { void browser.close(); }, 15000);
    const page = await browser.newPage({ viewport: { width: 960, height: 800 }, deviceScaleFactor: 2 });
    const origin = `http://127.0.0.1:${server.address().port}`;
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/*', route => route.request().url().startsWith(origin + '/') ? route.continue() : route.abort());
    await page.goto(origin, { waitUntil: 'load', timeout: 10000 });
    await page.waitForFunction(() => typeof window.renderChart === 'function', null, { timeout: 10000 });
    await page.evaluate(source => window.renderChart(source), source);
    await page.waitForFunction(() => window.chartError || document.querySelector('#chart svg'), null, { timeout: 10000 });
    const error = await page.evaluate(() => window.chartError);
    if (error || errors.length) throw new Error(error || errors.join('; '));
    await page.evaluate(() => document.fonts.ready);
    // Wait for layout and ResizeObserver-driven chart dimensions, not a fixed delay.
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const chart = page.locator('#chart');
    const box = await chart.boundingBox();
    if (!box || box.height > 1800) throw new Error('Chart is empty or exceeds the 1800 px height limit');
    const finalError = await page.evaluate(() => window.chartError);
    if (finalError) throw new Error(finalError);
    const png = await chart.screenshot({ timeout: 10000 });
    if (png.length > 256 * 1024) throw new Error('Rendered chart exceeds 256 KiB; simplify the chart');
    await writeFile(output, png);
  } finally {
    clearTimeout(deadline);
    await browser?.close();
    await new Promise(resolve => server.close(resolve));
  }
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { values, positionals } = parseArgs({ options: { out: { type: 'string' } }, allowPositionals: true });
  if (positionals.length !== 1 || !values.out?.endsWith('.png')) throw new Error('Usage: pnpm render chart.openui --out chart.png');
  await renderChart(await readFile(positionals[0], 'utf8'), values.out);
  console.log(`Chart written to ${values.out}`);
}
