// node --test web/print.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { printHtml } from './print.js';

const report = (over = {}) => ({
  project: 'Clinic <b>A</b>', rule_set: 'IN-2026', code_edition: '2026 IBC', rules_verified: 'partial', rules_reviewer: null,
  total_days: 20, total_cost_usd: 1234.5,
  phases: [{ id: 0, name: 'Shell', start_day: 0, end_day: 20, critical: true, cost_usd: 1234.5 }],
  elements: [{ id: 'a', name: 'Wall 1' }, { id: 'b', name: 'Col <1>' }],
  clashes: [{ a: 'a', b: 'b', overlap_m3: 0.15 }],
  advisories: [
    { rule_id: 'door.min_height', cite: '§1010', severity: 'warning', subject: 'D1', message: 'short' },
    { rule_id: 'egress', cite: '§1010.1.1', severity: 'error', subject: '<img onerror=x>', message: `2'-3" < 2'-8"`, check: 'egress door clear width', measured: 0.686, limit: 0.813, bound: 'min', measure: 'length' },
  ],
  rules_checked: [
    { id: 'egress', cite: '§1010.1.1', severity: 'error', verified: '2026-09-27 ICC', findings: 1 },
    { id: 'ramp', cite: '§1012', severity: 'error', verified: 'secondary source only', findings: 0 },
  ],
  spaces: [{ name: 'Office', occupancy: 'Business', area_m2: 96, exits: 2, occupant_load: 7 }],
  ...over,
});

test('escapes every model string', () => {
  const h = printHtml(report(), 'imperial', new Date(2026, 9, 12));
  assert.ok(!h.includes('<img onerror') && !h.includes('<b>A</b>') && !h.includes('Col <1>'));
  assert.ok(h.includes('&#60;img onerror=x&#62;') && h.includes('Col &#60;1&#62;'));
});

test('errors first, units, totals, verification, sign-off', () => {
  const h = printHtml(report(), 'imperial', new Date(2026, 9, 12));
  assert.ok(h.indexOf('&#60;img') < h.indexOf('>D1<'), 'errors sorted before warnings');
  assert.ok(h.includes('1,033 sf') && h.includes('5.3 cf') && h.includes('$1,234.50'));
  // structured findings: measured vs required in display units; plain notes fall back to the message
  assert.ok(h.includes('<td>egress door clear width</td><td class="num">2&#39;-3&#34;</td><td class="num">≥ 2&#39;-8&#34;</td>'), h);
  assert.ok(h.includes('<td>short</td><td class="num">—</td><td class="num">—</td>'));
  assert.ok(h.includes('No findings') && h.includes('class="unverified">secondary source only'));
  assert.ok(h.includes('not been reviewed by a licensed professional'));
  assert.ok(h.includes('October 12, 2026') && h.includes('License no.'));
  assert.ok(printHtml(report(), 'metric').includes('96.0 m²'));
});

test('spaces show their level when the project has levels', () => {
  const h = printHtml(report({ levels: [{ id: 0, name: 'Level <1>', elevation_m: 0 }], spaces: [{ name: 'Office', occupancy: 'Business', area_m2: 96, exits: 2, occupant_load: 7, level: 0 }] }), 'metric');
  assert.ok(h.includes('<th>Level</th>') && h.includes('<td>Level &#60;1&#62;</td>'));
  assert.ok(!printHtml(report(), 'metric').includes('<th>Level</th>'));
});

test('code analysis sheet', () => {
  const code_summary = { verified: 'secondary <src>', cite_exits: 'T1006', cite_capacity: '§1005.3', cite_plumbing: 'T2902.1',
    construction_type: null, sprinklered: true, voice_alarm: false, reduced_factors: false, stories: 2, height_m: 7.2,
    total_area_m2: 1096, total_occupant_load: 79, occupancies: [{ occupancy: 'Business', area_m2: 1096, occupant_load: 79 }],
    levels: [
      { name: 'Level 1', occupant_load: 7, spaces_without_factor: 0, exits_required: 2, exits_provided: 2, door_width_required_m: 0.0356, door_width_provided_m: 1.522, stair_width_required_m: null, stair_width_provided_m: 0 },
      { name: 'Level <2>', occupant_load: 72, spaces_without_factor: 1, exits_required: 2, exits_provided: 1, door_width_required_m: 0.366, door_width_provided_m: 0.836, stair_width_required_m: 0.549, stair_width_provided_m: 1.2 },
    ],
    plumbing: [{ occupancy: 'Business', fixture: 'Water closets', occupant_load: 79, per_group: 3, required: 6, per_sex: true, verified: 'study guide' }],
    notes: ['Allowable area & height not evaluated'] };
  const h = printHtml(report({ code_summary }), 'imperial');
  assert.ok(h.includes('<h2>Code analysis</h2>') && h.indexOf('Code analysis') < h.indexOf('<h2>Findings</h2>'));
  assert.ok(h.includes('not set</span> (allowable area and height not evaluated)'));
  assert.ok(h.includes('Level &#60;2&#62;') && h.includes('(1 space(s) uncounted)'));
  assert.ok(h.includes('<td class="num sev warning">2 / 1</td>'), 'short exits flagged');
  assert.ok(!/<td[^>]*class="[^"]*"[^>]*class=/.test(h), 'one class attribute per cell');
  assert.ok(h.includes('6 (3 per sex)') && h.includes('secondary &#60;src&#62;') && h.includes('Allowable area &#38; height'));
  assert.ok(!printHtml(report(), 'metric').includes('Code analysis'), 'no sheet without parameters');
});

