// node --test web/e2e/   (browser end-to-end: the real app, wasm engine and viewer)
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { start, close, openApp, ready, text, applyJson, download, zipNames, REPO } from './harness.mjs';

const FIXTURE_IFC = path.join(REPO, 'crates/bim-ifc/tests/data/fixture.ifc');
before(start);
after(close);

test('starts on the sample with every report section and no page errors', async () => {
  const { page, ctx, errors } = await openApp();
  assert.match(await text(page, '#status'), /ran in [\d.]+ ms · schema v\d/);
  assert.equal(await page.textContent('#runErr'), '');
  assert.match(await text(page, '#readiness'), /Model readiness: 0 warnings, 1 note/);
  assert.match(await text(page, '#codeSummary'), /Total occupant load 7/);
  assert.match(await text(page, '#compare'), /Compare with file/);
  assert.equal(await page.textContent('#nadv'), String(await page.evaluate(() => document.querySelectorAll('#advs .adv').length)));
  assert.equal(await page.textContent('#total'), '57');
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('place a column by clicking the model; undo and redo it', async () => {
  const { page, ctx, errors } = await openApp();
  const count = () => page.evaluate(() => JSON.parse(document.getElementById('jsonBox').value).elements.length);
  const n = await count();
  await page.click('[data-tab="edit"]');
  await page.click('#placeCol');
  const [x, y] = await page.evaluate(() => window.__viewer.project([6, 0.2, 4]));
  await page.mouse.click(x, y);
  await page.waitForFunction(m => JSON.parse(document.getElementById('jsonBox').value).elements.length === m, n + 1);
  const col = await page.evaluate(() => JSON.parse(document.getElementById('jsonBox').value).elements.at(-1));
  assert.ok(col.kind.Column, JSON.stringify(col));
  assert.ok(Math.abs(col.kind.Column.base[0] - 6) <= 0.25 && Math.abs(col.kind.Column.base[2] - 4) <= 0.25, 'snapped near the click');
  await page.click('#undo'); assert.equal(await count(), n);
  await page.click('#redo'); assert.equal(await count(), n + 1);
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('units switch the display and survive a reload, as does the autosaved model', async () => {
  const { page, ctx, errors } = await openApp();
  await page.selectOption('#units', 'metric'); await ready(page);
  assert.match(await text(page, '#spaceList'), /96\.0 m²/);
  await page.selectOption('#units', 'imperial'); await ready(page);
  assert.match(await text(page, '#spaceList'), /1,033 sf/);
  await applyJson(page, p => { p.name = 'Autosave check'; });
  await page.reload(); await ready(page);
  assert.match(await text(page, '#status'), /restored autosave/);
  assert.equal(await page.evaluate(() => JSON.parse(document.getElementById('jsonBox').value).name), 'Autosave check');
  assert.match(await text(page, '#spaceList'), /1,033 sf/);
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('IFC import: notes, levels and readiness', async () => {
  const { page, ctx, errors } = await openApp();
  await page.setInputFiles('#file', FIXTURE_IFC); await page.waitForSelector('#importNotes:not([hidden])');
  assert.match(await text(page, '#importNotes'), /Imported fixture\.ifc/);
  const p = await page.evaluate(() => JSON.parse(document.getElementById('jsonBox').value));
  assert.ok(p.elements.length > 0 && p.levels.length > 0 && p.source_offset, 'elements, storeys, re-centring offset');
  assert.equal(await page.textContent('#runErr'), '');
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('printed report: findings with measured vs required, code sheet, sign-off', async () => {
  const { page, ctx, errors } = await openApp();
  const [w] = await Promise.all([page.waitForEvent('popup'), page.click('#printReport')]);
  await w.waitForLoadState();
  const heads = await w.evaluate(() => [...document.querySelectorAll('h2')].map(h => h.textContent));
  for (const h of ['Code analysis', 'Findings', 'Rules checked', 'Spaces', 'Clashes']) assert.ok(heads.includes(h), `${h} in ${heads}`);
  const findings = await w.evaluate(() => { const h = [...document.querySelectorAll('h2')].find(x => x.textContent === 'Findings'); return h.nextElementSibling.innerText; });
  assert.match(findings, /Measured\s+Required/);
  assert.match(findings, /2'-3"\s+≥ 2'-8"/);
  assert.match(await w.evaluate(() => document.body.innerText), /Reviewed by \(licensed professional\)/);
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('exports: BCF archive of findings and clashes, CSV takeoff', async () => {
  const { page, ctx, errors } = await openApp();
  const bcf = await download(page, '#exportBcf');
  assert.match(bcf.name, /\.bcfzip$/);
  const names = zipNames(bcf.bytes);
  assert.ok(names.includes('bcf.version'));
  const topics = names.filter(n => n.endsWith('/markup.bcf'));
  assert.ok(topics.length >= 5, names.join(' '));
  assert.equal(names.filter(n => n.endsWith('/snapshot.png')).length, topics.length);
  const csv = (await download(page, '#exportCsv')).bytes.toString();
  assert.match(csv.split('\n')[0], /^element,cost_code,phase/);
  assert.equal(csv.trim().split('\n').length, 1 + await page.evaluate(() => JSON.parse(document.getElementById('jsonBox').value).elements.length));
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('version comparison: baseline, edit, resolved finding, highlight, reload, clear', async () => {
  const { page, ctx, errors } = await openApp();
  await page.click('#cmpSet');
  assert.match(await text(page, '#compare'), /No changes/);
  await applyJson(page, p => {
    p.elements.find(e => e.name === 'Main entry').kind.Door.width = 0.95;
    p.elements = p.elements.filter(e => e.name !== 'Column 4');
  });
  const panel = await text(page, '#compare');
  assert.match(panel, /Errors 2 → 1 −1/);
  assert.match(panel, /0 new · 0 changed · 1 resolved/);
  assert.match(panel, /Resolved North wall ↔ Column 4/);
  assert.match(panel, /~ Main entry geometry/);
  await page.click('#cmpHighlight');
  assert.equal(await page.evaluate(() => window.__viewer.selectedIds.length), 1);
  await page.reload(); await ready(page);
  assert.match(await text(page, '#compare'), /1 resolved/);
  await page.click('#cmpClear');
  assert.match(await text(page, '#compare'), /Compare with file/);
  assert.equal(await page.evaluate(() => localStorage.getItem('buildsim.baseline.v1')), '');
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('large model: merged meshes, and BCF snapshots still work', async () => {
  const { page, ctx, errors } = await openApp();
  await applyJson(page, p => {
    const col = p.elements.find(e => e.kind.Column);
    for (let i = 0; i < 2100; i++) {
      const c = JSON.parse(JSON.stringify(col));
      c.id = crypto.randomUUID(); c.name = `Grid column ${i}`;
      c.kind.Column.base = [40 + (i % 50) * 2, 0.2, Math.floor(i / 50) * 2];
      p.elements.push(c);
    }
  });
  assert.equal(await page.evaluate(() => window.__viewer.merged), true);
  const bcf = await download(page, '#exportBcf');
  assert.ok(zipNames(bcf.bytes).some(n => n.endsWith('snapshot.png')));
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('bad JSON never replaces the model', async () => {
  const { page, ctx, errors } = await openApp();
  await page.click('[data-tab="json"]');
  await page.fill('#jsonBox', '{"name": 1');
  await page.click('#apply');
  assert.notEqual(await page.textContent('#jsonMsg'), '');
  await page.click('[data-tab="report"]');
  assert.match(await text(page, '#codeSummary'), /Total occupant load 7/);
  assert.deepEqual(errors, []);
  await ctx.close();
});

test('egress: walking distances per room, longest-walk overlay, travel and common-path findings', async () => {
  const { page, ctx, errors } = await openApp();
  assert.match(await text(page, '#spaceList'), /longest walk to an exit 31'-\d+ ?[\d/]*" · common path 0"/);
  await page.check('#showPaths');
  // stretch the office to 70 m with only the main entry as an exit
  await applyJson(page, p => {
    const north = p.elements.find(e => e.name === 'North wall');
    north.kind.Wall.start[2] = 70; north.kind.Wall.end[2] = 70;
    p.spaces[0].bounds = [0, 0, 12, 70];
    for (const e of p.elements) if (e.kind.Door) e.kind.Door.is_egress = e.name === 'Main entry';
  });
  const advs = await text(page, '#advs');
  assert.match(advs, /exit access travel distance 230'-\d/, advs);
  assert.match(advs, /common path of egress travel 230'-\d.* > 75'-0"/);
  const [w] = await Promise.all([page.waitForEvent('popup'), page.click('#printReport')]);
  await w.waitForLoadState();
  assert.match(await w.evaluate(() => document.body.innerText), /Travel to exit\s+Common path/);
  await w.close();
  assert.deepEqual(errors, []);
  await ctx.close();
});
