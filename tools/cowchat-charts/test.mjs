import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { renderChart, validateSource } from './render.mjs';

test('rejects empty and oversized source before launching a browser', () => {
  assert.throws(() => validateSource('   '), /empty/);
  assert.throws(() => validateSource('x'.repeat(65537)), /64 KiB/);
});

test('OpenUI bar and line charts render to portable PNGs; remote image components are rejected', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'cowchat-chart-test-'));
  try {
    const bar = await readFile(new URL('./examples/revenue.openui', import.meta.url), 'utf8');
    for (const [kind, source] of [['bar', bar], ['line', bar.replace('BarChart(labels, [s1, s2], "grouped")', 'LineChart(labels, [s1, s2])')]]) {
      const output = join(temp, `${kind}.png`);
      await renderChart(source, output);
      const png = await readFile(output);
      assert.deepEqual(png.subarray(0, 8), Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
      assert.ok(png.length > 10000 && png.length <= 256 * 1024);
    }
    await assert.rejects(() => renderChart(bar.replace('Stack([title, chart])', 'Stack([Image("https://example.invalid/track"), chart])'), join(temp, 'unsafe.png')));
  } finally { await rm(temp, { recursive: true, force: true }); }
});
