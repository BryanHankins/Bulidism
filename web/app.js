// Live editor: JS owns the project JSON; every edit re-runs the Rust pipeline in wasm.
import init, * as bim from './pkg/bim_wasm.js';

const $ = id => document.getElementById(id);
let project = null, rules = null, lastDay = 0, selectedId = null, placing = null, placeStart = null, lastReport = null;
const undoStack = [], redoStack = [];
function snapshot() { if (project) { undoStack.push(JSON.stringify(project)); if (undoStack.length > 50) undoStack.shift(); redoStack.length = 0; updateUndo(); } }
function updateUndo() { $('undo').disabled = !undoStack.length; $('redo').disabled = !redoStack.length; }
function undo() { if (!undoStack.length) return; redoStack.push(JSON.stringify(project)); project = JSON.parse(undoStack.pop()); if (!project.elements.some(e => e.id === selectedId)) selectedId = null; updateUndo(); rerun(); }
function redo() { if (!redoStack.length) return; undoStack.push(JSON.stringify(project)); project = JSON.parse(redoStack.pop()); updateUndo(); rerun(); }
const SNAP = 0.5;
const snap = v => Math.round(v / SNAP) * SNAP;

const KINDS = {
  Wall:   { fields: { sx:['start x',0], sz:['start z',0], ex:['end x',6], ez:['end z',0], height:['height',3], thickness:['thickness',0.25] },
            build: f => ({ Wall: { start:[f.sx,0.2,f.sz], end:[f.ex,0.2,f.ez], height:f.height, thickness:f.thickness } }), disc:'Architectural', code:'B2010' },
  Column: { fields: { x:['x',0], z:['z',0], width:['width',0.4], depth:['depth',0.4], height:['height',3.2] },
            build: f => ({ Column: { base:[f.x,0.2,f.z], width:f.width, depth:f.depth, height:f.height } }), disc:'Structural', code:'B1010' },
  Slab:   { fields: { x:['origin x',0], z:['origin z',0], y:['elevation',0], size_x:['size x',12], size_z:['size z',8], thickness:['thickness',0.2] },
            build: f => ({ Slab: { origin:[f.x,f.y,f.z], size_x:f.size_x, size_z:f.size_z, thickness:f.thickness } }), disc:'Structural', code:'A1010' },
  Beam:   { fields: { sx:['start x',0], sy:['start y',3.4], sz:['start z',0], ex:['end x',12], ez:['end z',0], width:['width',0.3], depth:['depth',0.5] },
            build: f => ({ Beam: { start:[f.sx,f.sy,f.sz], end:[f.ex,f.sy,f.ez], width:f.width, depth:f.depth } }), disc:'Structural', code:'B1010' },
  Door:   { fields: { host:['host wall','wall'], offset:['offset',1], width:['width',0.9], height:['height',2.1], is_egress:['egress','bool'] },
            build: f => ({ Door: { host:f.host, offset:f.offset, width:f.width, height:f.height, is_egress:!!f.is_egress } }), disc:'Architectural', code:'C1020' },
  Stair:  { fields: { x:['x',1], z:['z',3], run_length:['run',3.5], total_rise:['rise',3], width:['width',1.2], riser_count:['risers',17] },
            build: f => ({ Stair: { base:[f.x,0.2,f.z], run_length:f.run_length, total_rise:f.total_rise, width:f.width, riser_count:Math.round(f.riser_count) } }), disc:'Architectural', code:'C2010' },
  Ramp:   { fields: { x:['x',6], z:['z',-3], run_length:['run',2.4], rise:['rise',0.2], width:['width',1.2] },
            build: f => ({ Ramp: { base:[f.x,0,f.z], run_length:f.run_length, rise:f.rise, width:f.width } }), disc:'Site', code:'G2030' },
};

function uuid(){ return crypto.randomUUID ? crypto.randomUUID() : 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g,c=>{const r=Math.random()*16|0;return (c==='x'?r:(r&3|8)).toString(16)}); }
function status(t){ $('status').textContent = t; }

