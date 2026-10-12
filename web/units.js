// Display/input units. The engine is metric (metres, m², m³); the UI converts at the edges.
export const M_PER_IN = 0.0254;
export const M_PER_FT = 0.3048;
const M2_PER_SF = M_PER_FT * M_PER_FT;
const M3_PER_CY = 0.764554857984;

/** Grid for click-placement and drag snapping: 0.5 m or 6 in. */
export const snapStep = sys => sys === 'imperial' ? 6 * M_PER_IN : 0.5;

/** Feet-inches to the nearest 1/8", e.g. 3'-6 1/2" (or 6" under a foot). */
export function ftIn(m) {
  let e = Math.round(Math.abs(m) / M_PER_IN * 8); // eighths of an inch
  const sign = m < 0 && e ? '-' : '';
  const ft = Math.floor(e / 96); e -= ft * 96;
  const inch = Math.floor(e / 8); let n = e % 8, d = 8;
  while (n && n % 2 === 0) { n /= 2; d /= 2; }
  const inStr = `${inch}${n ? ` ${n}/${d}` : ''}"`;
  return sign + (ft ? `${ft}'-${inStr}` : inStr);
}

/** Length for an <input>: plain metres, or feet-inches. */
export const lenToInput = (m, sys) => sys === 'imperial' ? ftIn(m) : String(+(+m).toFixed(4));

/** Length for display text, with its unit. */
export const fmtLen = (m, sys) => sys === 'imperial' ? ftIn(m) : `${+(+m).toFixed(3)} m`;

/**
 * Parse a typed length to metres (NaN if unreadable). Imperial accepts 3'-6", 3' 6 1/2", 42",
 * 6 1/2", 3.5', 3ft 6in, and a bare number as feet (3.5 = 3'-6").
 */
export function parseLen(s, sys) {
  s = String(s).trim();
  if (sys !== 'imperial') return /^[-+]?(\d+\.?\d*|\.\d+)$/.test(s) ? +s : NaN;
  const r = s.match(/^([-+])?\s*(?:(\d+(?:\.\d+)?|\.\d+)\s*(?:'|ft))?\s*-?\s*(?:(\d+(?:\.\d+)?|\.\d+)?(?:\s*(\d+)\/(\d+))?\s*("|in)?)?$/);
  if (!r) return NaN;
  const [, sign, ft, inch, fn, fd, inMark] = r;
  if (ft === undefined && inch === undefined && fn === undefined) return NaN;
  if (fd !== undefined && +fd === 0) return NaN;
  const frac = fn !== undefined ? fn / fd : 0;
  // a bare number with no ' or " marks is feet, as in most CAD tools
  const bareFeet = ft === undefined && !inMark && fn === undefined && !/'|ft/.test(s);
  const inches = bareFeet ? 0 : (+(inch || 0) + frac);
  const feet = bareFeet ? +inch : +(ft || 0);
  const m = feet * M_PER_FT + inches * M_PER_IN;
  return sign === '-' ? -m : m;
}

export const fmtArea = (m2, sys) => sys === 'imperial' ? `${Math.round(m2 / M2_PER_SF).toLocaleString()} sf` : `${(+m2).toFixed(1)} m²`;
export const fmtVol = (m3, sys) => sys === 'imperial' ? `${(m3 / (M_PER_FT ** 3)).toFixed(1)} cf` : `${(+m3).toFixed(3)} m³`;
export const areaToInput = (m2, sys) => sys === 'imperial' ? String(Math.round(m2 / M2_PER_SF)) : String(+(+m2).toFixed(2));
export const parseArea = (s, sys) => { const v = /^\s*(\d+\.?\d*|\.\d+)\s*$/.test(s) ? +s : NaN; return sys === 'imperial' ? v * M2_PER_SF : v; };

/** Takeoff quantity in display units: m2→SF, m3→CY, m→LF (costs stay as computed). */
export function qtyOut(qty, unit, sys) {
  if (sys !== 'imperial') return [qty, unit];
  switch (unit) {
    case 'M2': return [qty / M2_PER_SF, 'SF'];
    case 'M3': return [qty / M3_PER_CY, 'CY'];
    case 'M': return [qty / M_PER_FT, 'LF'];
    default: return [qty, unit];
  }
}

/** Rewrite metric lengths in engine messages ("0.686 m") for display. */
export const convertMessage = (msg, sys) => sys === 'imperial'
  ? String(msg).replace(/(-?\d+(?:\.\d+)?) m(?![\w²³])/g, (_, v) => ftIn(+v))
  : msg;
