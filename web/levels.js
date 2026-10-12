// Level edits that must keep the model consistent.

// Vector fields per element kind; [x, y, z] with y up. Doors have none (they follow their wall).
const VECS = { Wall: ['start', 'end'], Slab: ['origin'], Column: ['base'], Beam: ['start', 'end'], Stair: ['base'], Ramp: ['base'], Mesh: ['origin'], Door: [] };

/**
 * Set a level's elevation and move everything on it by the same amount, so changing a
 * floor-to-floor height keeps walls, slabs and columns on their floor. Spaces need no move:
 * their outlines are drawn at their level's elevation. Returns how many elements moved.
 */
export function setLevelElevation(project, id, elevation) {
  const level = (project.levels || []).find(l => l.id === id);
  if (!level) return 0;
  const dy = elevation - level.elevation_m;
  level.elevation_m = elevation;
  if (!dy) return 0;
  let moved = 0;
  for (const e of project.elements) {
    if (e.level !== id) continue;
    const kind = Object.keys(e.kind)[0], k = e.kind[kind];
    for (const v of VECS[kind] || []) k[v][1] += dy;
    moved++;
  }
  return moved;
}