function rerun(keepDay = true) {
  $('runErr').textContent = '';
  try {
    const t0 = performance.now();
    const report = JSON.parse(bim.run(JSON.stringify(project), rules));
    lastReport = report;
    $('spaceList').innerHTML = report.spaces.length
      ? report.spaces.map(s => `<div class="row"><span>${s.name} <small style="color:var(--muted)">${s.occupancy}</small></span><span>${s.area_m2.toFixed(1)} m² · ${s.exits} exit${s.exits===1?'':'s'}</span></div>`).join('')
      : '<div class="row"><span>No spaces defined</span></div>';
    renderReport(report, { day: keepDay ? Math.min(lastDay, report.total_days) : 0, selected: selectedId, onClick: onViewClick, onMove: onViewMove, onHandle: onViewHandle, handlesFor, onDay: d => lastDay = d });
    status(`ran in ${(performance.now()-t0).toFixed(1)} ms · schema v${bim.schema_version()}`);
  } catch (e) {
    $('runErr').textContent = String(e);
  }
  renderEditor(); renderProps(); renderSpaces(); $('jsonBox').value = JSON.stringify(project, null, 2);
}


// ---- selection + property editing ----
const NUMERIC = { Wall:['height','thickness'], Slab:['size_x','size_z','thickness'], Column:['width','depth','height'], Beam:['width','depth'], Door:['offset','width','height'], Stair:['run_length','total_rise','width','riser_count'], Ramp:['run_length','rise','width'], Mesh:[] };
const VEC = { Wall:['start','end'], Slab:['origin'], Column:['base'], Beam:['start','end'], Stair:['base'], Ramp:['base'], Mesh:['origin'], Door:[] };
function selectedEl(){ return project.elements.find(e => e.id === selectedId) || null; }
function renderProps() {
  const e = selectedEl(); $('selPanel').hidden = !e; if (!e) return;
  const kind = Object.keys(e.kind)[0], k = e.kind[kind];
  const phases = project.phases.map(p => `<option value="${p.id}" ${p.id===e.phase?'selected':''}>${p.name}</option>`).join('');
  let h = `<label class="full">Name<input name="name" value="${e.name}"></label><label>Phase<select name="phase">${phases}</select></label><label>Offset days<input name="start_offset_days" type="number" min="0" value="${e.start_offset_days||0}"></label><label>Cost code<input name="cost_code" value="${e.cost_code}"></label><label>Discipline<select name="discipline">${['Architectural','Structural','Mechanical','Electrical','Plumbing','Site'].map(d=>`<option ${d===e.discipline?'selected':''}>${d}</option>`).join('')}</select></label>`;
  for (const v of VEC[kind]) h += ['x','y','z'].map((ax,i) => `<label>${v} ${ax}<input name="${v}.${i}" type="number" step="0.05" value="${k[v][i]}"></label>`).join('');
  for (const n of NUMERIC[kind]) h += `<label>${n}<input name="${n}" type="number" step="${n==='riser_count'?1:0.05}" value="${k[n]}"></label>`;
  if (kind === 'Door') h += `<label>egress<select name="is_egress"><option value="1" ${k.is_egress?'selected':''}>yes</option><option value="" ${!k.is_egress?'selected':''}>no</option></select></label>`;
  $('propForm').innerHTML = h;
  $('propForm').querySelectorAll('input,select').forEach(inp => inp.onchange = () => {
    snapshot();
    const n = inp.name, val = inp.value;
    if (n === 'name' || n === 'cost_code' || n === 'discipline') e[n] = val;
    else if (n === 'phase' || n === 'start_offset_days') e[n] = +val;
    else if (n === 'is_egress') k.is_egress = !!val;
    else if (n.includes('.')) { const [v,i] = n.split('.'); k[v][+i] = +val; }
    else k[n] = n === 'riser_count' ? Math.max(1, Math.round(+val)) : +val;
    rerun();
  });
}
$('delSel').onclick = () => { const id = selectedId; if (!id) return; snapshot(); project.elements = project.elements.filter(e => e.id !== id && !(e.kind.Door && e.kind.Door.host === id)); project.spaces.forEach(s => s.egress_doors = s.egress_doors.filter(d => project.elements.some(e => e.id === d))); selectedId = null; rerun(); };
$('deselect').onclick = () => { selectedId = null; window.__viewer.highlight(null); renderProps(); };
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
function renderSpaces() {
  const doors = project.elements.filter(e => e.kind.Door);
  $('spaceEdit').innerHTML = project.spaces.map((s, i) => {
    const opts = OCCUPANCIES.map(o => `<option ${o === s.occupancy ? 'selected' : ''}>${o}</option>`).join('');
    const exits = doors.map(d => `<label style="display:block;font-size:12px"><input type="checkbox" data-sp="${i}" data-door="${d.id}" ${s.egress_doors.includes(d.id) ? 'checked' : ''}> ${d.name}${d.kind.Door.is_egress ? '' : ' (not egress)'}</label>`).join('') || '<small style="color:var(--muted)">no doors yet</small>';
    const b = s.bounds;
    return `<div style="border:1px solid var(--line);border-radius:6px;padding:8px;margin-bottom:8px">
      <div style="display:flex;gap:6px"><input data-sp="${i}" data-f="name" value="${s.name}" style="flex:1"><button data-del="${i}" class="btn">✕</button></div>
      <label class="chk">Occupancy<select data-sp="${i}" data-f="occupancy">${opts}</select></label>
      <label class="chk" style="font-size:12px"><input type="checkbox" data-sp="${i}" data-f="useBounds" ${b ? 'checked' : ''}> area from plan rectangle</label>
      ${b ? `<div style="display:grid;grid-template-columns:1fr 1fr;gap:4px">${['x','z','size x','size z'].map((l, j) => `<label style="font-size:11px;color:var(--muted)">${l}<input type="number" step="0.5" data-sp="${i}" data-b="${j}" value="${b[j]}" style="width:100%"></label>`).join('')}</div>`
          : `<label class="chk" style="font-size:12px">Area m²<input type="number" step="1" data-sp="${i}" data-f="floor_area_m2" value="${s.floor_area_m2}" style="width:80px"></label>`}
      <div style="margin-top:6px;font-size:11px;color:var(--muted)">Egress doors</div>${exits}</div>`;
  }).join('') + '<button class="btn" id="addSpace">+ space</button>';
  $('spaceEdit').querySelectorAll('[data-f],[data-b]').forEach(inp => inp.onchange = () => {
    snapshot(); const sp = project.spaces[+inp.dataset.sp];
    if (inp.dataset.b !== undefined) { sp.bounds[+inp.dataset.b] = +inp.value; sp.floor_area_m2 = sp.bounds[2] * sp.bounds[3]; }
    else if (inp.dataset.f === 'useBounds') { sp.bounds = inp.checked ? [0, 0, Math.sqrt(sp.floor_area_m2) || 4, Math.sqrt(sp.floor_area_m2) || 4] : null; if (sp.bounds) sp.floor_area_m2 = sp.bounds[2] * sp.bounds[3]; }
    else if (inp.dataset.f === 'floor_area_m2') sp.floor_area_m2 = +inp.value;
    else sp[inp.dataset.f] = inp.value;
    rerun();
  });
  $('spaceEdit').querySelectorAll('[data-door]').forEach(cb => cb.onchange = () => {
    snapshot(); const sp = project.spaces[+cb.dataset.sp], id = cb.dataset.door;
    sp.egress_doors = cb.checked ? [...new Set([...sp.egress_doors, id])] : sp.egress_doors.filter(d => d !== id);
    rerun();
  });
  $('spaceEdit').querySelectorAll('[data-del]').forEach(b => b.onclick = () => { snapshot(); project.spaces.splice(+b.dataset.del, 1); rerun(); });
  $('addSpace').onclick = () => { snapshot(); project.spaces.push({ name: `Space ${project.spaces.length+1}`, occupancy: 'Business', floor_area_m2: 50, egress_doors: [], bounds: null }); rerun(); };
}
$('showSpaces').onchange = e => window.__viewer.setSpacesVisible(e.target.checked);

