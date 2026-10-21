// Printable code-check report: a standalone page the browser prints or saves as PDF.
// Pure function of the engine report, so it can be tested without a browser.
import { fmtArea, fmtVol } from './units.js';
import './cites.js';

const { citeHtml } = globalThis.BuildsimCites;

const esc = v => String(v ?? '').replace(/[&<>"']/g, c => '&#' + c.charCodeAt(0) + ';');
const money = v => '$' + (+v).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
// CSS string literal content: escape \ and ", and drop line breaks; </style can't occur (no '<' allowed through)
const cssStr = v => String(v).replace(/[\\"]/g, c => '\\' + c).replace(/[\r\n<]/g, ' ');
const RANK = { error: 0, warning: 1, info: 2 };
const LABEL = { error: 'Error', warning: 'Warning', info: 'Info' };

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

<h2>Findings</h2>
${table(['##', 'Severity', 'Element / space', 'Finding', 'Citation'],
  findings.map((a, i) => `<tr><td class="num">${i + 1}</td><td class="sev ${esc(a.severity)}">${esc(LABEL[a.severity] || a.severity)}</td><td>${esc(a.subject)}</td><td>${esc(a.message)}</td><td>${citeHtml(a.cite)}</td></tr>`),
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
