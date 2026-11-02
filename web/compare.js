// Version comparison display: the engine's `comparison` (changes since a baseline version)
// as the report-pane panel. Pure functions of the report, so they test without a browser.
import { fmtMeasure } from './units.js';

const esc = v => String(v ?? '').replace(/[&<>"']/g, c => '&#' + c.charCodeAt(0) + ';');
const money = v => '$' + Math.round(v).toLocaleString('en-US');
const SEV = { error: 'Error', warning: 'Warning' };
export const FINDING_STATUS = { added: 'New', changed: 'Changed', removed: 'Resolved' };
export const MARK = { added: '+', changed: '~', removed: '−' };

/** A total's before / after / delta in display form; delta '' when unchanged. */
export function totalText(t) {
  const f = t.unit === 'usd' ? money : t.unit === 'days' ? v => `${v} d` : v => String(v);
  const d = t.after - t.before;
  const delta = !d ? '' : t.unit === 'usd' ? `${d > 0 ? '+' : '−'}${money(Math.abs(d))}` : `${d > 0 ? '+' : '−'}${Math.abs(d)}${t.unit === 'days' ? ' d' : ''}`;
  return { before: f(t.before), after: f(t.after), delta };
}

/** Measured value change of a "changed" finding, e.g. 2'-3" → 2'-1"; '' otherwise. */
export function measuredChange(f, units) {
  if (f.status !== 'changed') return '';
  return `${fmtMeasure(f.measured_before, f.finding.measure, units)} → ${fmtMeasure(f.finding.measured, f.finding.measure, units)}`;
}

/** Element ids worth highlighting in the current model: added and changed elements, and both
 * sides of new clashes (removed elements are no longer in the model). */
export function changedIds(c, present = () => true) {
  const ids = new Set();
  for (const e of c.elements) if (e.status !== 'removed') ids.add(e.id);
  for (const k of c.clashes) if (k.status === 'added') { ids.add(k.a); ids.add(k.b); }
  return [...ids].filter(present);
}

/** Counts for the panel summary line. */
export function summary(c) {
  const n = (list, s) => list.filter(x => x.status === s).length;
  return {
    newFindings: n(c.findings, 'added'), resolved: n(c.findings, 'removed'), changedFindings: n(c.findings, 'changed'),
    newClashes: n(c.clashes, 'added'), resolvedClashes: n(c.clashes, 'removed'),
    added: n(c.elements, 'added'), changed: n(c.elements, 'changed'), removed: n(c.elements, 'removed'),
  };
}

const LIMIT = 100;
// rows carry data-ids (space-separated element ids) so a click can select them
const row = (left, right, ids, title = '') =>
  `<div class="row${ids && ids.length ? ' pick' : ''}"${ids && ids.length ? ` data-ids="${esc(ids.join(' '))}"` : ''}${title ? ` title="${esc(title)}"` : ''}><span>${left}</span><span>${right}</span></div>`;
const capped = (items, fn) => items.slice(0, LIMIT).map(fn).join('') +
  (items.length > LIMIT ? `<div class="row"><small style="color:var(--muted)">and ${items.length - LIMIT} more (print for the full list)</small></div>` : '');

/**
 * The report-pane panel. `label` names the baseline (file name or when it was set);
 * `present(id)` says whether an element is in the current model (so it can be selected).
 */
export function comparePanel(c, { label, units, present = () => true }) {
  const s = summary(c);
  const changedTotals = c.totals.filter(t => t.before !== t.after);
  const head = `<div class="row"><span>vs <b>${esc(label || c.label || c.baseline)}</b></span><span>` +
    `<button class="btn" id="cmpHighlight" title="Select the added and changed elements and both sides of new clashes">Highlight</button> ` +
    `<button class="btn" id="cmpClear" title="Stop comparing">Clear</button></span></div>` +
    (c.rule_set_differs ? `<div class="row"><small style="color:var(--muted)">The versions name different rule sets; both were checked against the current one.</small></div>` : '');
  const nothing = !c.elements.length && !c.spaces.length && !c.findings.length && !c.clashes.length && !changedTotals.length;
  if (nothing) return head + `<div class="row"><span>No changes</span></div>`;
  const totals = changedTotals.map(t => { const x = totalText(t); return row(esc(t.label), `${x.before} → ${x.after} <b>${x.delta}</b>`); }).join('');
  const findings = c.findings.length ? `<h3>Findings <small>${s.newFindings} new · ${s.changedFindings} changed · ${s.resolved} resolved · ${c.unchanged_findings} unchanged</small></h3>` +
    capped(c.findings, f => {
      const a = f.finding, ids = a.element && present(a.element) ? [a.element] : [];
      const m = measuredChange(f, units);
      return row(`<b class="cmp-${f.status}">${FINDING_STATUS[f.status]}</b> <span style="color:var(--${a.severity === 'error' ? 'err' : 'warn'})">${SEV[a.severity] || esc(a.severity)}</span> ${esc(a.subject)}`,
        m ? esc(m) : '', ids, a.message);
    }) : '';
  const clashes = c.clashes.length ? `<h3>Clashes <small>${s.newClashes} new · ${s.resolvedClashes} resolved</small></h3>` +
    capped(c.clashes, k => row(`<b class="cmp-${k.status}">${k.status === 'added' ? 'New' : 'Resolved'}</b> ${esc(k.a_name)} ↔ ${esc(k.b_name)}`, '', [k.a, k.b].filter(present))) : '';
  const elements = c.elements.length ? `<details${c.elements.length <= 12 ? ' open' : ''}><summary><b>Elements</b> <small>${s.added} added · ${s.changed} changed · ${s.removed} removed</small></summary>` +
    capped(c.elements, e => row(`<b>${MARK[e.status]}</b> ${esc(e.name)}`,
      `<small style="color:var(--muted)">${esc(e.aspects.join(', '))}</small>`, e.status === 'removed' ? [] : [e.id].filter(present))) + '</details>' : '';
  const spaces = c.spaces.length ? `<details open><summary><b>Spaces</b></summary>` +
    c.spaces.map(sp => row(`<b>${MARK[sp.status]}</b> ${esc(sp.name)}`, `<small style="color:var(--muted)">${esc(sp.aspects.join(', '))}</small>`)).join('') + '</details>' : '';
  return head + totals + findings + clashes + elements + spaces;
}