// ---- click-to-place ----
function setPlacing(mode){ placing = mode; placeStart = null; window.__viewer.setMarker(null); $('placeMsg').textContent = mode === 'wall' ? 'click start point' : mode === 'column' ? 'click position' : mode === 'door' ? 'click a wall' : ''; document.body.style.cursor = mode ? 'crosshair' : ''; }
$('placeWall').onclick = () => setPlacing(placing === 'wall' ? null : 'wall');
$('placeCol').onclick = () => setPlacing(placing === 'column' ? null : 'column');
$('placeDoor').onclick = () => setPlacing(placing === 'door' ? null : 'door');

// Drag a selected element in plan; deltas arrive in world metres.
function onViewMove(id, dx, dz) {
  const e = project.elements.find(x => x.id === id); if (!e) return;
  const kind = Object.keys(e.kind)[0], k = e.kind[kind];
  if (kind === 'Door') { const w = project.elements.find(x => x.id === k.host); if (!w) return;
    const [sx,,sz] = w.kind.Wall.start, [ex,,ez] = w.kind.Wall.end, len = Math.hypot(ex-sx, ez-sz) || 1;
    const along = ((dx*(ex-sx)) + (dz*(ez-sz))) / len;
    k.offset = Math.max(0, Math.min(len - k.width, snap(k.offset + along)));
  } else for (const v of VEC[kind]) { k[v][0] = snap(k[v][0] + dx); k[v][2] = snap(k[v][2] + dz); }
  snapshot(); rerun();
}

