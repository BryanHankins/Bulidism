// node --test web/bcf.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { writeFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ifcGuid, stableGuid, toIfcPoint, toIfcDir, bcfTopics, markupXml, viewpointXml, buildBcf, crc32 } from './bcf.js';

const GUID = /^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[a-fA-F0-9]{4}-[a-fA-F0-9]{4}-[a-fA-F0-9]{12}$/;

test('IFC GlobalId compression matches the importer (bim_ifc::guid_to_uuid)', () => {
  assert.equal(ifcGuid('00000000-0000-0000-0000-000000000001'), '0000000000000000000001');
  assert.equal(ifcGuid('ffffffff-ffff-ffff-ffff-ffffffffffff'), '3$$$$$$$$$$$$$$$$$$$$$');
  // round-trip through a decoder written the same way as the Rust importer
  const B = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$';
  const decode = g => [...g].reduce((n, c) => (n << 6n) | BigInt(B.indexOf(c)), 0n).toString(16).padStart(32, '0');
  for (const g of ['0WallA0000000000000000', '2O2Fr$t4X7Zf8NOew3FLOH', '1Ano2ZUxnEIvVQ_beukl8b']) {
    const h = decode(g);
    assert.equal(ifcGuid(`${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`), g);
  }
});

test('stable GUIDs: valid, deterministic, distinct', () => {
  const a = stableGuid('x'), b = stableGuid('y');
  assert.match(a, GUID); assert.match(b, GUID);
  assert.equal(a, stableGuid('x')); assert.notEqual(a, b);
});

test('model coordinates map back to the source IFC frame', () => {
  // fixture: site at (100 m, 200 m), storey +3 m; Wall A starts at IFC (100, 200, 3) m.
  // The importer put it at model (0, 3, 4) with source_offset [-100, 0, 204].
  assert.deepEqual(toIfcPoint([0, 3, 4], [-100, 0, 204]), [100, 200, 3]);
  assert.deepEqual(toIfcDir([0, 1, 0]), [0, -0, 1]);   // model up -> IFC +Z
  assert.deepEqual(toIfcDir([0, 0, -1]), [0, 1, 0]);   // model -Z -> IFC +Y
});

const report = {
  levels: [{ id: 0, name: 'L1', elevation_m: 0 }],
  elements: [
    { id: '00000000-0000-4000-8000-000000000001', name: 'Door <A>', positions: [0, 0, 0, 1, 2, 0.1] },
    { id: '00000000-0000-4000-8000-000000000002', name: 'Col', positions: [5, 0, 5, 5.4, 3, 5.4] },
  ],
  spaces: [{ name: 'Room', bounds: [0, 0, 4, 4], level: 0 }],
  advisories: [
    { rule_id: 'egress', cite: '§1010.1.1', severity: 'error', subject: 'Door <A>', message: 'too narrow & "short"', element: '00000000-0000-4000-8000-000000000001', check: 'egress door clear width' },
    { rule_id: 'occ', cite: 'T1006', severity: 'warning', subject: 'Room', message: 'one exit', element: null, check: 'exits for an occupant load of 60' },
    { rule_id: 'cost.missing_rate', cite: '', severity: 'info', subject: 'X1', message: 'unpriced', element: null, check: null },
    { rule_id: 'rule_set.mismatch', cite: '', severity: 'warning', subject: 'P', message: 'mismatch', element: null, check: null },
  ],
  clashes: [{ a: '00000000-0000-4000-8000-000000000002', b: '00000000-0000-4000-8000-000000000001', min: [0, 0, 0], max: [1, 1, 1], overlap_m3: 0.15 }],
};

test('topics: findings with a subject and every clash; notices skipped', () => {
  const t = bcfTopics(report);
  assert.deepEqual(t.map(x => [x.type, x.priority, x.title]), [
    ['Issue', 'High', 'Egress door clear width: Door <A>'],
    ['Issue', 'Normal', 'Exits for an occupant load of 60: Room'],
    ['Clash', 'Normal', 'Clash: Col / Door <A>'],
  ]);
  assert.deepEqual(t[0].focus, { min: [0, 0, 0], max: [1, 2, 0.1] });
  assert.deepEqual(t[1].focus, { min: [0, 0, 0], max: [4, 3, 4] }); // space box at its level
  assert.equal(t[2].components.length, 2);
  // the same finding keeps its topic GUID across exports
  assert.equal(bcfTopics(report)[0].guid, t[0].guid);
});

test('XML escapes text and selects components by IFC GlobalId', () => {
  const [t] = bcfTopics(report);
  const m = markupXml(t, { date: '2026-10-12T00:00:00Z', viewpointGuid: stableGuid('v'), snapshot: true });
  assert.ok(m.includes('<Title>Egress door clear width: Door &lt;A&gt;</Title>') && m.includes('too narrow &amp; &quot;short&quot;'));
  assert.ok(m.includes('<Snapshot>snapshot.png</Snapshot>'));
  const v = viewpointXml(t, stableGuid('v'), { position: [1, 2, 3], direction: [0, 0, -1], up: [0, 1, 0], fov: 30 }, [0, 0, 0]);
  assert.ok(v.includes(`<Component IfcGuid="${ifcGuid(t.components[0])}"/>`));
  assert.ok(v.includes('<FieldOfView>45</FieldOfView>'), 'clamped to the BCF 2.1 range 45-60');
});

test('zip is readable by an independent implementation', () => {
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 1, 2, 3]);
  const { zip, files } = buildBcf(report, { source_offset: [-100, 0, 204] }, {
    views: t => ({ camera: { position: [0, 5, 10], direction: [0, -0.4, -0.9], up: [0, 1, 0], fov: 45 }, png }),
    now: new Date('2026-10-12T12:00:00Z'),
  });
  assert.equal(files.length, 1 + 3 * 3);
  assert.equal(crc32(new TextEncoder().encode('123456789')), 0xcbf43926); // standard CRC-32 check value
  const dir = mkdtempSync(join(tmpdir(), 'bcf-')), path = join(dir, 'x.bcfzip');
  writeFileSync(path, zip);
  let py;
  try { py = execFileSync('python3', ['-c', `import zipfile,sys;z=zipfile.ZipFile(sys.argv[1]);assert z.testzip() is None;print(len(z.namelist()))`, path]).toString().trim(); }
  catch (e) { if (e.code === 'ENOENT') return; throw e; } // no python: skip the cross-check
  assert.equal(py, String(files.length));
});
