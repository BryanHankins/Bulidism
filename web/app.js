// Live editor: JS owns the project JSON; every edit re-runs the Rust pipeline in wasm.
import init, * as bim from './pkg/bim_wasm.js';
import { printHtml } from './print.js';
import { setLevelElevation } from './levels.js';
import { costRows, perLabel, rateOut, rateIn, parseDollars, ratesToCsv, parseRatesCsv } from './costs.js';
import { snapStep, lenToInput, parseLen, fmtArea, areaToInput, parseArea, qtyOut, convertMessage } from './units.js';
import { kindOf, clickSelect, deleteElements, moveElements, copyElements, pasteElements, rectBounds, commonValue, dependsCycle } from './clipboard.js';
import { createCloud } from './cloud.js';

const $ = id => document.getElementById(id);
// user-supplied strings (names, cost codes) go into innerHTML: always escape
const esc = v => String(v).replace(/[&<>"']/g, c => '&#' + c.charCodeAt(0) + ';');
// sel: selected element ids (any number); clip: copied elements (deep copies) for Ctrl+V
let project = null, rules = null, rulesId = null, lastDay = 0, lastLevel = 'all', activeLevel = null, sel = [], clip = null, placing = null, placeStart = null, lastReport = null;
const one = () => sel.length === 1 ? sel[0] : null;
const undoStack = [], redoStack = [];
const store = { get(k) { try { return localStorage.getItem(k); } catch (_) { return null; } }, set(k, v) { try { localStorage.setItem(k, v); return true; } catch (_) { return false; } } };
// display units only; the engine stays metric
let units = store.get('buildsim.units') || (navigator.language === 'en-US' ? 'imperial' : 'metric');
const AUTOSAVE_KEY = 'buildsim.autosave.v1';
let notice = '';
// synchronous on every change: a debounce loses the last edit when the tab closes or reloads
function autosave() {
  if (!store.set(AUTOSAVE_KEY, JSON.stringify({ saved_at: new Date().toISOString(), project }))) status($('status').textContent + ' · autosave unavailable');
}
/** Last autosaved project if it still loads (right schema). It may fail validation: restore it
 * anyway so the user sees the errors instead of silently losing their work to the sample. */
function restoreAutosave() {
  try {
    const s = JSON.parse(store.get(AUTOSAVE_KEY));
    if (!s || !s.project) return null;
    bim.validate(JSON.stringify(s.project)); // throws on bad JSON shape or schema version
    return s;
  } catch (_) { return null; }
}
const isLen = n => n !== 'riser_count';
/** <input> for a length in the current display units. */
const lenInput = (attrs, m) => units === 'imperial'
  ? `<input ${attrs} type="text" inputmode="decimal" value="${esc(lenToInput(m, units))}">`
  : `<input ${attrs} type="number" step="0.05" value="${esc(lenToInput(m, units))}">`;
const badLen = v => { $('runErr').textContent = `Could not read length "${v}"` + (units === 'imperial' ? ` — try 3'-6", 42" or 3.5'` : ''); };
// one undo step per operation; `before` lets an operation decide after the fact whether it changed anything
function snapshot(before) { if (project) { undoStack.push(before ?? JSON.stringify(project)); if (undoStack.length > 50) undoStack.shift(); redoStack.length = 0; updateUndo(); } }
function updateUndo() { $('undo').disabled = !undoStack.length; $('redo').disabled = !redoStack.length; }
const pruneSel = () => { sel = sel.filter(id => project.elements.some(e => e.id === id)); };
function undo() { if (!undoStack.length) return; showImportNotes(); redoStack.push(JSON.stringify(project)); project = JSON.parse(undoStack.pop()); pruneSel(); updateUndo(); rerun(); }
function redo() { if (!redoStack.length) return; showImportNotes(); undoStack.push(JSON.stringify(project)); project = JSON.parse(redoStack.pop()); pruneSel(); updateUndo(); rerun(); }
const step = () => snapStep(units);
const snap = v => Math.round(v / step()) * step();

const KINDS = {
  Wall:   { fields: { sx:['start x',0], sz:['start z',0], ex:['end x',6], ez:['end z',0], height:['height',3], thickness:['thickness',0.25] },
            build: f => ({ Wall: { start:[f.sx,lvlY(),f.sz], end:[f.ex,lvlY(),f.ez], height:f.height, thickness:f.thickness } }), disc:'Architectural', code:'B2010' },
  Column: { fields: { x:['x',0], z:['z',0], width:['width',0.4], depth:['depth',0.4], height:['height',3.2] },
            build: f => ({ Column: { base:[f.x,lvlY(),f.z], width:f.width, depth:f.depth, height:f.height } }), disc:'Structural', code:'B1010' },
  Slab:   { fields: { x:['origin x',0], z:['origin z',0], y:['elevation',0], size_x:['size x',12], size_z:['size z',8], thickness:['thickness',0.2] },
            build: f => ({ Slab: { origin:[f.x,f.y,f.z], size_x:f.size_x, size_z:f.size_z, thickness:f.thickness } }), disc:'Structural', code:'A1010' },
  Beam:   { fields: { sx:['start x',0], sy:['start y',3.4], sz:['start z',0], ex:['end x',12], ez:['end z',0], width:['width',0.3], depth:['depth',0.5] },
            build: f => ({ Beam: { start:[f.sx,f.sy,f.sz], end:[f.ex,f.sy,f.ez], width:f.width, depth:f.depth } }), disc:'Structural', code:'B1010' },
  Door:   { fields: { host:['host wall','wall'], offset:['offset',1], width:['width',0.9], height:['height',2.1], is_egress:['egress','bool'] },
            build: f => ({ Door: { host:f.host, offset:f.offset, width:f.width, height:f.height, is_egress:!!f.is_egress } }), disc:'Architectural', code:'C1020' },
  Stair:  { fields: { x:['x',1], z:['z',3], run_length:['run',3.5], total_rise:['rise',3], width:['width',1.2], riser_count:['risers',17] },
            build: f => ({ Stair: { base:[f.x,lvlY(),f.z], run_length:f.run_length, total_rise:f.total_rise, width:f.width, riser_count:Math.round(f.riser_count) } }), disc:'Architectural', code:'C2010' },
  Ramp:   { fields: { x:['x',6], z:['z',-3], run_length:['run',2.4], rise:['rise',0.2], width:['width',1.2] },
            build: f => ({ Ramp: { base:[f.x,lvlY(),f.z], run_length:f.run_length, rise:f.rise, width:f.width } }), disc:'Site', code:'G2030' },
};

// ---- levels: new elements go on the active level, at its elevation ----
const sortedLevels = () => [...(project.levels || [])].sort((a, b) => a.elevation_m - b.elevation_m || a.id - b.id);
function currentLevel() { const ls = sortedLevels(); return ls.find(l => l.id === activeLevel) || ls[0] || null; }
const lvlY = () => { const l = currentLevel(); return l ? l.elevation_m : 0; };
const levelOptions = sel => '<option value="">(none / site)</option>' + sortedLevels().map(l => `<option value="${l.id}" ${l.id === sel ? 'selected' : ''}>${esc(l.name)}</option>`).join('');

function uuid(){ return crypto.randomUUID ? crypto.randomUUID() : 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g,c=>{const r=Math.random()*16|0;return (c==='x'?r:(r&3|8)).toString(16)}); }
function status(t){ $('status').textContent = t; }

// Rules follow project.rule_set; a missing file means no advisories, never another jurisdiction's.
async function loadRules(id) {
  rulesId = id; rules = null;
  try { const r = await fetch(`rules/${encodeURIComponent(id)}.json`); if (r.ok) rules = await r.text(); } catch (_) { /* offline: run without rules */ }
}

function rerun(keepDay = true) {
  if (rulesId !== project.rule_set) { loadRules(project.rule_set).then(() => rerun(keepDay)); return; }
  $('runErr').textContent = '';
  try {
    const t0 = performance.now();
    const report = JSON.parse(bim.run(JSON.stringify(project), rules));
    for (const a of report.advisories) a.message = convertMessage(a.message, units);
    lastReport = report;
    $('spaceList').innerHTML = report.spaces.length
      ? report.spaces.map(s => `<div class="row"><span>${esc(s.name)} <small style="color:var(--muted)">${s.occupancy}</small></span><span>${fmtArea(s.area_m2, units)} · ${s.exits} exit${s.exits===1?'':'s'}</span></div>`).join('')
      : '<div class="row"><span>No spaces defined</span></div>';
    renderReport(report, { day: keepDay ? Math.min(lastDay, report.total_days) : 0, selected: sel, onClick: onViewClick, onMove: onViewMove, onHandle: onViewHandle, handlesFor, onDay: d => lastDay = d, level: lastLevel, onLevel: l => lastLevel = l, groundY: lvlY(),
      rectMode: () => placing === 'space', onRect: onViewRect, snapRect: v => Math.round(v / 0.5) * 0.5 });
    status(`ran in ${(performance.now()-t0).toFixed(1)} ms · schema v${bim.schema_version()}${rules ? '' : ` · no rule file for '${project.rule_set}', advisories skipped`}${notice}`);
    notice = '';
  } catch (e) {
    $('runErr').textContent = String(e);
  }
  renderEditor(); renderProps(); renderSpaces(); renderCosts(); renderLevels(); renderRuleSets(); renderCloud(); $('jsonBox').value = JSON.stringify(project, null, 2);
  autosave();
}

// ---- rule set: rules/index.json lists what ships; the project's own id is always offered ----
let ruleIndex = null;
async function loadRuleIndex() {
  try { const r = await fetch('rules/index.json'); if (r.ok) ruleIndex = await r.json(); } catch (_) { /* static copy without an index */ }
  if (!Array.isArray(ruleIndex)) ruleIndex = [];
  renderRuleSets();
}
function renderRuleSets() {
  if (!project) return;
  const list = [...(ruleIndex || [])];
  if (!list.some(r => r.id === project.rule_set)) list.push({ id: project.rule_set, name: `${project.rule_set} (no listing)` });
  $('ruleSetEdit').innerHTML = `<label class="chk">Rule set<select id="ruleSet">${list.map(r => `<option value="${esc(r.id)}" ${r.id === project.rule_set ? 'selected' : ''}>${esc(r.name || r.id)}</option>`).join('')}</select></label>`;
  $('ruleSet').onchange = ev => { snapshot(); project.rule_set = ev.target.value; notice = ` · rule set ${project.rule_set}`; rerun(); };
}


// ---- selection + property editing ----
const NUMERIC = { Wall:['height','thickness'], Slab:['size_x','size_z','thickness'], Column:['width','depth','height'], Beam:['width','depth'], Door:['offset','width','height'], Stair:['run_length','total_rise','width','riser_count'], Ramp:['run_length','rise','width'], Mesh:[] };
const VEC = { Wall:['start','end'], Slab:['origin'], Column:['base'], Beam:['start','end'], Stair:['base'], Ramp:['base'], Mesh:['origin'], Door:[] };
const DISCIPLINES = ['Architectural','Structural','Mechanical','Electrical','Plumbing','Site'];
const selectedEls = () => { const s = new Set(sel); return project.elements.filter(e => s.has(e.id)); };
/** Change the selection (viewer highlight, property panel, element list) without re-running the engine. */
function select(ids) { sel = ids; if (window.__viewer) window.__viewer.highlight(sel); renderProps(); renderElementList(); }
function renderProps() {
  const els = selectedEls(); $('selPanel').hidden = !els.length; if (!els.length) return;
  $('selTitle').textContent = els.length === 1 ? 'Selected' : `${els.length} elements selected`;
  if (els.length > 1) return renderMultiProps(els);
  const e = els[0];
  const kind = kindOf(e), k = e.kind[kind];
  const phases = project.phases.map(p => `<option value="${p.id}" ${p.id===e.phase?'selected':''}>${esc(p.name)}</option>`).join('');
  let h = `<label class="full">Name<input name="name" value="${esc(e.name)}"></label><label>Phase<select name="phase">${phases}</select></label><label>Offset days<input name="start_offset_days" type="number" min="0" value="${e.start_offset_days||0}"></label><label>Cost code<input name="cost_code" list="costCodes" value="${esc(e.cost_code)}"></label><label>Discipline<select name="discipline">${DISCIPLINES.map(d=>`<option ${d===e.discipline?'selected':''}>${d}</option>`).join('')}</select></label><label>Level<select name="level">${levelOptions(e.level)}</select></label>`;
  for (const v of VEC[kind]) h += ['x','y','z'].map((ax,i) => `<label>${v} ${ax}${lenInput(`name="${v}.${i}"`, k[v][i])}</label>`).join('');
  for (const n of NUMERIC[kind]) h += isLen(n) ? `<label>${n}${lenInput(`name="${n}"`, k[n])}</label>` : `<label>${n}<input name="${n}" type="number" step="1" value="${k[n]}"></label>`;
  if (kind === 'Door') h += `<label>egress<select name="is_egress"><option value="1" ${k.is_egress?'selected':''}>yes</option><option value="" ${!k.is_egress?'selected':''}>no</option></select></label>`;
  // element-level schedule: own duration and predecessors (engine: depends_on / duration_days)
  const byId = new Map(project.elements.map(x => [x.id, x])), deps = e.depends_on || [];
  const label = x => `${esc(x.name)} (${kindOf(x)})`;
  const others = project.elements.filter(x => x.id !== e.id && !deps.includes(x.id) && !dependsCycle(project, e.id, x.id));
  h += `<label>Duration days<input name="duration_days" type="number" min="0" step="1" placeholder="none" value="${e.duration_days ?? ''}"></label>
    <div class="full" style="font-size:11px;color:var(--muted)">Depends on${deps.length ? '' : ': nothing'}
    ${deps.map(d => `<div class="el"><span>${byId.has(d) ? label(byId.get(d)) : esc(d)}</span><button type="button" data-undep="${esc(d)}" title="Remove dependency">✕</button></div>`).join('')}
    <select id="depAdd" style="width:100%;margin-top:4px"><option value="">+ add predecessor…</option>${others.map(x => `<option value="${esc(x.id)}">${label(x)}</option>`).join('')}</select></div>`;
  $('propForm').innerHTML = h;
  $('propForm').querySelectorAll('input[name],select[name]').forEach(inp => inp.onchange = () => {
    const n = inp.name, val = inp.value;
    const len = (n.includes('.') || (n in k && isLen(n) && n !== 'is_egress')) ? parseLen(val, units) : null;
    if (Number.isNaN(len)) { badLen(val); renderProps(); return; }
    snapshot();
    if (n === 'name' || n === 'cost_code' || n === 'discipline') e[n] = val;
    else if (n === 'phase' || n === 'start_offset_days') e[n] = +val;
    else if (n === 'duration_days') { if (val.trim() === '') delete e.duration_days; else e.duration_days = Math.max(0, Math.round(+val) || 0); }
    else if (n === 'level') e.level = val === '' ? null : +val;
    else if (n === 'is_egress') k.is_egress = !!val;
    else if (n.includes('.')) { const [v,i] = n.split('.'); k[v][+i] = len; }
    else k[n] = n === 'riser_count' ? Math.max(1, Math.round(+val)) : len;
    rerun();
  });
  $('depAdd').onchange = ev => { const d = ev.target.value; if (!d) return; snapshot(); e.depends_on = [...deps, d]; rerun(); };
  $('propForm').querySelectorAll('[data-undep]').forEach(b => b.onclick = () => { snapshot(); e.depends_on = deps.filter(d => d !== b.dataset.undep); if (!e.depends_on.length) delete e.depends_on; rerun(); });
}
// several selected: only the fields they can share; a differing value shows as "(mixed)" and is left alone
function renderMultiProps(els) {
  const ph = commonValue(els, 'phase'), dc = commonValue(els, 'discipline'), lv = commonValue(els, 'level'), cc = commonValue(els, 'cost_code');
  const mixed = v => v === undefined ? '<option value="__mixed" selected>(mixed)</option>' : '';
  const levelSel = lv === undefined ? mixed(lv) + levelOptions(Symbol()) : levelOptions(lv);
  $('propForm').innerHTML = `<label>Phase<select name="phase">${mixed(ph)}${project.phases.map(p => `<option value="${p.id}" ${p.id===ph?'selected':''}>${esc(p.name)}</option>`).join('')}</select></label>
    <label>Discipline<select name="discipline">${mixed(dc)}${DISCIPLINES.map(d => `<option ${d===dc?'selected':''}>${d}</option>`).join('')}</select></label>
    <label>Level<select name="level">${levelSel}</select></label>
    <label>Cost code<input name="cost_code" list="costCodes" value="${esc(cc ?? '')}" placeholder="${cc === undefined ? '(mixed)' : ''}"></label>`;
  $('propForm').querySelectorAll('[name]').forEach(inp => inp.onchange = () => {
    const n = inp.name, val = inp.value;
    if (val === '__mixed' || (n === 'cost_code' && val.trim() === '' && cc === undefined)) return;
    snapshot();
    for (const e of els) e[n] = n === 'phase' ? +val : n === 'level' ? (val === '' ? null : +val) : val;
    rerun();
  });
}
function deleteSelected() { if (!sel.length) return; snapshot(); const n = deleteElements(project, sel); sel = []; notice = ` · deleted ${n} element${n === 1 ? '' : 's'}`; rerun(); }
// clipboard: in-page; each paste lands +1 m in X and Z from the last, snapped to the display grid
function copySel() { if (!sel.length) return false; clip = copyElements(project, sel); status(`copied ${clip.length} element${clip.length === 1 ? '' : 's'}`); return true; }
function paste() {
  if (!clip || !clip.length) return;
  const before = JSON.stringify(project), out = pasteElements(project, clip, { uuid, dx: 1, dz: 1, snap });
  if (!out.length) { status('nothing to paste: the copied doors\' walls no longer exist'); return; }
  snapshot(before); clip = JSON.parse(JSON.stringify(out)); sel = out.map(e => e.id);
  notice = ` · pasted ${out.length} element${out.length === 1 ? '' : 's'}`; rerun();
}
$('delSel').onclick = deleteSelected;
$('dupSel').onclick = () => { if (copySel()) paste(); };
$('deselect').onclick = () => select([]);
function selectTab(t){ document.querySelectorAll('.tabs .btn').forEach(x => x.setAttribute('aria-selected', x.dataset.tab === t)); document.querySelectorAll('.pane').forEach(p => p.hidden = p.id !== 'pane-' + t); }

// ---- endpoint handles ----
// Returns the draggable points of an element: [start, end] for walls/beams, corners for slabs.
function handlesFor(id) {
  const e = project.elements.find(x => x.id === id); if (!e) return [];
  const kind = Object.keys(e.kind)[0], k = e.kind[kind];
  if (kind === 'Wall' || kind === 'Beam') return [k.start, k.end];
  if (kind === 'Slab') return [k.origin, [k.origin[0] + k.size_x, k.origin[1], k.origin[2] + k.size_z]];
  return [];
}
function onViewHandle(id, idx, x, z) {
  const e = project.elements.find(v => v.id === id); if (!e) return;
  const kind = Object.keys(e.kind)[0], k = e.kind[kind];
  const [sx, sz] = [snap(x), snap(z)];
  snapshot();
  if (kind === 'Wall' || kind === 'Beam') { const t = idx === 0 ? k.start : k.end; t[0] = sx; t[2] = sz; }
  else if (kind === 'Slab') {
    if (idx === 0) { const fx = k.origin[0] + k.size_x, fz = k.origin[2] + k.size_z; k.origin[0] = Math.min(sx, fx - 0.5); k.origin[2] = Math.min(sz, fz - 0.5); k.size_x = fx - k.origin[0]; k.size_z = fz - k.origin[2]; }
    else { k.size_x = Math.max(0.5, sx - k.origin[0]); k.size_z = Math.max(0.5, sz - k.origin[2]); }
  }
  rerun();
}

// ---- spaces ----
const OCCUPANCIES = ['Assembly','Business','Educational','Residential','Mercantile','Storage','Institutional'];
// occupancy for new spaces (+ space and the drag tool), remembered per browser
let spaceOcc = OCCUPANCIES.includes(store.get('buildsim.spaceOcc')) ? store.get('buildsim.spaceOcc') : 'Business';
function renderSpaces() {
  const doors = project.elements.filter(e => e.kind.Door);
  $('spaceEdit').innerHTML = `<label class="chk" style="font-size:12px">New spaces are<select id="spaceOcc">${OCCUPANCIES.map(o => `<option ${o === spaceOcc ? 'selected' : ''}>${o}</option>`).join('')}</select></label>` + project.spaces.map((s, i) => {
    const opts = OCCUPANCIES.map(o => `<option ${o === s.occupancy ? 'selected' : ''}>${o}</option>`).join('');
    const exits = doors.map(d => `<label style="display:block;font-size:12px"><input type="checkbox" data-sp="${i}" data-door="${d.id}" ${s.egress_doors.includes(d.id) ? 'checked' : ''}> ${esc(d.name)}${d.kind.Door.is_egress ? '' : ' (not egress)'}</label>`).join('') || '<small style="color:var(--muted)">no doors yet</small>';
    const b = s.bounds;
    return `<div style="border:1px solid var(--line);border-radius:6px;padding:8px;margin-bottom:8px">
      <div style="display:flex;gap:6px"><input data-sp="${i}" data-f="name" value="${esc(s.name)}" style="flex:1"><button data-del="${i}" class="btn">✕</button></div>
      <label class="chk">Occupancy<select data-sp="${i}" data-f="occupancy">${opts}</select></label>
      <label class="chk">Level<select data-sp="${i}" data-f="level">${levelOptions(s.level)}</select></label>
      <label class="chk" style="font-size:12px"><input type="checkbox" data-sp="${i}" data-f="useBounds" ${b ? 'checked' : ''}> area from plan rectangle</label>
      ${b ? `<div style="display:grid;grid-template-columns:1fr 1fr;gap:4px">${['x','z','size x','size z'].map((l, j) => `<label style="font-size:11px;color:var(--muted)">${l}${lenInput(`data-sp="${i}" data-b="${j}" style="width:100%"`, b[j])}</label>`).join('')}</div>`
          : `<label class="chk" style="font-size:12px">Area ${units === 'imperial' ? 'sf' : 'm²'}<input type="number" step="1" data-sp="${i}" data-f="floor_area_m2" value="${areaToInput(s.floor_area_m2, units)}" style="width:80px"></label>`}
      <div style="margin-top:6px;font-size:11px;color:var(--muted)">Egress doors</div>${exits}</div>`;
  }).join('') + '<button class="btn" id="addSpace">+ space</button>';
  $('spaceEdit').querySelectorAll('[data-f],[data-b]').forEach(inp => inp.onchange = () => {
    const v = inp.dataset.b !== undefined ? parseLen(inp.value, units) : inp.dataset.f === 'floor_area_m2' ? parseArea(inp.value, units) : 0;
    if (Number.isNaN(v)) { badLen(inp.value); renderSpaces(); return; }
    snapshot(); const sp = project.spaces[+inp.dataset.sp];
    if (inp.dataset.b !== undefined) { sp.bounds[+inp.dataset.b] = v; sp.floor_area_m2 = sp.bounds[2] * sp.bounds[3]; }
    else if (inp.dataset.f === 'useBounds') { sp.bounds = inp.checked ? [0, 0, Math.sqrt(sp.floor_area_m2) || 4, Math.sqrt(sp.floor_area_m2) || 4] : null; if (sp.bounds) sp.floor_area_m2 = sp.bounds[2] * sp.bounds[3]; }
    else if (inp.dataset.f === 'floor_area_m2') sp.floor_area_m2 = v;
    else if (inp.dataset.f === 'level') sp.level = inp.value === '' ? null : +inp.value;
    else sp[inp.dataset.f] = inp.value;
    rerun();
  });
  $('spaceEdit').querySelectorAll('[data-door]').forEach(cb => cb.onchange = () => {
    snapshot(); const sp = project.spaces[+cb.dataset.sp], id = cb.dataset.door;
    sp.egress_doors = cb.checked ? [...new Set([...sp.egress_doors, id])] : sp.egress_doors.filter(d => d !== id);
    rerun();
  });
  $('spaceEdit').querySelectorAll('[data-del]').forEach(b => b.onclick = () => { snapshot(); project.spaces.splice(+b.dataset.del, 1); rerun(); });
  $('addSpace').onclick = () => { snapshot(); project.spaces.push({ name: `Space ${project.spaces.length+1}`, occupancy: spaceOcc, floor_area_m2: 50, egress_doors: [], bounds: null, level: currentLevel()?.id ?? null }); rerun(); };
  $('spaceOcc').onchange = ev => { spaceOcc = ev.target.value; store.set('buildsim.spaceOcc', spaceOcc); };
}
$('showSpaces').onchange = e => window.__viewer.setSpacesVisible(e.target.checked);

// ---- unit costs ----
// takeoff unit per element kind; mirrors bim_takeoff::quantity
const UNIT_OF = { Wall: 'M2', Slab: 'M3', Column: 'M3', Beam: 'M3', Door: 'Each', Stair: 'M', Ramp: 'M2', Mesh: 'Each' };
function rateRows() {
  return costRows(project.unit_costs, project.elements.map(e => ({ cost_code: e.cost_code, unit: UNIT_OF[Object.keys(e.kind)[0]] })));
}
function renderCosts() {
  const rows = rateRows(), unitOf = code => (rows.find(r => r.code === code) || {}).unit || null;
  const shown = (r) => r.cents == null ? '' : rateOut(r.cents, r.unit, units).toFixed(2);
  $('costEdit').innerHTML = `<table class="costs"><thead><tr><th>Code</th><th>Used by</th><th>Rate</th><th></th></tr></thead><tbody>${rows.map(r => {
    const per = perLabel(r.unit, units);
    const used = r.count ? `${r.count} element${r.count === 1 ? '' : 's'}${r.mixed ? ' · <b title="one code used for different units; split it">mixed units</b>' : ''}` : 'unused';
    const rate = r.code ? `$<input data-code="${esc(r.code)}" value="${shown(r)}" placeholder="${r.count ? 'missing' : ''}" inputmode="decimal"> ${per ? '/' + per : ''}` : '<small>give these a cost code</small>';
    return `<tr${r.cents == null && r.count ? ' class="missing"' : ''}><td>${r.code ? esc(r.code) : '(none)'}</td><td>${used}</td><td>${rate}</td><td>${r.cents != null ? `<button class="btn" data-delcode="${esc(r.code)}" title="Remove rate">✕</button>` : ''}</td></tr>`;
  }).join('')}</tbody></table>
  <div style="display:flex;flex-wrap:wrap;gap:6px;margin:6px 0 12px"><input id="newCode" placeholder="code" size="8"><input id="newRate" placeholder="$ rate" size="7" inputmode="decimal"><button class="btn" id="addRate">Add</button><button class="btn" id="exportRates">Export CSV</button><button class="btn" id="importRates">Import CSV</button></div>`;
  $('costCodes').innerHTML = rows.filter(r => r.code).map(r => `<option value="${esc(r.code)}">`).join('');
  const bad = v => { $('runErr').textContent = `Enter a rate in dollars, like 12.50 (got "${v}")`; };
  $('costEdit').querySelectorAll('input[data-code]').forEach(inp => inp.onchange = () => {
    const r = rows.find(x => x.code === inp.dataset.code), v = inp.value.trim();
    if (v === '' || v === shown(r)) { renderCosts(); return; } // unchanged: don't let display rounding drift the stored rate
    const d = parseDollars(v); if (Number.isNaN(d)) { bad(v); renderCosts(); return; }
    snapshot(); project.unit_costs[r.code] = rateIn(d, r.unit, units); rerun();
  });
  $('costEdit').querySelectorAll('[data-delcode]').forEach(b => b.onclick = () => { snapshot(); delete project.unit_costs[b.dataset.delcode]; rerun(); });
  $('addRate').onclick = () => {
    const code = $('newCode').value.trim(), v = $('newRate').value, d = parseDollars(v);
    if (!code) { $('runErr').textContent = 'Enter a cost code.'; return; }
    if (Number.isNaN(d)) { bad(v); return; }
    snapshot(); project.unit_costs[code] = rateIn(d, unitOf(code), units); rerun();
  };
  $('exportRates').onclick = () => download(`${project.name.replace(/\W+/g,'_')}_rates.csv`, ratesToCsv(rows, units), 'text/csv');
  $('importRates').onclick = () => $('ratesFile').click();
}
$('ratesFile').onchange = async ev => {
  const f = ev.target.files[0]; ev.target.value = ''; if (!f) return;
  const { rates, errors } = parseRatesCsv(await f.text());
  if (errors.length) { $('runErr').textContent = 'Rates not imported:\n' + errors.join('\n'); return; }
  snapshot(); Object.assign(project.unit_costs, rates); notice = ` · imported ${Object.keys(rates).length} rates`; rerun();
};

// ---- click-to-place ----
const PLACE_MSG = { wall: 'click start point', column: 'click position', door: 'click a wall', space: 'drag a rectangle on the floor (0.5 m grid; Esc cancels)' };
function setPlacing(mode){ placing = mode; placeStart = null; if (window.__viewer) window.__viewer.setMarker(null); $('placeMsg').textContent = PLACE_MSG[mode] || ''; document.body.style.cursor = mode ? 'crosshair' : ''; }
$('placeWall').onclick = () => setPlacing(placing === 'wall' ? null : 'wall');
$('placeCol').onclick = () => setPlacing(placing === 'column' ? null : 'column');
$('placeDoor').onclick = () => setPlacing(placing === 'door' ? null : 'door');
$('placeSpace').onclick = () => setPlacing(placing === 'space' ? null : 'space');

// Drag the selection in plan; deltas arrive in world metres. One undo step for the whole move.
function onViewMove(ids, dx, dz) {
  const before = JSON.stringify(project);
  if (!moveElements(project, ids, dx, dz, snap)) return;
  snapshot(before); rerun();
}
// Space tool: the dragged rectangle (already on the 0.5 m grid) becomes a space on the active level.
function onViewRect(a, b) {
  const bounds = rectBounds(a, b);
  if (!bounds) { $('placeMsg').textContent = 'too small — drag at least 0.5 m each way (Esc cancels)'; return; }
  snapshot();
  project.spaces.push({ name: `Space ${project.spaces.length + 1}`, occupancy: spaceOcc, floor_area_m2: bounds[2] * bounds[3], egress_doors: [], bounds, level: currentLevel()?.id ?? null });
  setPlacing(null); notice = ` · added ${project.spaces[project.spaces.length - 1].name}`; rerun();
}

// Exports built from the report the wasm just produced.
// RFC 4180 quoting; leading =,+,-,@ prefixed with ' so spreadsheets don't evaluate it (matches bim_takeoff::csv_field)
function csvField(v) { v = String(v); if (/^[=+\-@\t\r]/.test(v)) v = "'" + v; return /[",\r\n]/.test(v) ? '"' + v.replace(/"/g, '""') + '"' : v; }
function download(name, text, type) { const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([text], { type })); a.download = name; a.click(); }
$('exportCsv').onclick = () => {
  if (!lastReport) return;
  const rows = [['element','cost_code','phase','start_day','qty','unit','cost_usd'].join(',')]
    .concat(lastReport.elements.map(e => { const [q, u] = qtyOut(e.qty, e.unit, units); return [csvField(e.name), csvField(e.cost_code), e.phase, e.start_day, q.toFixed(3), u, e.cost_usd.toFixed(2)].join(','); }));
  download(`${project.name.replace(/\W+/g,'_')}_takeoff.csv`, rows.join('\n'), 'text/csv');
};
// binary exports straight from the engine (feature-detected at startup)
function engineExport(fn, name, type) {
  try { download(name, bim[fn](JSON.stringify(project)), type); } catch (e) { $('runErr').textContent = `${name}: ${e}`; }
}
$('exportXlsx').onclick = () => engineExport('takeoff_xlsx', 'takeoff.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet');
$('exportBcf').onclick = () => engineExport('clashes_bcf', 'clashes.bcfzip', 'application/octet-stream');
$('printReport').onclick = () => {
  if (!lastReport) return;
  const w = window.open('', '_blank');
  if (!w) { $('runErr').textContent = 'Allow pop-ups for this site to open the printable report.'; return; }
  w.document.write(printHtml(lastReport, units)); w.document.close();
};
$('exportHtml').onclick = async () => {
  if (!lastReport) return;
  const [shell, css, js, three, cites] = await Promise.all(['report_shell.html','viewer.css','viewer.js','three.r128.min.js','cites.js'].map(f => fetch(f).then(r => r.text())));
  const page = shell.replace('/*__CSS__*/', () => css).replace('/*__CITES__*/', () => cites).replace('/*__THREE__*/', () => three).replace('/*__VIEWER__*/', () => js)
    .replace('/*__DATA__*/', () => JSON.stringify(lastReport).replace(/</g, '\\u003c'));
  download(`${project.name.replace(/\W+/g,'_')}_report.html`, page, 'text/html');
};
$('undo').onclick = undo; $('redo').onclick = redo;
document.addEventListener('keydown', ev => {
  const typing = ['INPUT','TEXTAREA','SELECT'].includes(document.activeElement.tagName);
  if (ev.key === 'Escape') setPlacing(null);
  if (ev.key === 'Delete' && sel.length && !typing) deleteSelected();
  if ((ev.ctrlKey || ev.metaKey) && !typing) {
    const key = ev.key.toLowerCase();
    if (key === 'z' && !ev.shiftKey) { ev.preventDefault(); undo(); }
    if (key === 'y' || (key === 'z' && ev.shiftKey)) { ev.preventDefault(); redo(); }
    // only claim copy when elements are selected, so copying page text still works
    if (key === 'c' && sel.length) { ev.preventDefault(); copySel(); }
    if (key === 'v' && clip) { ev.preventDefault(); paste(); }
    if (key === 'd') { ev.preventDefault(); if (copySel()) paste(); }
  }
});
function newEl(name, kind, disc, code){ const ph = project.phases[project.phases.length-1]; return { id: uuid(), name, kind, discipline: disc, phase: ph ? ph.id : 0, start_offset_days: 0, lod: 'Lod200', cost_code: code, material: '', level: currentLevel()?.id ?? null }; }
function onViewClick({ id, point, ground, toggle }) {
  if (placing === 'door') {
    const w = id && project.elements.find(e => e.id === id && e.kind.Wall);
    if (!w || !point) { $('placeMsg').textContent = 'that is not a wall — click a wall (Esc cancels)'; return; }
    const [sx,,sz] = w.kind.Wall.start, [ex,,ez] = w.kind.Wall.end, len = Math.hypot(ex-sx, ez-sz) || 1;
    const along = ((point[0]-sx)*(ex-sx) + (point[2]-sz)*(ez-sz)) / len;
    const width = 0.9, offset = Math.max(0, Math.min(len - width, snap(along - width/2)));
    snapshot();
    const e = newEl(`Door ${project.elements.length+1}`, { Door: { host: w.id, offset, width, height: 2.1, is_egress: true } }, 'Architectural', 'C1020');
    project.elements.push(e); sel = [e.id]; setPlacing(null); rerun(); return;
  }
  if (placing === 'space') return; // the space tool drags; a plain click does nothing
  if (placing && ground) {
    const [x, z] = [snap(ground[0]), snap(ground[1])];
    if (placing === 'column') { snapshot(); const e = newEl(`Column ${project.elements.length+1}`, { Column: { base:[x,lvlY(),z], width:0.4, depth:0.4, height:3.2 } }, 'Structural', 'B1010'); project.elements.push(e); sel = [e.id]; rerun(); return; }
    if (!placeStart) { placeStart = [x, z]; window.__viewer.setMarker(placeStart); $('placeMsg').textContent = 'click end point (Esc cancels)'; return; }
    if (Math.hypot(x - placeStart[0], z - placeStart[1]) < step()) return;
    snapshot();
    const e = newEl(`Wall ${project.elements.length+1}`, { Wall: { start:[placeStart[0],lvlY(),placeStart[1]], end:[x,lvlY(),z], height:3.0, thickness:0.25 } }, 'Architectural', 'B2010');
    project.elements.push(e); sel = [e.id]; setPlacing(null); rerun(); return;
  }
  select(clickSelect(sel, id, toggle)); if (sel.length) selectTab('edit');
}


function renderLevels() {
  const ls = sortedLevels(), cur = currentLevel();
  const used = id => project.elements.filter(e => e.level === id).length + project.spaces.filter(s => s.level === id).length;
  $('levelEdit').innerHTML = (ls.length ? ls.map(l => `<div class="el" style="border:0;align-items:center">
      <label style="display:flex;gap:4px;align-items:center" title="Active level: new elements are placed here"><input type="radio" name="activeLevel" data-active="${l.id}" ${cur && l.id === cur.id ? 'checked' : ''}><input data-lv="${l.id}" data-f="name" value="${esc(l.name)}" style="width:110px"></label>
      <span>${lenInput(`data-lv="${l.id}" data-f="elevation_m" style="width:84px" title="Finished-floor elevation; changing it moves everything on this level"`, l.elevation_m)} <small style="color:var(--muted)" title="elements and spaces on this level">${used(l.id)}</small> <button class="btn" data-dellv="${l.id}" title="Delete level (its elements and spaces become unassigned)">✕</button></span></div>`).join('')
    : '<small style="color:var(--muted)">No levels: everything is on one floor.</small>') + '<div><button class="btn" id="addLevel">+ level</button></div>';
  $('levelEdit').querySelectorAll('[data-active]').forEach(r => r.onchange = () => { activeLevel = +r.dataset.active; rerun(); });
  $('levelEdit').querySelectorAll('[data-f]').forEach(inp => inp.onchange = () => {
    const l = project.levels.find(x => x.id === +inp.dataset.lv);
    if (inp.dataset.f === 'name') { snapshot(); l.name = inp.value; rerun(); return; }
    const v = parseLen(inp.value, units); if (Number.isNaN(v)) { badLen(inp.value); renderLevels(); return; }
    snapshot(); const n = setLevelElevation(project, l.id, v);
    if (n) notice = ` · moved ${n} element${n === 1 ? '' : 's'} with ${l.name}`;
    rerun();
  });
  $('levelEdit').querySelectorAll('[data-dellv]').forEach(b => b.onclick = () => {
    const id = +b.dataset.dellv; snapshot();
    project.levels = project.levels.filter(l => l.id !== id);
    for (const x of [...project.elements, ...project.spaces]) if (x.level === id) x.level = null;
    if (activeLevel === id) activeLevel = null;
    rerun();
  });
  $('addLevel').onclick = () => {
    snapshot(); project.levels = project.levels || [];
    const top = ls[ls.length - 1], id = project.levels.reduce((m, l) => Math.max(m, l.id), -1) + 1;
    project.levels.push({ id, name: `Level ${project.levels.length + 1}`, elevation_m: top ? top.elevation_m + 3.6576 : 0 }); // 12 ft floor-to-floor
    activeLevel = id; rerun();
  };
}

function renderEditor() {
  const walls = project.elements.filter(e => e.kind.Wall);
  const kindSel = $('kind'); if (!kindSel.options.length) { for (const k in KINDS) kindSel.add(new Option(k, k)); kindSel.onchange = renderFields; }
  renderFields();
  $('phaseEdit').innerHTML = project.phases.map((p, i) => {
    const deps = project.phases.filter(o => o.id !== p.id).map(o => `<label style="font-size:11px;margin-right:8px"><input type="checkbox" data-ph="${i}" data-dep="${o.id}" ${p.depends_on.includes(o.id) ? 'checked' : ''}> ${esc(o.name)}</label>`).join('');
    return `<div style="border-bottom:1px dashed var(--line);padding:4px 0"><div class="el" style="border:0"><input data-ph="${i}" data-f="name" value="${esc(p.name)}" style="width:45%"><span><input data-ph="${i}" data-f="duration_days" type="number" min="0" value="${p.duration_days}" style="width:56px"> d <button class="btn" data-delph="${i}">✕</button></span></div>
      <div style="color:var(--muted);font-size:11px">after: ${deps || 'nothing'}</div></div>`;
  }).join('') + `<button class="btn" id="addPhase">+ phase (after last)</button>`;
  $('phaseEdit').querySelectorAll('input[data-f]').forEach(inp => inp.onchange = () => { snapshot(); const p = project.phases[+inp.dataset.ph]; p[inp.dataset.f] = inp.dataset.f === 'name' ? inp.value : +inp.value; rerun(); });
  $('phaseEdit').querySelectorAll('input[data-dep]').forEach(cb => cb.onchange = () => { snapshot(); const p = project.phases[+cb.dataset.ph], d = +cb.dataset.dep; p.depends_on = cb.checked ? [...new Set([...p.depends_on, d])] : p.depends_on.filter(x => x !== d); rerun(); });
  $('phaseEdit').querySelectorAll('[data-delph]').forEach(b => b.onclick = () => { if (project.phases.length < 2) { $('runErr').textContent = 'A project needs at least one phase.'; return; } snapshot(); const id = project.phases[+b.dataset.delph].id; project.phases = project.phases.filter(p => p.id !== id); project.phases.forEach(p => p.depends_on = p.depends_on.filter(d => d !== id)); const first = project.phases[0]; project.elements.forEach(e => { if (e.phase === id && first) e.phase = first.id; }); rerun(); });
  $('addPhase').onclick = () => { snapshot(); const last = project.phases[project.phases.length-1]; const id = project.phases.reduce((m, p) => Math.max(m, p.id), -1) + 1; project.phases.push({ id, name: `Phase ${id+1}`, duration_days: 10, depends_on: last ? [last.id] : [] }); rerun(); };
  renderElementList();
  function renderFields() {
    const k = KINDS[kindSel.value]; const phases = project.phases.map(p => `<option value="${p.id}">${esc(p.name)}</option>`).join('');
    $('fields').innerHTML = `<label>Name<input name="name" value="${kindSel.value} ${project.elements.length+1}"></label><label>Phase<select name="phase">${phases}</select></label>` +
      Object.entries(k.fields).map(([n,[lbl,def]]) => def === 'wall' ? `<label>${lbl}<select name="${n}">${walls.map(w=>`<option value="${w.id}">${esc(w.name)}</option>`).join('')}</select></label>` :
        def === 'bool' ? `<label>${lbl}<select name="${n}"><option value="1">yes</option><option value="">no</option></select></label>` :
        isLen(n) ? `<label>${lbl}${lenInput(`name="${n}"`, def)}</label>` : `<label>${lbl}<input name="${n}" type="number" step="1" value="${def}"></label>`).join('');
  }
}

// element list: click selects, Shift/Ctrl/Cmd+click toggles
function renderElementList() {
  const on = new Set(sel);
  $('elements').innerHTML = project.elements.map((e, i) => `<div class="el" data-id="${esc(e.id)}" style="cursor:pointer${on.has(e.id)?';color:var(--accent);font-weight:600':''}"><span>${esc(e.name)} <small style="color:var(--muted)">${kindOf(e)} · ph${e.phase}</small></span><button data-i="${i}" title="delete">✕</button></div>`).join('');
  $('elements').querySelectorAll('.el').forEach(d => d.onclick = ev => { if (ev.target.tagName === 'BUTTON') return; select(clickSelect(sel, d.dataset.id, ev.shiftKey || ev.ctrlKey || ev.metaKey)); });
  $('elements').querySelectorAll('button').forEach(b => b.onclick = () => { snapshot(); const id = project.elements[+b.dataset.i].id; deleteElements(project, [id]); pruneSel(); rerun(); });
}

$('addForm').onsubmit = ev => {
  ev.preventDefault();
  const k = KINDS[$('kind').value], fd = new FormData(ev.target), f = {};
  for (const [n, v] of fd) f[n] = (n === 'host' || n === 'name' || n === 'phase') ? v : n === 'is_egress' ? !!v : n === 'riser_count' ? +v : parseLen(v, units);
  const bad = Object.entries(f).find(([n, v]) => typeof v === 'number' && Number.isNaN(v));
  if (bad) { badLen(fd.get(bad[0])); return; }
  if ($('kind').value === 'Door' && !f.host) { $('runErr').textContent = 'Add a wall first.'; return; }
  snapshot();
  project.elements.push({ id: uuid(), name: f.name, kind: k.build(f), discipline: k.disc, phase: +fd.get('phase'), start_offset_days: 0, lod: 'Lod200', cost_code: k.code, material: '', level: currentLevel()?.id ?? null });
  rerun();
};
$('apply').onclick = () => { try { const p = JSON.parse($('jsonBox').value); const v = JSON.parse(bim.validate(JSON.stringify(p))); $('jsonMsg').textContent = v.join('\n'); if (!v.length) { snapshot(); project = p; pruneSel(); rerun(); } } catch (e) { $('jsonMsg').textContent = String(e); } };
$('jsonBox').oninput = () => { try { $('jsonMsg').textContent = JSON.parse(bim.validate($('jsonBox').value)).join('\n'); } catch (e) { $('jsonMsg').textContent = String(e); } };
$('save').onclick = () => { const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([JSON.stringify(project, null, 2)], { type: 'application/json' })); a.download = `${project.name.replace(/\W+/g,'_')}.json`; a.click(); };
$('open').onclick = () => $('file').click();
// Open a project (.json) or import a model (.ifc: IFC2x3/IFC4, converted by the Rust importer)
$('file').onchange = async ev => {
  const f = ev.target.files[0]; ev.target.value = ''; if (!f) return;
  try {
    const text = await f.text();
    if (/\.ifc$/i.test(f.name) || text.trimStart().startsWith('ISO-10303-21')) {
      status(`importing ${f.name}…`);
      const r = JSON.parse(bim.import_ifc(text, f.name.replace(/\.[^.]+$/, '')));
      snapshot(); project = r.project; sel = []; setCloudLink(null);
      showImportNotes(f.name, r.notes);
      rerun(false);
      return;
    }
    const p = JSON.parse(text); const v = JSON.parse(bim.validate(JSON.stringify(p)));
    if (v.length) throw new Error(v.join('\n'));
    snapshot(); project = p; sel = []; setCloudLink(null); showImportNotes(); rerun(false);
  } catch (e) { $('runErr').textContent = String(e); }
};
function showImportNotes(name, notes) {
  const box = $('importNotes');
  box.hidden = !notes;
  if (!notes) return;
  box.innerHTML = `<div style="display:flex;justify-content:space-between;gap:8px"><b>Imported ${esc(name)}</b><button class="btn" id="closeNotes" title="Dismiss">✕</button></div><ul style="margin:4px 0 0;padding-left:18px">${notes.map(n => `<li>${esc(n)}</li>`).join('')}</ul>`;
  $('closeNotes').onclick = () => { box.hidden = true; };
}
$('new').onclick = () => { const name = prompt('Project name', 'New project'); if (!name) return; snapshot(); project = JSON.parse(bim.new_project(name, project?.rule_set || 'IN-2026')); sel = []; setCloudLink(null); showImportNotes(); rerun(false); };
document.querySelectorAll('.tabs .btn').forEach(b => b.onclick = () => selectTab(b.dataset.tab));

// ---- cloud projects (only when an API answers /api/health; static hosting never shows this) ----
const cloud = createCloud();
let cloudOn = false, cloudUser = null, cloudList = null, cloudShares = null, cloudConflict = false, cloudMsg = '';
const CLOUD_LINK_KEY = 'buildsim.cloud.link';
// the cloud project the open model was loaded from / saved to: {id, name, role, updated_at}
let cloudLink = (() => { try { return JSON.parse(store.get(CLOUD_LINK_KEY)) || null; } catch (_) { return null; } })();
function setCloudLink(l) { cloudLink = l; cloudConflict = false; cloudShares = null; store.set(CLOUD_LINK_KEY, JSON.stringify(l)); }
async function initCloud() {
  if (!(await cloud.health())) return; // no API: keep the tab hidden
  cloudOn = true; document.querySelector('.tabs [data-tab="cloud"]').hidden = false;
  if (cloud.signedIn) { try { const u = await cloud.me(); cloudUser = u.user || u; await refreshCloud(); } catch (_) { cloudUser = null; } }
  renderCloud();
}
async function cloudDo(fn, ok) {
  cloudMsg = '';
  try { await fn(); if (ok && !cloudConflict) cloudMsg = ok; }
  catch (e) { if (e.status === 401) cloudUser = null; cloudMsg = String(e.message || e); }
  renderCloud();
}
async function refreshCloud() {
  cloudList = await cloud.list();
  cloudShares = cloudLink && cloudLink.role === 'owner' ? await cloud.shares(cloudLink.id).catch(() => null) : null;
}
function openCloudData(r) {
  const v = JSON.parse(bim.validate(JSON.stringify(r.data)));
  if (v.length) throw new Error(`cloud project does not validate:\n${v.join('\n')}`);
  snapshot(); project = r.data; sel = []; showImportNotes();
  setCloudLink({ id: r.id, name: r.name, role: r.role, updated_at: r.updated_at });
  rerun(false);
}
async function cloudSave(force) {
  cloudConflict = false;
  const r = await cloud.save(cloudLink.id, { name: project.name, data: project, updated_at: force ? undefined : cloudLink.updated_at })
    .catch(e => { if (e.conflict) { cloudConflict = true; return null; } throw e; });
  if (!r) { cloudMsg = ''; return; }
  setCloudLink({ ...cloudLink, name: r.name, role: r.role || cloudLink.role, updated_at: r.updated_at });
  await refreshCloud();
}
function renderCloud() {
  const box = $('cloudPane'); if (!box || !cloudOn) return;
  const msg = cloudMsg ? `<div class="err">${esc(cloudMsg)}</div>` : '';
  if (!cloudUser) {
    box.innerHTML = `<h2>Cloud projects</h2><form class="add" id="cloudAuth"><label class="full">Email<input name="email" type="email" autocomplete="username" required></label>
      <label class="full">Password<input name="password" type="password" autocomplete="current-password" required></label>
      <button class="btn" type="submit" data-act="login">Sign in</button><button class="btn" type="submit" data-act="register">Create account</button></form>${msg}`;
    let act = 'login'; box.querySelectorAll('[data-act]').forEach(b => b.onclick = () => { act = b.dataset.act; });
    $('cloudAuth').onsubmit = ev => { ev.preventDefault(); const f = new FormData(ev.target);
      cloudDo(async () => { cloudUser = await cloud[act](f.get('email'), f.get('password')); await refreshCloud(); }); };
    return;
  }
  const L = cloudLink, canWrite = L && L.role !== 'viewer', when = t => t ? new Date(t).toLocaleString() : '';
  box.innerHTML = `<h2>Cloud projects</h2><div class="row"><span>Signed in as ${esc(cloudUser.email || '')}</span><button class="btn" id="cloudOut">Sign out</button></div>
    <h2>This model</h2>${L ? `<div style="font-size:13px">Linked to <b>${esc(L.name)}</b> <small style="color:var(--muted)">${esc(L.role)} · saved ${esc(when(L.updated_at))}</small></div>` : '<div style="font-size:13px;color:var(--muted)">Not saved to the cloud yet.</div>'}
    ${cloudConflict ? `<div class="adv error">“${esc(L.name)}” was changed in the cloud since you opened or last saved it.<div style="display:flex;gap:6px;margin-top:6px"><button class="btn" id="cloudReload">Reload cloud version</button><button class="btn" id="cloudOverwrite">Overwrite with mine</button></div></div>` : ''}
    <div style="display:flex;flex-wrap:wrap;gap:6px;margin:6px 0">${L ? `<button class="btn" id="cloudSave" ${canWrite ? '' : 'disabled title="view-only: use Save as new"'}>Save</button>` : ''}<button class="btn" id="cloudSaveAs">Save as new…</button>${L ? '<button class="btn" id="cloudUnlink">Unlink</button>' : ''}</div>
    ${L && L.role === 'owner' ? `<h2>Share</h2><form class="add" id="cloudShare"><label>Email<input name="email" type="email" required></label><label>Role<select name="role"><option value="editor">editor</option><option value="viewer">viewer</option></select></label><button class="btn full" type="submit">Share</button></form>
      ${(cloudShares || []).map(s => `<div class="el"><span>${esc(s.email)} <small style="color:var(--muted)">${esc(s.role)}</small></span><button data-unshare="${esc(s.email)}" title="Remove access">✕</button></div>`).join('')}` : ''}
    <h2>Your projects <button class="btn" id="cloudRefresh" style="padding:0 6px">↻</button></h2>
    ${(cloudList || []).map(p => `<div class="el"><span>${esc(p.name)} <small style="color:var(--muted)">${esc(p.role)} · ${esc(when(p.updated_at))}</small></span><span><button class="btn" data-open="${esc(p.id)}" style="color:var(--ink)">Open</button>${p.role === 'owner' ? ` <button data-del="${esc(p.id)}" title="Delete from the cloud">✕</button>` : ''}</span></div>`).join('') || '<small style="color:var(--muted)">No cloud projects yet.</small>'}${msg}`;
  $('cloudOut').onclick = () => cloudDo(async () => { await cloud.logout(); cloudUser = null; cloudList = null; });
  if ($('cloudSave')) $('cloudSave').onclick = () => cloudDo(() => cloudSave(false), 'saved');
  if ($('cloudOverwrite')) $('cloudOverwrite').onclick = () => cloudDo(() => cloudSave(true), 'saved (overwrote the cloud version)');
  if ($('cloudReload')) $('cloudReload').onclick = () => cloudDo(async () => openCloudData(await cloud.get(L.id)), 'reloaded the cloud version');
  if ($('cloudUnlink')) $('cloudUnlink').onclick = () => { setCloudLink(null); renderCloud(); };
  $('cloudSaveAs').onclick = () => { const name = prompt('Cloud project name', project.name); if (!name) return;
    cloudDo(async () => { const r = await cloud.create(name, project); setCloudLink(r); await refreshCloud(); }, 'saved'); };
  $('cloudRefresh').onclick = () => cloudDo(refreshCloud);
  box.querySelectorAll('[data-open]').forEach(b => b.onclick = () => cloudDo(async () => openCloudData(await cloud.get(b.dataset.open))));
  box.querySelectorAll('[data-del]').forEach(b => b.onclick = () => { const p = cloudList.find(x => x.id === b.dataset.del);
    if (!confirm(`Delete “${p.name}” from the cloud for everyone it is shared with?`)) return;
    cloudDo(async () => { await cloud.remove(p.id); if (cloudLink && cloudLink.id === p.id) setCloudLink(null); await refreshCloud(); }); });
  if ($('cloudShare')) $('cloudShare').onsubmit = ev => { ev.preventDefault(); const f = new FormData(ev.target);
    cloudDo(async () => { await cloud.share(L.id, f.get('email'), f.get('role')); await refreshCloud(); }, `shared with ${f.get('email')}`); };
  box.querySelectorAll('[data-unshare]').forEach(b => b.onclick = () => cloudDo(async () => { await cloud.unshare(L.id, b.dataset.unshare); await refreshCloud(); }));
}


(async () => {
  try {
    await init();
    const saved = restoreAutosave();
    project = saved ? saved.project : JSON.parse(bim.sample_project());
    if (saved) notice = ` · restored autosave from ${new Date(saved.saved_at).toLocaleString()}`;
    $('units').value = units;
    $('units').onchange = () => { units = $('units').value; store.set('buildsim.units', units); rerun(); };
    // engine exports added in newer builds: show their buttons only when this wasm has them
    $('exportXlsx').hidden = typeof bim.takeoff_xlsx !== 'function';
    $('exportBcf').hidden = typeof bim.clashes_bcf !== 'function';
    rerun(false);
    loadRuleIndex(); initCloud();
  } catch (e) { $('runErr').textContent = 'Failed to start: ' + e + '\n(Build the wasm first — see web/README.md.)'; }
})();
