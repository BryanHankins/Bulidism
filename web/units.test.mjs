// node --test web/units.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ftIn, parseLen, fmtArea, fmtVol, parseArea, qtyOut, convertMessage, lenToInput, snapStep, M_PER_IN, M_PER_FT } from './units.js';

const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} != ${b}`);

test('formats feet-inches to 1/8"', () => {
  assert.equal(ftIn(42 * M_PER_IN), `3'-6"`);
  assert.equal(ftIn(0.813), `2'-8"`);            // IBC 32 in clear width
  assert.equal(ftIn(6.5 * M_PER_IN), `6 1/2"`);
  assert.equal(ftIn(12 * M_PER_FT + 0.125 * M_PER_IN), `12'-0 1/8"`);
  assert.equal(ftIn(-3 * M_PER_FT), `-3'-0"`);
  assert.equal(ftIn(0), `0"`);
});

test('parses the ways people type lengths', () => {
  const ft = (f, i = 0) => f * M_PER_FT + i * M_PER_IN;
  near(parseLen(`3'-6"`, 'imperial'), ft(3, 6));
  near(parseLen(`3' 6 1/2"`, 'imperial'), ft(3, 6.5));
  near(parseLen(`42"`, 'imperial'), ft(0, 42));
  near(parseLen(`6 1/2"`, 'imperial'), ft(0, 6.5));
  near(parseLen(`1/2"`, 'imperial'), ft(0, 0.5));
  near(parseLen(`3.5'`, 'imperial'), ft(3.5));
  near(parseLen(`3ft 6in`, 'imperial'), ft(3, 6));
  near(parseLen(`3.5`, 'imperial'), ft(3.5));      // bare number = feet
  near(parseLen(`-10'`, 'imperial'), -ft(10));
  for (const bad of ['', 'abc', `3'x`, `1/0"`, `3"4'`]) assert.ok(Number.isNaN(parseLen(bad, 'imperial')), bad);
  near(parseLen('2.75', 'metric'), 2.75);
  assert.ok(Number.isNaN(parseLen(`3'`, 'metric')));
});

test('round-trips every 1/8" up to 20 ft', () => {
  for (let e = -160; e <= 20 * 96; e += 7) {
    const m = e / 8 * M_PER_IN;
    near(parseLen(ftIn(m), 'imperial'), m);
    assert.equal(lenToInput(parseLen(lenToInput(m, 'imperial'), 'imperial'), 'imperial'), ftIn(m));
  }
});

test('areas, quantities, messages, snap', () => {
  assert.equal(fmtArea(96, 'imperial'), '1,033 sf');
  assert.equal(fmtArea(96, 'metric'), '96.0 m²');
  near(parseArea('1000', 'imperial'), 1000 * M_PER_FT * M_PER_FT);
  assert.deepEqual(qtyOut(1, 'Each', 'imperial'), [1, 'Each']);
  assert.equal(qtyOut(3.5, 'M', 'imperial')[1], 'LF');
  near(qtyOut(0.764554857984, 'M3', 'imperial')[0], 1);
  assert.equal(
    convertMessage('egress door clear width 0.686 m (leaf 0.750 m - 0.064 m) < 0.813 m', 'imperial'),
    `egress door clear width 2'-3" (leaf 2'-5 1/2" - 2 1/2") < 2'-8"`);
  assert.equal(convertMessage('ramp slope 1:8.0 steeper than 1:12; area 3 m2', 'imperial'), 'ramp slope 1:8.0 steeper than 1:12; area 3 m2');
  near(snapStep('imperial'), 0.1524);
  assert.equal(fmtVol(0.15, 'imperial'), '5.3 cf');
  assert.equal(fmtVol(0.15, 'metric'), '0.150 m³');
});