// Exports built from the report the wasm just produced.
function download(name, text, type) { const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([text], { type })); a.download = name; a.click(); }
$('exportCsv').onclick = () => {
  if (!lastReport) return;
  const rows = [['element','cost_code','phase','start_day','qty','unit','cost_usd'].join(',')]
    .concat(lastReport.elements.map(e => [e.name.replace(/,/g,' '), e.cost_code, e.phase, e.start_day, e.qty.toFixed(3), e.unit, e.cost_usd.toFixed(2)].join(',')));
  download(`${project.name.replace(/\W+/g,'_')}_takeoff.csv`, rows.join('\n'), 'text/csv');
};
$('exportHtml').onclick = async () => {
  if (!lastReport) return;
  const [shell, css, js, three] = await Promise.all(['report_shell.html','viewer.css','viewer.js','three.r128.min.js'].map(f => fetch(f).then(r => r.text())));
  const page = shell.replace('/*__CSS__*/', css).replace('/*__THREE__*/', three).replace('/*__VIEWER__*/', js)
    .replace('/*__DATA__*/', JSON.stringify(lastReport).replace(/<\//g, '<\\/'));
  download(`${project.name.replace(/\W+/g,'_')}_report.html`, page, 'text/html');
};
$('undo').onclick = undo; $('redo').onclick = redo;
document.addEventListener('keydown', ev => {
  const typing = ['INPUT','TEXTAREA','SELECT'].includes(document.activeElement.tagName);
  if (ev.key === 'Escape') setPlacing(null);
  if (ev.key === 'Delete' && selectedId && !typing) $('delSel').onclick();
  if ((ev.ctrlKey || ev.metaKey) && !typing) {
    if (ev.key === 'z' && !ev.shiftKey) { ev.preventDefault(); undo(); }
    if (ev.key === 'y' || (ev.key === 'z' && ev.shiftKey)) { ev.preventDefault(); redo(); }
  }
});
function newEl(name, kind, disc, code){ const ph = project.phases[project.phases.length-1]; return { id: uuid(), name, kind, discipline: disc, phase: ph ? ph.id : 0, start_offset_days: 0, lod: 'Lod200', cost_code: code, material: '' }; }
function onViewClick({ id, point, ground }) {
  if (placing === 'door') {
    const w = id && project.elements.find(e => e.id === id && e.kind.Wall);
    if (!w || !point) { $('placeMsg').textContent = 'that is not a wall — click a wall (Esc cancels)'; return; }
    const [sx,,sz] = w.kind.Wall.start, [ex,,ez] = w.kind.Wall.end, len = Math.hypot(ex-sx, ez-sz) || 1;
    const along = ((point[0]-sx)*(ex-sx) + (point[2]-sz)*(ez-sz)) / len;
    const width = 0.9, offset = Math.max(0, Math.min(len - width, snap(along - width/2)));
    snapshot();
    const e = newEl(`Door ${project.elements.length+1}`, { Door: { host: w.id, offset, width, height: 2.1, is_egress: true } }, 'Architectural', 'C1020');
    project.elements.push(e); selectedId = e.id; setPlacing(null); rerun(); return;
  }
  if (placing && ground) {
    snapshot();
    const [x, z] = [snap(ground[0]), snap(ground[1])];
    if (placing === 'column') { const e = newEl(`Column ${project.elements.length+1}`, { Column: { base:[x,0.2,z], width:0.4, depth:0.4, height:3.2 } }, 'Structural', 'B1010'); project.elements.push(e); selectedId = e.id; rerun(); return; }
    if (!placeStart) { placeStart = [x, z]; window.__viewer.setMarker(placeStart); $('placeMsg').textContent = 'click end point (Esc cancels)'; return; }
    if (Math.hypot(x - placeStart[0], z - placeStart[1]) < SNAP) return;
    const e = newEl(`Wall ${project.elements.length+1}`, { Wall: { start:[placeStart[0],0.2,placeStart[1]], end:[x,0.2,z], height:3.0, thickness:0.25 } }, 'Architectural', 'B2010');
    project.elements.push(e); selectedId = e.id; setPlacing(null); rerun(); return;
  }
  selectedId = id; window.__viewer.highlight(id); renderProps(); if (id) selectTab('edit');
}


function renderEditor() {
  const walls = project.elements.filter(e => e.kind.Wall);
  const kindSel = $('kind'); if (!kindSel.options.length) { for (const k in KINDS) kindSel.add(new Option(k, k)); kindSel.onchange = renderFields; }
  renderFields();
  $('phaseEdit').innerHTML = project.phases.map((p, i) => {
    const deps = project.phases.filter(o => o.id !== p.id).map(o => `<label style="font-size:11px;margin-right:8px"><input type="checkbox" data-ph="${i}" data-dep="${o.id}" ${p.depends_on.includes(o.id) ? 'checked' : ''}> ${o.name}</label>`).join('');
    return `<div style="border-bottom:1px dashed var(--line);padding:4px 0"><div class="el" style="border:0"><input data-ph="${i}" data-f="name" value="${p.name}" style="width:45%"><span><input data-ph="${i}" data-f="duration_days" type="number" min="0" value="${p.duration_days}" style="width:56px"> d <button class="btn" data-delph="${i}">✕</button></span></div>
      <div style="color:var(--muted);font-size:11px">after: ${deps || 'nothing'}</div></div>`;
  }).join('') + `<button class="btn" id="addPhase">+ phase (after last)</button>`;
  $('phaseEdit').querySelectorAll('input[data-f]').forEach(inp => inp.onchange = () => { snapshot(); const p = project.phases[+inp.dataset.ph]; p[inp.dataset.f] = inp.dataset.f === 'name' ? inp.value : +inp.value; rerun(); });
  $('phaseEdit').querySelectorAll('input[data-dep]').forEach(cb => cb.onchange = () => { snapshot(); const p = project.phases[+cb.dataset.ph], d = +cb.dataset.dep; p.depends_on = cb.checked ? [...new Set([...p.depends_on, d])] : p.depends_on.filter(x => x !== d); rerun(); });
  $('phaseEdit').querySelectorAll('[data-delph]').forEach(b => b.onclick = () => { snapshot(); const id = project.phases[+b.dataset.delph].id; project.phases = project.phases.filter(p => p.id !== id); project.phases.forEach(p => p.depends_on = p.depends_on.filter(d => d !== id)); const first = project.phases[0]; project.elements.forEach(e => { if (e.phase === id && first) e.phase = first.id; }); rerun(); });
  $('addPhase').onclick = () => { snapshot(); const last = project.phases[project.phases.length-1]; const id = (last ? last.id : -1) + 1; project.phases.push({ id, name: `Phase ${id+1}`, duration_days: 10, depends_on: last ? [last.id] : [] }); rerun(); };
  $('elements').innerHTML = project.elements.map((e, i) => `<div class="el" data-id="${e.id}" style="cursor:pointer${e.id===selectedId?';color:var(--accent)':''}"><span>${e.name} <small style="color:var(--muted)">${Object.keys(e.kind)[0]} · ph${e.phase}</small></span><button data-i="${i}" title="delete">✕</button></div>`).join('');
  $('elements').querySelectorAll('.el').forEach(d => d.onclick = ev => { if (ev.target.tagName === 'BUTTON') return; selectedId = d.dataset.id; window.__viewer.highlight(selectedId); renderProps(); renderEditor(); });
  $('elements').querySelectorAll('button').forEach(b => b.onclick = () => { snapshot(); const id = project.elements[+b.dataset.i].id; project.elements = project.elements.filter(e => e.id !== id && !(e.kind.Door && e.kind.Door.host === id)); project.spaces.forEach(s => s.egress_doors = s.egress_doors.filter(d => project.elements.some(e => e.id === d))); rerun(); });
  function renderFields() {
    const k = KINDS[kindSel.value]; const phases = project.phases.map(p => `<option value="${p.id}">${p.name}</option>`).join('');
    $('fields').innerHTML = `<label>Name<input name="name" value="${kindSel.value} ${project.elements.length+1}"></label><label>Phase<select name="phase">${phases}</select></label>` +
      Object.entries(k.fields).map(([n,[lbl,def]]) => def === 'wall' ? `<label>${lbl}<select name="${n}">${walls.map(w=>`<option value="${w.id}">${w.name}</option>`).join('')}</select></label>` :
        def === 'bool' ? `<label>${lbl}<select name="${n}"><option value="1">yes</option><option value="">no</option></select></label>` :
        `<label>${lbl}<input name="${n}" type="number" step="0.05" value="${def}"></label>`).join('');
  }
}

$('addForm').onsubmit = ev => {
  ev.preventDefault();
  const k = KINDS[$('kind').value], fd = new FormData(ev.target), f = {};
  snapshot();
  for (const [n, v] of fd) f[n] = (n === 'host' || n === 'name') ? v : (n === 'is_egress' ? !!v : +v);
  if ($('kind').value === 'Door' && !f.host) { $('runErr').textContent = 'Add a wall first.'; return; }
  project.elements.push({ id: uuid(), name: f.name, kind: k.build(f), discipline: k.disc, phase: +fd.get('phase'), start_offset_days: 0, lod: 'Lod200', cost_code: k.code, material: '' });
  rerun();
};
$('apply').onclick = () => { try { const p = JSON.parse($('jsonBox').value); snapshot(); const v = JSON.parse(bim.validate(JSON.stringify(p))); $('jsonMsg').textContent = v.join('\n'); if (!v.length) { project = p; rerun(); } } catch (e) { $('jsonMsg').textContent = String(e); } };
$('jsonBox').oninput = () => { try { $('jsonMsg').textContent = JSON.parse(bim.validate($('jsonBox').value)).join('\n'); } catch (e) { $('jsonMsg').textContent = String(e); } };
$('save').onclick = () => { const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([JSON.stringify(project, null, 2)], { type: 'application/json' })); a.download = `${project.name.replace(/\W+/g,'_')}.json`; a.click(); };
$('open').onclick = () => $('file').click();
$('file').onchange = async ev => { const f = ev.target.files[0]; if (!f) return; try { const p = JSON.parse(await f.text()); const v = JSON.parse(bim.validate(JSON.stringify(p))); if (v.length) throw new Error(v.join('\n')); project = p; rerun(false); } catch (e) { $('runErr').textContent = String(e); } ev.target.value = ''; };
$('new').onclick = () => { const name = prompt('Project name', 'New project'); if (!name) return; snapshot(); project = JSON.parse(bim.new_project(name, 'IN-2026')); rerun(false); };
document.querySelectorAll('.tabs .btn').forEach(b => b.onclick = () => selectTab(b.dataset.tab));


(async () => {
  try {
    await init();
    rules = await (await fetch('rules/IN-2026.json')).text();
    project = JSON.parse(bim.sample_project());
    rerun(false);
  } catch (e) { $('runErr').textContent = 'Failed to start: ' + e + '\n(Build the wasm first — see web/README.md.)'; }
})();
