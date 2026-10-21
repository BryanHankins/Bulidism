// node --test web/clipboard.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clickSelect, deleteElements, moveElements, copyElements, pasteElements, rectBounds, commonValue, dependsCycle } from './clipboard.js';

const el = (id, kind, extra = {}) => ({ id, name: id, kind, discipline: 'Architectural', phase: 0, start_offset_days: 0, lod: 'Lod200', cost_code: 'X', material: '', level: 0, ...extra });
const model = () => ({
  elements: [
    el('w1', { Wall: { start: [0, 0, 0], end: [6, 0, 0], height: 3, thickness: 0.25 } }),
    el('w2', { Wall: { start: [0, 0, 0], end: [0, 0, 4], height: 3, thickness: 0.25 } }),
    el('d1', { Door: { host: 'w1', offset: 1, width: 0.9, height: 2.1, is_egress: true } }),
    el('d2', { Door: { host: 'w2', offset: 1, width: 0.9, height: 2.1, is_egress: true } }),
    el('c1', { Column: { base: [2, 0, 2], width: 0.4, depth: 0.4, height: 3 } }, { depends_on: ['w1'] }),
  ],
  spaces: [{ name: 'S', occupancy: 'Business', floor_area_m2: 10, egress_doors: ['d1', 'd2'], bounds: null, level: 0 }],
});
let n = 0; const uuid = () => `new-${++n}`;

test('click selection: plain click replaces, modifier toggles', () => {
  assert.deepEqual(clickSelect(['a'], 'b', false), ['b']);
  assert.deepEqual(clickSelect(['a'], null, false), []);
  assert.deepEqual(clickSelect(['a'], 'b', true), ['a', 'b']);
  assert.deepEqual(clickSelect(['a', 'b'], 'a', true), ['b']);
  assert.deepEqual(clickSelect(['a'], null, true), ['a']);
});

test('delete removes hosted doors, egress refs and dependencies', () => {
  const p = model();
  assert.equal(deleteElements(p, ['w1', 'c1']), 3);
  assert.deepEqual(p.elements.map(e => e.id), ['w2', 'd2']);
  assert.deepEqual(p.spaces[0].egress_doors, ['d2']);
  const q = model(); deleteElements(q, ['w1']);
  assert.equal(q.elements.find(e => e.id === 'c1').depends_on, undefined, 'empty depends_on is omitted');
});

test('move: vectors snap, doors slide unless their wall moves', () => {
  const p = model(), snap = v => Math.round(v / 0.5) * 0.5;
  moveElements(p, ['w2', 'c1', 'd1'], 1.1, 0.2, snap);
  assert.deepEqual(p.elements[1].kind.Wall.start, [1, 0, 0]);
  assert.deepEqual(p.elements[4].kind.Column.base, [3, 0, 2]);
  assert.equal(p.elements[2].kind.Door.offset, 2, 'd1 slid along w1 (x axis)');
  assert.equal(p.elements[3].kind.Door.offset, 1, 'd2 not selected');
  const q = model(); moveElements(q, ['w1', 'd1'], 1, 0);
  assert.equal(q.elements[2].kind.Door.offset, 1, 'door rides with its wall');
  const r = model(); moveElements(r, ['d1'], 100, 0);
  assert.equal(r.elements[2].kind.Door.offset, 6 - 0.9, 'clamped to wall length');
});

test('paste: new ids, offset + snap, door hosts remapped or kept', () => {
  const p = model();
  const clip = copyElements(p, ['w1', 'd1', 'd2', 'c1']);
  const out = pasteElements(p, clip, { uuid, dx: 1, dz: 1, snap: v => Math.round(v / 0.5) * 0.5 });
  assert.equal(out.length, 4); assert.equal(p.elements.length, 9);
  const [w, d1, d2, c] = out;
  assert.ok(out.every(e => e.id.startsWith('new-')));
  assert.deepEqual(w.kind.Wall.start, [1, 0, 1]); assert.deepEqual(w.kind.Wall.end, [7, 0, 1]);
  assert.equal(d1.kind.Door.host, w.id, 'host copied: remapped');
  assert.equal(d2.kind.Door.host, 'w2', 'host not copied but still exists: kept');
  assert.equal(d2.kind.Door.offset, 2, 'and slid along it by the offset so it does not overlap the original');
  assert.equal(d1.kind.Door.offset, 1, 'door on a copied wall keeps its offset');
  assert.deepEqual(c.depends_on, [w.id]);
  assert.deepEqual(clip[0].kind.Wall.start, [0, 0, 0], 'clipboard untouched');
  // host gone since copying: the door is dropped
  const q = model(), clip2 = copyElements(q, ['d2', 'c1']);
  deleteElements(q, ['w2', 'w1']);
  const out2 = pasteElements(q, clip2, { uuid });
  assert.deepEqual(out2.map(e => e.kind.Column ? 'col' : 'door'), ['col']);
  assert.equal(out2[0].depends_on, undefined, 'dependency on a deleted element dropped');
});

test('space rectangle from a drag', () => {
  assert.deepEqual(rectBounds([4, 3], [1, 0.5]), [1, 0.5, 3, 2.5]);
  assert.equal(rectBounds([1, 1], [1, 4]), null);
  assert.equal(rectBounds([0, 0], [0.4, 3]), null);
});

test('common value and dependency cycles', () => {
  const p = model();
  assert.equal(commonValue(p.elements, 'phase'), 0);
  p.elements[0].phase = 1;
  assert.equal(commonValue(p.elements, 'phase'), undefined);
  assert.equal(commonValue([], 'phase'), undefined);
  assert.ok(dependsCycle(p, 'w1', 'w1'));
  assert.ok(dependsCycle(p, 'w1', 'c1'), 'c1 already depends on w1');
  assert.ok(!dependsCycle(p, 'c1', 'w2'));
});
