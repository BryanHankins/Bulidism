// Multi-element edits (delete, move, copy/paste, element dependencies, drawn spaces) as pure functions
// of the project JSON, so the editor's rules are testable without a browser.

// vector fields per element kind; [x, y, z] with y up. Doors have none (they sit on their host wall).
const VECS = { Wall: ['start', 'end'], Slab: ['origin'], Column: ['base'], Beam: ['start', 'end'], Stair: ['base'], Ramp: ['base'], Mesh: ['origin'], Door: [] };
export const kindOf = e => Object.keys(e.kind)[0];
const clone = v => JSON.parse(JSON.stringify(v));

/** Selection after a click: toggle `id` in or out when a modifier is held, else select just it. */
export function clickSelect(sel, id, toggle) {
  if (!toggle) return id ? [id] : [];
  if (!id) return sel;
  return sel.includes(id) ? sel.filter(x => x !== id) : [...sel, id];
}

/** Remove elements and the doors hosted on removed walls; drop dangling egress doors and dependencies.
 * Returns the number of elements removed. */
export function deleteElements(project, ids) {
  const gone = new Set(ids), n = project.elements.length;
  for (const e of project.elements) if (e.kind.Door && gone.has(e.kind.Door.host)) gone.add(e.id);
  project.elements = project.elements.filter(e => !gone.has(e.id));
  for (const s of project.spaces) s.egress_doors = s.egress_doors.filter(d => !gone.has(d));
  for (const e of project.elements) if (e.depends_on) { e.depends_on = e.depends_on.filter(d => !gone.has(d)); if (!e.depends_on.length) delete e.depends_on; }
  return n - project.elements.length;
}

/** Move elements by (dx, dz) in plan. Doors slide along their wall unless the wall moves too. */
export function moveElements(project, ids, dx, dz, snap = v => v) {
  const set = new Set(ids), byId = new Map(project.elements.map(e => [e.id, e]));
  let moved = 0;
  for (const id of ids) {
    const e = byId.get(id); if (!e) continue;
    const kind = kindOf(e), k = e.kind[kind];
    if (kind === 'Door') {
      if (set.has(k.host)) continue; // rides along with its wall
      const w = byId.get(k.host); if (!w || !w.kind.Wall) continue;
      const [sx,, sz] = w.kind.Wall.start, [ex,, ez] = w.kind.Wall.end, len = Math.hypot(ex - sx, ez - sz) || 1;
      const along = (dx * (ex - sx) + dz * (ez - sz)) / len;
      k.offset = Math.max(0, Math.min(len - k.width, snap(k.offset + along)));
    } else for (const v of VECS[kind] || []) { k[v][0] = snap(k[v][0] + dx); k[v][2] = snap(k[v][2] + dz); }
    moved++;
  }
  return moved;
}

/** Deep copies of the selected elements, in model order. */
export function copyElements(project, ids) {
  const set = new Set(ids);
  return project.elements.filter(e => set.has(e.id)).map(clone);
}

/**
 * Paste copies with new ids, offset by (dx, dz) and snapped. A door keeps its copied host (remapped);
 * if its host was not copied it is pasted onto the original host while that still exists, else dropped.
 * Dependencies are remapped the same way. Appends to project.elements; returns the new elements.
 */
export function pasteElements(project, clip, { uuid, dx = 1, dz = 1, snap = v => v } = {}) {
  const exists = new Set(project.elements.map(e => e.id)), map = new Map();
  for (const e of clip) map.set(e.id, uuid());
  const keep = clip.filter(e => !e.kind.Door || map.has(e.kind.Door.host) || exists.has(e.kind.Door.host));
  for (const e of clip) if (!keep.includes(e)) map.delete(e.id);
  const out = keep.map(src => {
    const e = clone(src), kind = kindOf(e), k = e.kind[kind];
    e.id = map.get(src.id);
    if (kind === 'Door') {
      if (map.has(k.host)) k.host = map.get(k.host);
      else { // same wall as the original: slide along it by the paste offset (at least a door width) so the openings don't overlap
        const w = project.elements.find(x => x.id === k.host);
        if (w && w.kind.Wall) {
          const [sx,, sz] = w.kind.Wall.start, [ex,, ez] = w.kind.Wall.end, len = Math.hypot(ex - sx, ez - sz) || 1;
          const along = (dx * (ex - sx) + dz * (ez - sz)) / len, step = (along < 0 ? -1 : 1) * Math.max(Math.abs(along), k.width);
          k.offset = Math.max(0, Math.min(len - k.width, snap(k.offset + step)));
        }
      }
    }
    else for (const v of VECS[kind] || []) { k[v][0] = snap(k[v][0] + dx); k[v][2] = snap(k[v][2] + dz); }
    if (e.depends_on) { e.depends_on = e.depends_on.map(d => map.get(d) || (exists.has(d) ? d : null)).filter(Boolean); if (!e.depends_on.length) delete e.depends_on; }
    return e;
  });
  project.elements.push(...out);
  return out;
}

/** Plan rectangle [x, z, size_x, size_z] from two dragged corners, or null when degenerate. */
export function rectBounds(a, b, min = 0.5) {
  const x = Math.min(a[0], b[0]), z = Math.min(a[1], b[1]), sx = Math.abs(a[0] - b[0]), sz = Math.abs(a[1] - b[1]);
  return sx >= min - 1e-9 && sz >= min - 1e-9 ? [x, z, sx, sz] : null;
}

/** Shared value of a field across elements, or undefined when they differ. */
export function commonValue(els, key) {
  if (!els.length) return undefined;
  const v = els[0][key];
  return els.every(e => e[key] === v) ? v : undefined;
}

/** True when making `id` depend on `dep` would close a cycle (dep already waits on id). */
export function dependsCycle(project, id, dep) {
  if (id === dep) return true;
  const byId = new Map(project.elements.map(e => [e.id, e])), seen = new Set(), stack = [dep];
  while (stack.length) {
    const x = stack.pop(); if (x === id) return true;
    if (seen.has(x)) continue; seen.add(x);
    for (const d of (byId.get(x) || {}).depends_on || []) stack.push(d);
  }
  return false;
}
