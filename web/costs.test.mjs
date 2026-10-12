// node --test web/costs.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { rateOut, rateIn, parseDollars, perLabel, costRows, ratesToCsv, parseRatesCsv } from './costs.js';

test('rates convert between display and stored metric cents', () => {
  // sample wall rate: $220.00/m² = $20.44/SF
  assert.equal(rateOut(22000, 'M2', 'imperial').toFixed(2), '20.44');
  assert.equal(rateOut(22000, 'M2', 'metric'), 220);
  assert.equal(rateIn(20.44, 'M2', 'imperial'), 22001);       // nearest cent per m²
  assert.equal(rateIn(150, 'M3', 'imperial'), 19619);         // $150/CY = $196.192/m³
  assert.equal(rateIn(850, 'Each', 'imperial'), 85000);
  for (const [c, u] of [[22000, 'M2'], [18000, 'M3'], [120000, 'M'], [85000, 'Each']])
    assert.ok(Math.abs(rateIn(+rateOut(c, u, 'imperial').toFixed(2), u, 'imperial') - c) <= 6, u);
  assert.equal(perLabel('M3', 'imperial'), 'CY');
  assert.equal(perLabel(null, 'imperial'), '');
});

test('dollar parsing', () => {
  assert.equal(parseDollars('$1,234.50'), 1234.5);
  assert.equal(parseDollars(' 12 '), 12);
  for (const bad of ['', '-5', 'abc', '1.2.3']) assert.ok(Number.isNaN(parseDollars(bad)), bad);
});

test('rows cover priced, unpriced, unused and mixed-unit codes', () => {
  const rows = costRows({ B2010: 22000, Z1: 500 }, [
    { cost_code: 'B2010', unit: 'M2' }, { cost_code: 'B2010', unit: 'M2' },
    { cost_code: 'C9', unit: 'Each' },
    { cost_code: 'MIX', unit: 'M2' }, { cost_code: 'MIX', unit: 'M3' },
  ]);
  assert.deepEqual(rows.map(r => [r.code, r.cents, r.unit, r.count, r.mixed]), [
    ['B2010', 22000, 'M2', 2, false], ['C9', null, 'Each', 1, false], ['MIX', null, 'M2', 2, true], ['Z1', 500, null, 0, false],
  ]);
});

test('CSV round-trip and errors', () => {
  const rows = costRows({ B2010: 22000, 'A,1': 18000 }, [{ cost_code: 'B2010', unit: 'M2' }, { cost_code: 'A,1', unit: 'M3' }]);
  const csv = ratesToCsv(rows, 'imperial');
  assert.equal(csv, 'code,rate_usd,per\n"A,1",137.62,CY\nB2010,20.44,SF\n');
  const { rates, errors } = parseRatesCsv(csv);
  assert.deepEqual(errors, []);
  assert.ok(Math.abs(rates.B2010 - 22000) <= 1 && Math.abs(rates['A,1'] - 18000) <= 1);
  const r = parseRatesCsv('C1020,"$1,200.00",EA\nX,abc,SF\nY,5,furlong\n,5,SF\nG2030,350,m2');
  assert.equal(r.rates.C1020, 120000);
  assert.equal(r.rates.G2030, 35000);
  assert.equal(r.errors.length, 3);
});
