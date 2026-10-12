// node --test web/levels.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { setLevelElevation } from './levels.js';

const project = () => ({
  levels: [{ id: 0, name: 'L1', elevation_m: 0 }, { id: 1, name: 'L2', elevation_m: 3.6 }],
  elements: [
    { name: 'w1', level: 0, kind: { Wall: { start: [0, 0, 0], end: [5, 0, 0], height: 3, thickness: 0.2 } } },
    { name: 'w2', level: 1, kind: { Wall: { start: [0, 3.6, 0], end: [5, 3.6, 0], height: 3, thickness: 0.2 } } },
    { name: 'b2', level: 1, kind: { Beam: { start: [0, 6.2, 0], end: [5, 6.2, 0], width: 0.3, depth: 0.4 } } },
    { name: 's2', level: 1, kind: { Slab: { origin: [0, 3.4, 0], size_x: 5, size_z: 5, thickness: 0.2 } } },
    { name: 'd2', level: 1, kind: { Door: { host: 'x', offset: 1, width: 0.9, height: 2.1, is_egress: false } } },
    { name: 'ramp', level: null, kind: { Ramp: { base: [0, 0, 0], run_length: 2, rise: 0.2, width: 1 } } },
  ],
  spaces: [],
});

test('moves everything on the level by the elevation change', () => {
  const p = project();
  assert.equal(setLevelElevation(p, 1, 4.0), 4);
  const y = n => { const k = Object.values(p.elements.find(e => e.name === n).kind)[0]; return k.start?.[1] ?? k.origin?.[1] ?? k.base?.[1]; };
  const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} != ${b}`);
  near(p.levels[1].elevation_m, 4.0);
  near(y('w2'), 4.0); near(p.elements[1].kind.Wall.end[1], 4.0);
  near(y('b2'), 6.6); near(p.elements[2].kind.Beam.end[1], 6.6);
  near(y('s2'), 3.8);
  // other levels and site work stay put; doors have no coordinates of their own
  near(y('w1'), 0); near(y('ramp'), 0);
  assert.deepEqual(p.elements[4].kind.Door, project().elements[4].kind.Door);
});

test('no-ops', () => {
  const p = project();
  assert.equal(setLevelElevation(p, 1, 3.6), 0);
  assert.equal(setLevelElevation(p, 9, 1), 0);
  assert.deepEqual(p, project());
});
