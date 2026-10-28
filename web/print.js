// Printable code-check report: a standalone page the browser prints or saves as PDF.
// Pure function of the engine report, so it can be tested without a browser.
import { fmtArea, fmtVol, fmtMeasure, fmtLen } from './units.js';
import './cites.js';
import { totalText, measuredChange, FINDING_STATUS, MARK } from './compare.js';

const { citeHtml } = globalThis.BuildsimCites;

const esc = v => String(v ?? '').replace(/[&<>"']/g, c => '&#' + c.charCodeAt(0) + ';');
const money = v => '$' + (+v).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
// CSS string literal content: escape \ and ", and drop line breaks; </style can't occur (no '<' allowed through)
const cssStr = v => String(v).replace(/[\\"]/g, c => '\\' + c).replace(/[\r\n<]/g, ' ');
const RANK = { error: 0, warning: 1, info: 2 };
const LABEL = { error: 'Error', warning: 'Warning', info: 'Info' };

/** Data gaps that weaken the results; listed before the analysis so readers weigh it. */
function readinessBlock(items) {
  if (!items || !items.length) return '';
  return `<h2>Model readiness</h2><ul style="margin:4px 0 0;padding-left:18px">${items.map(i =>
    `<li${i.severity === 'warning' ? ' class="sev warning" style="font-weight:400"' : ''}>${i.count ? i.count + ' ' : ''}${esc(i.message)}${i.examples.length ? ` (${i.examples.map(esc).join(', ')}${i.count > i.examples.length ? ', …' : ''})` : ''}</li>`).join('')}</ul>`;
}

/** Changes since the baseline version (revision narrative for a reissue); empty without one. */
function changesBlock(c, units, table) {
  if (!c) return '';
  const totals = c.totals.filter(t => t.before !== t.after);
  const rows = [];
  for (const f of c.findings) rows.push(`<tr><td>Finding</td><td class="sev ${esc(f.finding.severity)}">${FINDING_STATUS[f.status]}</td><td>${esc(f.finding.subject)}</td><td>${esc(f.finding.check || f.finding.message)}${measuredChange(f, units) ? ` (${esc(measuredChange(f, units))})` : ''}</td></tr>`);
  for (const k of c.clashes) rows.push(`<tr><td>Clash</td><td>${k.status === 'added' ? 'New' : 'Resolved'}</td><td>${esc(k.a_name)}</td><td>with ${esc(k.b_name)}</td></tr>`);
  for (const e of c.elements) rows.push(`<tr><td>Element</td><td>${MARK[e.status]} ${e.status}</td><td>${esc(e.name)}</td><td>${esc(e.aspects.join(', '))}</td></tr>`);
  for (const sp of c.spaces) rows.push(`<tr><td>Space</td><td>${MARK[sp.status]} ${sp.status}</td><td>${esc(sp.name)}</td><td>${esc(sp.aspects.join(', '))}</td></tr>`);
  return `<h2>Changes since ${esc(c.label || c.baseline)}</h2>` +
    (totals.length ? `<p class="meta">${totals.map(t => { const x = totalText(t); return `${esc(t.label)} ${x.before} → <b>${x.after}</b> (${x.delta})`; }).join(' · ')}</p>` : '') +
    table(['Item', 'Change', 'Element / space', 'Detail'], rows, 'No changes.') +
    (c.unchanged_findings ? `<p class="none">${c.unchanged_findings} finding${c.unchanged_findings === 1 ? '' : 's'} unchanged.</p>` : '');
}

/** The code analysis sheet ("code data block"); empty when the rule set has no parameters. */
function codeSheet(c, units, table) {
  if (!c) return '';
  const len = m => esc(fmtLen(m, units)), yes = b => b ? 'Yes' : 'No';
  const cell = (prov, req) => `<td class="num${prov + 1e-6 >= req ? '' : ' sev warning'}">`;
  const rows = [
    ['Construction type', c.construction_type ? esc(c.construction_type) : '<span class="unverified">not set</span> (allowable area and height not evaluated)'],
    ['Sprinklered throughout', yes(c.sprinklered)], ['Emergency voice/alarm', yes(c.voice_alarm)],
    ['Stories', c.stories || '—'], ['Height above lowest level', c.height_m == null ? '—' : len(c.height_m)],
    ['Total floor area (spaces)', esc(fmtArea(c.total_area_m2, units))], ['Total occupant load', c.total_occupant_load],
    ['Egress capacity factors', c.reduced_factors ? 'reduced (sprinklered + voice/alarm)' : 'standard'],
  ];
  return `<h2>Code analysis</h2>
<table><tbody>${rows.map(([k, v]) => `<tr><th style="width:40%">${k}</th><td>${v}</td></tr>`).join('')}</tbody></table>
${table(['Occupancy', 'Area#', 'Occupant load#'], c.occupancies.map(o => `<tr><td>${esc(o.occupancy)}</td><td class="num">${esc(fmtArea(o.area_m2, units))}</td><td class="num">${o.occupant_load}</td></tr>`), 'No spaces defined.')}
<h3 style="font-size:10.5pt;margin:12px 0 4px">Means of egress by story <span style="font-weight:400;color:#444">(${esc(c.cite_exits)}; ${esc(c.cite_capacity)})</span></h3>
${table(['Story', 'Occupant load#', 'Exits req / prov#', 'Door width req / prov#', 'Stair width req / prov#'],
  c.levels.map(l => `<tr><td>${esc(l.name)}${l.spaces_without_factor ? ` <span class="unverified">(${l.spaces_without_factor} space(s) uncounted)</span>` : ''}</td><td class="num">${l.occupant_load}</td>` +
    `${cell(l.exits_provided, l.exits_required)}${l.exits_required} / ${l.exits_provided}</td>` +
    `${cell(l.door_width_provided_m, l.door_width_required_m)}${len(l.door_width_required_m)} / ${len(l.door_width_provided_m)}</td>` +
    `${l.stair_width_required_m == null ? '<td class="num">' : cell(l.stair_width_provided_m, l.stair_width_required_m)}${l.stair_width_required_m == null ? '—' : `${len(l.stair_width_required_m)} / ${len(l.stair_width_provided_m)}`}</td></tr>`),
  'No stories.')}
<h3 style="font-size:10.5pt;margin:12px 0 4px">Plumbing fixtures required <span style="font-weight:400;color:#444">(${esc(c.cite_plumbing)})</span></h3>
${table(['Occupancy', 'Fixture', 'Occupant load#', 'Required#', 'Parameter verification'],
  c.plumbing.map(p => `<tr><td>${esc(p.occupancy)}</td><td>${esc(p.fixture)}</td><td class="num">${p.occupant_load}</td><td class="num">${p.required}${p.per_sex ? ` (${p.per_group} per sex)` : ''}</td><td class="unverified">${esc(p.verified)}</td></tr>`),
  'No plumbing ratios for these occupancies in this rule set.')}
<p class="none">Analysis parameters: ${esc(c.verified)}</p>
<ul class="none" style="margin:4px 0 0;padding-left:18px">${c.notes.map(n => `<li>${esc(n)}</li>`).join('')}</ul>`;
}

/**
 * @param r       report JSON from bim.run (advisory messages already in display units)
 * @param units   'imperial' | 'metric'
 * @param now     generation time (injectable for tests)
 */
export function printHtml(r, units, now = new Date()) {
  const name = id => (r.elements.find(e => e.id === id) || {}).name || '?';
  const levels = r.levels || [], levelName = id => (levels.find(l => l.id === id) || {}).name || '—';
  const findings = [...r.advisories].sort((a, b) => RANK[a.severity] - RANK[b.severity]);
  const count = s => r.advisories.filter(a => a.severity === s).length;
  const total = r.total_cost_usd ?? r.phases.reduce((a, p) => a + p.cost_usd, 0);
  const date = now.toLocaleDateString('en-US', { year: 'numeric', month: 'long', day: 'numeric' });
  // a heading ending in '#' is a numeric (right-aligned) column
  const table = (head, rows, empty) => rows.length
    ? `<table><thead><tr>${head.map(h => h.endsWith('#') ? `<th class="num">${h.slice(0, -1)}</th>` : `<th>${h}</th>`).join('')}</tr></thead><tbody>${rows.join('')}</tbody></table>`
    : `<p class="none">${empty}</p>`;

  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(r.project)} — Code Check Report</title>
<style>
/* running footer in the page margin box, so it can never overlap content; on screen the .foot div shows instead */
@page { size: letter; margin: 0.55in 0.6in 0.7in;
  @bottom-center { content: "${cssStr(`${r.project} · Code Check Report · ${date} · Advisory only`)}"; font: 8pt -apple-system, Helvetica, Arial, sans-serif; color: #666; }
  @bottom-right { content: "Page " counter(page) " of " counter(pages); font: 8pt -apple-system, Helvetica, Arial, sans-serif; color: #666; } }
* { box-sizing: border-box; }
body { font: 10.5pt/1.4 -apple-system, "Segoe UI", Helvetica, Arial, sans-serif; color: #111; background: #fff; margin: 0 auto; max-width: 7.3in; padding: 16px; }
h1 { font-size: 18pt; margin: 0 0 2px; } h2 { font-size: 12pt; margin: 18px 0 6px; border-bottom: 1.5px solid #111; padding-bottom: 2px; break-after: avoid; }
.meta { color: #444; margin: 0 0 10px; } .meta b { color: #111; }
.disclaimer { border: 1.5px solid #111; padding: 8px 10px; margin: 10px 0; font-size: 9.5pt; }
.summary { display: grid; grid-template-columns: repeat(4, 1fr); gap: 6px; margin: 8px 0; }
.summary div { border: 1px solid #999; padding: 6px 8px; } .summary b { display: block; font-size: 15pt; }
table { width: 100%; border-collapse: collapse; font-size: 9.5pt; } thead { display: table-header-group; }
th, td { text-align: left; vertical-align: top; padding: 4px 6px; border-bottom: 1px solid #bbb; } th { border-bottom: 1.5px solid #111; }
tr { break-inside: avoid; } td.num, th.num { text-align: right; white-space: nowrap; }
.sev { font-weight: 700; white-space: nowrap; } .sev.error { color: #a40000; } .sev.warning { color: #8a5a00; } .sev.info { color: #444; }
.keep { break-inside: avoid; } .unverified { color: #8a5a00; } .none { color: #444; font-style: italic; }
/* title-block sign-off: always on page 1, signed by hand after printing */
.sign { display: grid; grid-template-columns: 1.2fr 0.6fr 1.4fr 0.8fr; gap: 0 14px; margin: 22px 0 10px; }
.sign div { border-top: 1px solid #111; padding-top: 3px; font-size: 9pt; color: #444; }
.foot { font-size: 8pt; color: #666; text-align: center; }
.toolbar { margin-bottom: 12px; } .toolbar button { font: inherit; padding: 4px 12px; }
@media print { .toolbar { display: none; } body { padding: 0; } .foot { display: none; } }
@media screen { .foot { margin-top: 24px; } }
</style></head><body>
<div class="toolbar"><button onclick="print()">Print / Save as PDF</button></div>
<h1>Code Check Report</h1>
<p class="meta"><b>${esc(r.project)}</b> · Generated ${esc(date)} · Rule set <b>${esc(r.rule_set)}</b> — ${esc(r.code_edition)} · Units: ${units === 'imperial' ? 'feet-inches' : 'metric'}</p>
<div class="disclaimer"><b>ADVISORY ONLY — NOT A COMPLIANCE DETERMINATION.</b> Automated checks of a simplified model against selected code parameters. They do not replace review of the full code text, local amendments, or the authority having jurisdiction. Rule verification: ${esc(r.rules_verified)}. ${r.rules_reviewer ? `Rule set reviewed by ${esc(r.rules_reviewer)}.` : '<b>This rule set has not been reviewed by a licensed professional.</b>'}</div>
<div class="sign"><div>Prepared by</div><div>Date</div><div>Reviewed by (licensed professional)</div><div>License no. / State</div></div>
<div class="summary">
 <div><b>${count('error')}</b>Errors</div><div><b>${count('warning')}</b>Warnings</div>
 <div><b>${(r.rules_checked || []).length}</b>Rules checked</div><div><b>${r.clashes.length}</b>Clashes</div>
</div>

${changesBlock(r.comparison, units, table)}
${readinessBlock(r.readiness)}
${codeSheet(r.code_summary, units, table)}
<h2>Findings</h2>
${table(['##', 'Severity', 'Element / space', 'Finding', 'Measured#', 'Required#', 'Citation'],
  findings.map((a, i) => `<tr><td class="num">${i + 1}</td><td class="sev ${esc(a.severity)}">${esc(LABEL[a.severity] || a.severity)}</td><td>${esc(a.subject)}</td><td>${esc(a.check || a.message)}</td><td class="num">${esc(fmtMeasure(a.measured, a.measure, units))}</td><td class="num">${a.limit == null ? '—' : `${a.bound === 'max' ? '≤' : '≥'} ${esc(fmtMeasure(a.limit, a.measure, units))}`}</td><td>${citeHtml(a.cite)}</td></tr>`),
  'No findings from this rule set.')}

<h2>Rules checked</h2>
${table(['Rule', 'Citation', 'Result', 'Parameter verification'],
  (r.rules_checked || []).map(c => `<tr><td>${esc(c.id)}</td><td>${citeHtml(c.cite)}</td><td>${c.findings ? `<span class="sev ${esc(c.severity)}">${c.findings} finding${c.findings === 1 ? '' : 's'}</span>` : 'No findings'}</td><td class="${c.verified && !/unverified|secondary/i.test(c.verified) ? '' : 'unverified'}">${esc(c.verified || 'Unverified')}</td></tr>`),
  'No rule file was loaded, so no code checks ran.')}

<h2>Spaces</h2>
${table(['Space', ...(levels.length ? ['Level'] : []), 'Occupancy', 'Area#', 'Occupant load#', 'Exits#'],
  r.spaces.map(s => `<tr><td>${esc(s.name)}</td>${levels.length ? `<td>${esc(levelName(s.level))}</td>` : ''}<td>${esc(s.occupancy)}</td><td class="num">${esc(fmtArea(s.area_m2, units))}</td><td class="num">${s.occupant_load ?? '—'}</td><td class="num">${s.exits}</td></tr>`),
  'No spaces defined.')}

<h2>Clashes</h2>
${table(['Element', 'Clashes with', 'Overlap#'],
  r.clashes.map(c => `<tr><td>${esc(name(c.a))}</td><td>${esc(name(c.b))}</td><td class="num">${esc(fmtVol(c.overlap_m3, units))}</td></tr>`),
  'No cross-discipline clashes.')}

<section class="keep"><h2>Schedule and estimated cost</h2>
${table(['Phase', 'Days', 'Estimated cost#'],
  r.phases.map(p => `<tr><td>${esc(p.name)}${p.critical ? ' *' : ''}</td><td>${p.start_day}–${p.end_day}</td><td class="num">${money(p.cost_usd)}</td></tr>`)
    .concat(`<tr><td><b>Total</b></td><td><b>${r.total_days} days</b></td><td class="num"><b>${money(total)}</b></td></tr>`),
  'No phases.')}
<p class="none">* critical path. Costs are estimates from the project's unit rates.</p></section>

<div class="foot">${esc(r.project)} · Code Check Report · ${esc(date)} · Advisory only</div>
</body></html>`;
}
