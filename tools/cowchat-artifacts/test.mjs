import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { chromium } from 'playwright';
import { buildRenderer } from './build.mjs';


test('live Gallop table and OpenUI chart update from snapshots, reconnect, and clear deleted data', { timeout: 45000 }, async () => {
  await buildRenderer();
  const view = await readFile(new URL('./examples/overview.openui', import.meta.url), 'utf8');
  let snapshot = { title:'Team overview', view, data:JSON.parse(await readFile(new URL('./examples/data.json', import.meta.url))), revision:1, updated_at:'2026-10-16T12:00:00Z' };
  let snapshotReads = 0;
  let connections = 0;
  let failEventsOnce = false;
  let blockSnapshot = false;
  let blockedResponse;
  let onSnapshotBlocked;
  const streams = new Set();
  const server = createServer(async (request, response) => {
    const path = request.url;
    if (path === '/a/test/snapshot') { if (!snapshot) { response.writeHead(404).end(); return; } snapshotReads++; if (blockSnapshot) { blockedResponse = response; onSnapshotBlocked(); return; } response.writeHead(200, {'content-type':'application/json'}).end(JSON.stringify(snapshot)); return; }
    if (path === '/a/test/events') {
      connections++;
      if (failEventsOnce) { failEventsOnce = false; response.writeHead(502, {'content-type':'text/plain'}).end('Deploying'); return; }
      response.writeHead(200, {'content-type':'text/event-stream', 'cache-control':'no-store'});
      response.write('retry: 50\n\n'); streams.add(response); request.on('close', () => streams.delete(response)); return;
    }
    const filename = path === '/a/test' ? 'artifact.html' : path.slice(1);
    if (!/^artifact\.(html|js|css)$/.test(filename)) { response.writeHead(404).end(); return; }
    response.writeHead(200, {'content-type':filename.endsWith('.js')?'text/javascript':filename.endsWith('.css')?'text/css':'text/html'}).end(await readFile(new URL(`../../crates/cowchat-server/web/${filename}`, import.meta.url)));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch({headless:true, executablePath:process.env.COWCHAT_CHART_BROWSER_PATH});
    const page = await browser.newPage({viewport:{width:1280,height:1100}});
    const errors = [];
    const requests = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', request => requests.push(request.url()));
    const origin = `http://127.0.0.1:${server.address().port}`;
    await page.goto(`${origin}/a/test`);
    await page.getByRole('cell', {name:'Sam', exact:true}).waitFor();
    await page.locator('.artifact-content svg').first().waitFor();
    assert.equal(await page.getByRole('alert').count(),0);
    const before = await page.locator('.artifact-content svg').first().innerHTML();
    assert.ok(before.includes('April'));
    const shapesBefore = await page.locator('.artifact-content svg').first().locator('path').evaluateAll(nodes => nodes.map(node => node.getAttribute('d')));
    const chip = page.locator('.artifact-badge').first();
    assert.equal(await chip.evaluate(node => getComputedStyle(node).display), 'inline-flex');
    await page.screenshot({path:'/tmp/cowchat-artifact-before.png',fullPage:true});
    snapshot = {...snapshot, revision:2, data:{...snapshot.data, work:{...snapshot.data.work,report:{name:'Release review',owner:'Taylor',status:{text:'Done',badge:true}}}, revenue:{labels:['July','August','September'],values:[64,19,95]}}};
    for (const stream of streams) stream.write('event: revision\ndata: {"revision":2}\n\n');
    await page.getByRole('cell', {name:'Taylor',exact:true}).waitFor();
    await page.waitForFunction(before => document.querySelector('.artifact-content svg')?.innerHTML !== before, before);
    assert.equal(await page.getByRole('cell',{name:'Sam',exact:true}).count(),0);
    assert.equal(await page.getByRole('cell',{name:'Alex',exact:true}).count(),1, 'Updating one map entry preserves siblings');
    const after = await page.locator('.artifact-content svg').first().innerHTML();
    assert.ok(after.includes('September') && !after.includes('April'));
    const shapesAfter = await page.locator('.artifact-content svg').first().locator('path').evaluateAll(nodes => nodes.map(node => node.getAttribute('d')));
    assert.notDeepEqual(shapesAfter, shapesBefore, 'Bar geometry changes with the bound numeric values');
    await page.screenshot({path:'/tmp/cowchat-artifact-after.png',fullPage:true});
    const readsBeforeReconnect = snapshotReads;
    snapshot = {...snapshot, revision:3, data:{...snapshot.data,work:{report:{name:'Reconnected',owner:'Jordan',status:'Ready'}}}};
    failEventsOnce = true;
    const failedReconnect = page.waitForResponse(response => response.url().endsWith('/events') && response.status() === 502);
    for (const stream of streams) stream.end();
    await failedReconnect;
    await page.getByRole('cell',{name:'Jordan',exact:true}).waitFor();
    assert.equal(await page.getByRole('status').textContent(),'Reconnecting', 'Snapshot success does not claim the closed stream is live');
    await page.getByRole('status').filter({hasText:/^Live$/}).waitFor();
    assert.ok(connections >= 3, 'A permanently closed EventSource is replaced after 502');
    assert.equal(await page.getByRole('cell',{name:'Alex',exact:true}).count(),0, 'Deleting a map entry clears its row');
    assert.ok(snapshotReads > readsBeforeReconnect);
    const {work, ...remaining} = snapshot.data;
    snapshot = {...snapshot, revision:4,data:remaining};
    for (const stream of streams) stream.write('event: revision\ndata: {"revision":4}\n\n');
    await page.getByText('No data',{exact:true}).waitFor();
    assert.equal(await page.getByRole('cell',{name:'Jordan',exact:true}).count(),0);
    assert.equal(await page.getByRole('alert').count(),0);
    assert.deepEqual(errors,[]);
    assert.ok(requests.every(url => url.startsWith(origin)), 'No external requests including fonts/telemetry');
    // Authored HTML and executable link schemes cannot become browser behavior.
    snapshot = {...snapshot, revision:5, view:'root = Stack([Markdown($notes), Link("Unsafe link", "javascript:alert(1)"), Text("<script>alert(1)</script>")])', data:{notes:'<img src="https://example.invalid/tracker" onerror="alert(1)">\\n\\n[Unsafe](javascript:alert(1))'}};
    for (const stream of streams) stream.write('event: revision\ndata: {"revision":5}\n\n');
    await page.getByText('Unsafe link',{exact:true}).waitFor();
    assert.equal(await page.locator('.artifact-content img, .artifact-content script, .artifact-content a[href^="javascript:"]').count(),0);
    assert.equal(await page.getByText('<script>alert(1)</script>',{exact:true}).count(),1);
    assert.ok(requests.every(url => url.startsWith(origin)));
    snapshot = {...snapshot,revision:6,view:'root = Stack([Image("https://example.invalid/tracker")])'};
    for (const stream of streams) stream.write('event: revision\ndata: {"revision":6}\n\n');
    await page.waitForFunction(() => document.querySelector('[role=alert]') || document.querySelector('.artifact-content[data-revision="6"]'));
    assert.equal(await page.locator('.artifact-content img').count(),0);
    snapshot = {...snapshot,revision:7,view:await readFile(new URL('./fixtures/repeated-aliases.openui',import.meta.url),'utf8')};
    for (const stream of streams) stream.write('event: revision\ndata: {"revision":8}\n\n');
    await page.locator('[role=alert][data-revision="7"]').waitFor();
    assert.equal(await page.locator('.artifact-content').count(),0, 'Rejected raw AST never mounts Renderer');
    snapshot = {...snapshot, revision:8, view, data:JSON.parse(await readFile(new URL('./examples/data.json', import.meta.url)))};
    for (const stream of streams) stream.write('event: revision\ndata: {"revision":8}\n\n');
    await page.getByRole('cell',{name:'Sam',exact:true}).waitFor();
    // Rotation/delete sends a revoked event and closes the stream. It must clear
    // content without waiting for a snapshot 404 or EventSource reconnect.
    blockSnapshot = true;
    const blocked = new Promise(resolve => { onSnapshotBlocked = resolve; });
    for (const stream of streams) stream.write('event: revision\ndata: {"revision":8}\n\n');
    await blocked;
    const readsBeforeRevoked = snapshotReads;
    const connectionsBeforeRevoked = connections;
    for (const stream of streams) { stream.write('event: revoked\ndata: {}\n\n'); stream.end(); }
    await page.getByRole('heading', {name:'This artifact is unavailable.'}).waitFor();
    blockedResponse.end(JSON.stringify({...snapshot,revision:8,title:'Stale content'}));
    blockSnapshot = false;
    assert.equal(await page.locator('.artifact-content').count(),0);
    assert.equal(await page.title(),'Cowchat artifact');
    await page.waitForTimeout(200);
    assert.equal(snapshotReads, readsBeforeRevoked);
    assert.equal(connections, connectionsBeforeRevoked);
    assert.equal(await page.getByRole('cell',{name:'Sam',exact:true}).count(),0);
    // If revocation was missed while disconnected, a failed SSE connection
    // probes the snapshot and clears old content on 404.
    const reconnectPage = await browser.newPage();
    await reconnectPage.goto(`${origin}/a/test`);
    await reconnectPage.getByRole('cell',{name:'Sam',exact:true}).waitFor();
    snapshot = null;
    for (const stream of streams) stream.end();
    await reconnectPage.getByRole('heading',{name:'This artifact is unavailable.'}).waitFor();
    assert.equal(await reconnectPage.locator('.artifact-content').count(),0);
    assert.equal(await reconnectPage.title(),'Cowchat artifact');
    await reconnectPage.close();
    assert.ok(requests.every(url => url.startsWith(origin)));
    assert.deepEqual(errors,[]);
  } finally {
    await browser?.close();
    for (const stream of streams) stream.end();
    await new Promise(resolve => server.close(resolve));
  }
});