test('readiness items print before the analysis', () => {
  const h = printHtml(report({ readiness: [{ id: 'space.no_exits', severity: 'warning', message: 'spaces with <no> exits', count: 7, examples: ['A', 'B'] }] }), 'metric');
  assert.ok(h.includes('<h2>Model readiness</h2>') && h.includes('7 spaces with &#60;no&#62; exits (A, B, …)'));
  assert.ok(h.indexOf('Model readiness') < h.indexOf('<h2>Findings</h2>'));
  assert.ok(!printHtml(report({ readiness: [] }), 'metric').includes('Model readiness'));
});

test('page footer is a safe CSS string', () => {
  const h = printHtml(report({ project: 'A "quoted" \\ name</style><script>' }), 'metric');
  const css = h.slice(h.indexOf('<style>'), h.indexOf('</style>'));
  assert.ok(css.includes('content: "A \\"quoted\\" \\\\ name /style> script>'), css.slice(0, 400));
  assert.equal((h.match(/<\/style>/g) || []).length, 1);
});

test('empty states and a reviewed rule set', () => {
  const h = printHtml(report({ advisories: [], clashes: [], spaces: [], rules_checked: [], rules_reviewer: 'J. Doe, RA' }), 'metric');
  assert.ok(h.includes('No findings from this rule set.') && h.includes('No rule file was loaded'));
  assert.ok(h.includes('No cross-discipline clashes.') && h.includes('No spaces defined.'));
  assert.ok(h.includes('reviewed by J. Doe, RA') && !h.includes('not been reviewed'));
});

test('citations link to the code text in a new tab', () => {
  assert.ok(printHtml(report(), 'metric').includes('<td>§1010.1.1</td>'), 'a bare section names no code book: plain text');
  const r = printHtml(report({ advisories: [{ rule_id: 'x', cite: 'IBC 2024 §1010.1.1', severity: 'error', subject: 'D', message: 'm' }] }), 'metric');
  assert.ok(r.includes('<a href="https://codes.iccsafe.org/content/IBC2024P1/chapter-10-means-of-egress" target="_blank" rel="noopener noreferrer">IBC 2024 §1010.1.1</a>'));
});

test('walking distances per room when rooms have a plan shape', () => {
  const r = report({ spaces: [
    { name: 'Office', occupancy: 'Business', area_m2: 96, exits: 2, occupant_load: 7, outline: [[0, 0], [1, 0], [1, 1]], travel_m: 9.6, common_path_m: 0 },
    { name: 'Store', occupancy: 'Storage', area_m2: 9, exits: 0, occupant_load: 1, outline: [[0, 0], [1, 0], [1, 1]], travel_m: null, common_path_m: null },
    { name: 'Lobby', occupancy: 'Business', area_m2: 20, exits: 1, occupant_load: 2 },
  ] });
  const h = printHtml(r, 'imperial', new Date(2026, 9, 12));
  assert.ok(h.includes('<th class="num">Travel to exit</th><th class="num">Common path</th>'));
  assert.ok(h.includes(`<td class="num">31&#39;-6&#34;</td><td class="num">0&#34;</td>`), h.slice(h.indexOf('Spaces')));
  assert.ok(h.includes('<span class="sev error">no route</span>'));
  assert.ok(h.includes('<td class="num">not checked</td><td class="num">—</td>'));
  assert.ok(!printHtml(report(), 'imperial').includes('Travel to exit'), 'no column without shapes');
});
