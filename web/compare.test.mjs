// node --test web/compare.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { comparePanel, changedIds, totalText, measuredChange, summary } from './compare.js';
import { printHtml } from './print.js';

const finding = (over = {}) => ({ rule_id: 'egress', cite: '§1010.1.1', severity: 'error', subject: 'Main entry', message: 'clear width', element: 'd1', check: 'egress door clear width', measured: 0.636, limit: 0.813, bound: 'min', measure: 'length', ...over });
const comparison = (over = {}) => ({
  baseline: 'Office', label: 'issued-2026-10.json', rule_set_differs: false, unchanged_findings: 3,
  totals: [
    { label: 'Elements', unit: 'count', before: 12, after: 12 },
    { label: 'Cost', unit: 'usd', before: 29442.46, after: 28863.66 },
    { label: 'Duration', unit: 'days', before: 57, after: 60 },
  ],
  elements: [
    { id: 'c9', name: 'New <col>', status: 'added', aspects: [], cost_before_usd: 0, cost_after_usd: 100 },
    { id: 'd1', name: 'Main entry', status: 'changed', aspects: ['geometry', 'name'], cost_before_usd: 1, cost_after_usd: 1 },
    { id: 'c4', name: 'Column 4', status: 'removed', aspects: [], cost_before_usd: 5, cost_after_usd: 0 },
  ],
  spaces: [{ name: 'Office', status: 'changed', aspects: ['area 96.0 → 106.0 m2'] }],
  findings: [
    { status: 'added', finding: finding({ element: 'c9', subject: 'New <col>', severity: 'warning' }), measured_before: null },
    { status: 'changed', finding: finding(), measured_before: 0.686 },
    { status: 'removed', finding: finding({ element: 'gone', subject: 'Back door' }), measured_before: 0.686 },
  ],
  clashes: [
    { status: 'added', a: 'c9', b: 'w1', a_name: 'New <col>', b_name: 'South wall' },
    { status: 'removed', a: 'c4', b: 'w2', a_name: 'Column 4', b_name: 'North wall' },
  ],
  ...over,
});

test('totals and measured changes in display units', () => {
  assert.deepEqual(totalText({ unit: 'usd', before: 29442.46, after: 28863.66 }), { before: '$29,442', after: '$28,864', delta: '−$579' });
  assert.deepEqual(totalText({ unit: 'days', before: 57, after: 60 }), { before: '57 d', after: '60 d', delta: '+3 d' });
  assert.equal(totalText({ unit: 'count', before: 2, after: 2 }).delta, '');
  assert.equal(measuredChange(comparison().findings[1], 'imperial'), `2'-3" → 2'-1"`);
  assert.equal(measuredChange(comparison().findings[0], 'imperial'), '');
});

test('highlight ids: added/changed elements and new-clash pairs still in the model', () => {
  const present = id => id !== 'w1';
  assert.deepEqual(changedIds(comparison(), present).sort(), ['c9', 'd1']);
  assert.deepEqual(changedIds(comparison()).sort(), ['c9', 'd1', 'w1']);
});

test('panel: escaped, clickable rows only for elements in the model', () => {
  const present = id => ['c9', 'd1', 'w1'].includes(id);
  const h = comparePanel(comparison(), { label: 'issued <1>.json', units: 'imperial', present });
  assert.ok(!h.includes('<col>') && !h.includes('issued <1>') && h.includes('New &#60;col&#62;'));
  assert.ok(h.includes('1 new · 1 changed · 1 resolved · 3 unchanged'));
  assert.ok(h.includes('$29,442 → $28,864 <b>−$579</b>') && h.includes('57 d → 60 d <b>+3 d</b>'));
  assert.ok(!h.includes('>Elements</span>'), 'unchanged totals are left out');
  assert.ok(h.includes('data-ids="c9 w1"') && h.includes('data-ids="d1"'));
  assert.ok(!h.includes('data-ids="gone"') && !h.includes('data-ids="c4'), 'removed elements are not selectable');
  assert.ok(h.includes('1 added · 1 changed · 1 removed') && h.includes('geometry, name'));
  assert.deepEqual(summary(comparison()).newClashes, 1);
});

test('panel: no changes, and a rule-set mismatch is called out', () => {
  const none = comparison({ totals: [], elements: [], spaces: [], findings: [], clashes: [], rule_set_differs: true });
  const h = comparePanel(none, { units: 'metric' });
  assert.ok(h.includes('No changes') && h.includes('vs <b>issued-2026-10.json</b>') && h.includes('different rule sets'));
});

test('printed report carries the change list', () => {
  const r = {
    project: 'P', rule_set: 'IN-2026', code_edition: 'x', rules_verified: 'x', total_days: 60, total_cost_usd: 1,
    phases: [], elements: [], clashes: [], advisories: [], spaces: [], comparison: comparison(),
  };
  const h = printHtml(r, 'imperial', new Date(2026, 9, 12));
  assert.ok(h.includes('<h2>Changes since issued-2026-10.json</h2>'));
  assert.ok(h.includes(`egress door clear width (2&#39;-3&#34; → 2&#39;-1&#34;)`), h);
  assert.ok(h.includes('<td>Clash</td><td>New</td><td>New &#60;col&#62;</td><td>with South wall</td>'));
  assert.ok(h.includes('Cost $29,442 → <b>$28,864</b> (−$579)'));
  assert.ok(h.includes('3 findings unchanged.'));
  assert.ok(!printHtml({ ...r, comparison: undefined }, 'imperial').includes('Changes since'));
});
