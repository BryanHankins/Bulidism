// Unit-cost rates. The model stores integer cents per metric unit (per m², m³, m or each);
// the editor shows dollars per display unit (per SF, CY, LF in imperial).
import { M_PER_FT } from './units.js';

// metric units per one imperial display unit
const FACTOR = { M2: M_PER_FT * M_PER_FT, M3: 0.764554857984, M: M_PER_FT, Each: 1 };
const LABEL = { imperial: { M2: 'SF', M3: 'CY', M: 'LF', Each: 'EA' }, metric: { M2: 'm²', M3: 'm³', M: 'm', Each: 'EA' } };
// CSV "per" column -> [engine unit, system]
const PER = { SF: ['M2', 'imperial'], CY: ['M3', 'imperial'], LF: ['M', 'imperial'], EA: ['Each', 'metric'], EACH: ['Each', 'metric'],
  M2: ['M2', 'metric'], 'M²': ['M2', 'metric'], M3: ['M3', 'metric'], 'M³': ['M3', 'metric'], M: ['M', 'metric'] };

const factor = (unit, sys) => sys === 'imperial' ? (FACTOR[unit] ?? 1) : 1;
/** Label for a rate's denominator, e.g. 'SF'; '' when the code is not used by any element. */
export const perLabel = (unit, sys) => unit ? (LABEL[sys === 'imperial' ? 'imperial' : 'metric'][unit] || unit) : '';
/** Stored cents per metric unit -> dollars per display unit. */
export const rateOut = (cents, unit, sys) => cents / 100 * factor(unit, sys);
/** Dollars per display unit -> integer cents per metric unit. */
export const rateIn = (dollars, unit, sys) => Math.round(dollars * 100 / factor(unit, sys));
/** "$1,234.50" -> 1234.5; NaN for anything that is not a non-negative amount. */
export const parseDollars = s => { const t = String(s).trim().replace(/^\$/, '').replace(/,/g, ''); return /^(\d+\.?\d*|\.\d+)$/.test(t) ? +t : NaN; };

/**
 * One row per cost code in use or priced: { code, cents|null, unit|null, count, mixed }.
 * `elements` are report elements ({ cost_code, unit }); unit is null for unused codes.
 */
export function costRows(unitCosts, elements) {
  const by = new Map();
  for (const [code, cents] of Object.entries(unitCosts)) by.set(code, { code, cents, units: new Set(), count: 0 });
  for (const e of elements) {
    const r = by.get(e.cost_code) || { code: e.cost_code, cents: null, units: new Set(), count: 0 };
    r.count++; r.units.add(e.unit); by.set(e.cost_code, r);
  }
  return [...by.values()].sort((a, b) => a.code.localeCompare(b.code))
    .map(({ units, ...r }) => ({ ...r, unit: units.size ? [...units][0] : null, mixed: units.size > 1 }));
}

const field = v => /[",\r\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
export function ratesToCsv(rows, sys) {
  return 'code,rate_usd,per\n' + rows.filter(r => r.cents != null)
    .map(r => [field(r.code), rateOut(r.cents, r.unit, sys).toFixed(2), perLabel(r.unit, sys)].join(',')).join('\n') + '\n';
}

/** Parse "code,rate_usd,per" CSV (header optional, per may be blank = per metric unit/each). */
export function parseRatesCsv(text) {
  const rates = {}, errors = [];
  String(text).split(/\r?\n/).forEach((line, i) => {
    if (!line.trim()) return;
    const cells = [...line.matchAll(/("(?:[^"]|"")*"|[^,]*)(,|$)/g)].map(m => m[1].replace(/^"|"$/g, '').replace(/""/g, '"').trim()).slice(0, 3);
    const [code, rate, per = ''] = cells;
    if (i === 0 && /^code$/i.test(code)) return;
    const dollars = parseDollars(rate);
    const p = per ? PER[per.toUpperCase()] : ['Each', 'metric'];
    if (!code) errors.push(`line ${i + 1}: missing code`);
    else if (Number.isNaN(dollars)) errors.push(`line ${i + 1}: bad rate "${rate}"`);
    else if (!p) errors.push(`line ${i + 1}: unknown unit "${per}" (use SF, CY, LF, EA, m2, m3, m)`);
    else rates[code] = rateIn(dollars, p[0], p[1]);
  });
  return { rates, errors };
}
