// BCF 2.1 export (buildingSMART BIM Collaboration Format): one topic per finding or clash,
// with the element selected by IFC GlobalId, a camera framed on it and a snapshot, so the
// issues open in Revit, Solibri, BIMcollab and other BCF tools. Pure functions; the viewer
// supplies snapshots. Coordinates: the model is Y-up metres, re-centred on import; BCF
// cameras are in the source IFC frame (Z-up metres), so the import offset is undone here.

const B64 = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$';

/** UUID -> 22-char IFC GlobalId (inverse of the importer's decoding). */
export function ifcGuid(uuid) {
  let n = BigInt('0x' + String(uuid).replace(/-/g, ''));
  let s = '';
  for (let i = 0; i < 21; i++) { s = B64[Number(n & 63n)] + s; n >>= 6n; }
  return B64[Number(n & 3n)] + s;
}

/** Deterministic GUID from text (two FNV-1a 64-bit hashes), so re-exports update the same topics. */
export function stableGuid(text) {
  const fnv = (seed) => {
    let h = 0xcbf29ce484222325n ^ BigInt(seed);
    for (const c of new TextEncoder().encode(text)) { h ^= BigInt(c); h = (h * 0x100000001b3n) & 0xffffffffffffffffn; }
    return h.toString(16).padStart(16, '0');
  };
  const h = fnv(1) + fnv(2);
  // RFC 4122 layout: version 5 nibble, variant 10xx
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-5${h.slice(13, 16)}-${((parseInt(h[16], 16) & 3) | 8).toString(16)}${h.slice(17, 20)}-${h.slice(20, 32)}`;
}

const xml = s => String(s ?? '').replace(/[<>&"']/g, c => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' })[c])
  // drop characters XML 1.0 cannot carry
  .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F]/g, '');
const num = v => Number.isFinite(v) ? String(+v.toFixed(6)) : '0';
const xyz = (tag, [x, y, z]) => `<${tag}><X>${num(x)}</X><Y>${num(y)}</Y><Z>${num(z)}</Z></${tag}>`;

/** Model point (Y up, re-centred) -> source IFC point (Z up). */
export function toIfcPoint([x, y, z], offset = [0, 0, 0]) {
  const [ox, oy, oz] = offset;
  return [x - ox, -(z - oz), y - oy];
}
/** Model direction -> IFC direction (no offset). */
export const toIfcDir = ([x, y, z]) => [x, -z, y];

function bboxOf(positions) {
  const min = [Infinity, Infinity, Infinity], max = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < positions.length; i += 3) for (let k = 0; k < 3; k++) {
    min[k] = Math.min(min[k], positions[i + k]); max[k] = Math.max(max[k], positions[i + k]);
  }
  return min[0] === Infinity ? null : { min, max };
}

/**
 * Topics from an engine report: every error/warning finding about an element or space,
 * and every clash. Project-level notices (rule-set mismatch, unpriced codes) are not issues.
 * Each topic has a `focus` box (model coordinates) for the viewer to frame.
 */
export function bcfTopics(report, { describe = a => a.message } = {}) {
  const els = new Map(report.elements.map(e => [e.id, e]));
  const levelY = id => ((report.levels || []).find(l => l.id === id) || {}).elevation_m || 0;
  const topics = [];
  for (const a of report.advisories) {
    if (a.severity === 'info' || !a.check) continue;
    let focus = null, components = [];
    const el = a.element && els.get(a.element);
    if (el) { focus = bboxOf(el.positions); components = [a.element]; }
    else {
      const sp = (report.spaces || []).find(s => s.name === a.subject);
      if (sp && sp.bounds) { const [x, z, sx, sz] = sp.bounds, y = levelY(sp.level); focus = { min: [x, y, z], max: [x + sx, y + 3, z + sz] }; }
    }
    topics.push({
      guid: stableGuid(`finding|${a.rule_id}|${a.element || a.subject}|${a.check}`),
      type: 'Issue', priority: a.severity === 'error' ? 'High' : 'Normal',
      title: `${a.check[0].toUpperCase()}${a.check.slice(1)}: ${a.subject}`.slice(0, 250),
      description: `${describe(a)}\n${a.cite}\nAdvisory only, not a compliance determination.`,
      labels: ['buildsim', a.rule_id], components, focus,
    });
  }
  for (const c of report.clashes) {
    const na = (els.get(c.a) || {}).name || c.a, nb = (els.get(c.b) || {}).name || c.b;
    topics.push({
      guid: stableGuid(`clash|${[c.a, c.b].sort().join('|')}`),
      type: 'Clash', priority: 'Normal',
      title: `Clash: ${na} / ${nb}`.slice(0, 250),
      description: `${na} overlaps ${nb} by ${c.overlap_m3.toFixed(3)} m³.`,
      labels: ['buildsim', 'clash'], components: [c.a, c.b], focus: { min: c.min, max: c.max },
    });
  }
  return topics;
}

export const versionXml = () => '<?xml version="1.0" encoding="UTF-8"?>\n<Version VersionId="2.1"><DetailedVersion>2.1</DetailedVersion></Version>\n';

export function markupXml(t, { date, author = 'buildsim', viewpointGuid, snapshot }) {
  return '<?xml version="1.0" encoding="UTF-8"?>\n<Markup>' +
    `<Topic Guid="${t.guid}" TopicType="${xml(t.type)}" TopicStatus="Open">` +
    `<Title>${xml(t.title)}</Title><Priority>${xml(t.priority)}</Priority>` +
    t.labels.map(l => `<Labels>${xml(l)}</Labels>`).join('') +
    `<CreationDate>${date}</CreationDate><CreationAuthor>${xml(author)}</CreationAuthor>` +
    `<Description>${xml(t.description)}</Description></Topic>` +
    (viewpointGuid ? `<Viewpoints Guid="${viewpointGuid}"><Viewpoint>viewpoint.bcfv</Viewpoint>${snapshot ? '<Snapshot>snapshot.png</Snapshot>' : ''}</Viewpoints>` : '') +
    '</Markup>\n';
}

/** camera: { position, direction, up } in model coordinates; fov 45-60 per BCF 2.1. */
export function viewpointXml(t, viewpointGuid, camera, offset) {
  const sel = t.components.length
    ? `<Selection>${t.components.map(id => `<Component IfcGuid="${ifcGuid(id)}"/>`).join('')}</Selection>` : '';
  const cam = camera
    ? '<PerspectiveCamera>' + xyz('CameraViewPoint', toIfcPoint(camera.position, offset)) +
      xyz('CameraDirection', toIfcDir(camera.direction)) + xyz('CameraUpVector', toIfcDir(camera.up)) +
      `<FieldOfView>${num(Math.min(60, Math.max(45, camera.fov || 45)))}</FieldOfView></PerspectiveCamera>` : '';
  return `<?xml version="1.0" encoding="UTF-8"?>\n<VisualizationInfo Guid="${viewpointGuid}"><Components>${sel}<Visibility DefaultVisibility="true"/></Components>${cam}</VisualizationInfo>\n`;
}

// ---- minimal ZIP writer (stored entries; BCF readers accept stored and deflated)
const CRC = new Uint32Array(256).map((_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
export function crc32(bytes) { let c = 0xffffffff; for (const b of bytes) c = CRC[(c ^ b) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; }

export function zipStore(files, when = new Date()) {
  const enc = new TextEncoder(), parts = [], central = [];
  const dosTime = (when.getHours() << 11) | (when.getMinutes() << 5) | (when.getSeconds() >> 1);
  const dosDate = ((when.getFullYear() - 1980) << 9) | ((when.getMonth() + 1) << 5) | when.getDate();
  let offset = 0;
  for (const f of files) {
    const name = enc.encode(f.name), data = typeof f.data === 'string' ? enc.encode(f.data) : f.data, crc = crc32(data);
    const head = (sig, extra) => { const b = new DataView(new ArrayBuffer(extra)); b.setUint32(0, sig, true); return b; };
    const loc = head(0x04034b50, 30);
    loc.setUint16(4, 20, true); loc.setUint16(6, 0x0800, true); loc.setUint16(8, 0, true); // UTF-8 names, stored
    loc.setUint16(10, dosTime, true); loc.setUint16(12, dosDate, true); loc.setUint32(14, crc, true);
    loc.setUint32(18, data.length, true); loc.setUint32(22, data.length, true); loc.setUint16(26, name.length, true);
    parts.push(new Uint8Array(loc.buffer), name, data);
    const cen = head(0x02014b50, 46);
    cen.setUint16(4, 20, true); cen.setUint16(6, 20, true); cen.setUint16(8, 0x0800, true);
    cen.setUint16(12, dosTime, true); cen.setUint16(14, dosDate, true); cen.setUint32(16, crc, true);
    cen.setUint32(20, data.length, true); cen.setUint32(24, data.length, true); cen.setUint16(28, name.length, true);
    cen.setUint32(42, offset, true);
    central.push(new Uint8Array(cen.buffer), name);
    offset += 30 + name.length + data.length;
  }
  const cenSize = central.reduce((n, p) => n + p.length, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true); end.setUint16(8, files.length, true); end.setUint16(10, files.length, true);
  end.setUint32(12, cenSize, true); end.setUint32(16, offset, true);
  const all = [...parts, ...central, new Uint8Array(end.buffer)];
  const out = new Uint8Array(all.reduce((n, p) => n + p.length, 0));
  let o = 0; for (const p of all) { out.set(p, o); o += p.length; }
  return out;
}

/**
 * Whole .bcfzip. `views(topic)` returns { camera, png } (png: Uint8Array) or null.
 */
export function buildBcf(report, project, { views = () => null, now = new Date(), describe } = {}) {
  const offset = project.source_offset || [0, 0, 0];
  const date = now.toISOString().replace(/\.\d{3}Z$/, 'Z');
  const files = [{ name: 'bcf.version', data: versionXml() }];
  const topics = bcfTopics(report, { describe });
  for (const t of topics) {
    const v = views(t), vp = stableGuid(`viewpoint|${t.guid}`);
    files.push({ name: `${t.guid}/markup.bcf`, data: markupXml(t, { date, viewpointGuid: vp, snapshot: !!(v && v.png) }) });
    files.push({ name: `${t.guid}/viewpoint.bcfv`, data: viewpointXml(t, vp, v && v.camera, offset) });
    if (v && v.png) files.push({ name: `${t.guid}/snapshot.png`, data: v.png });
  }
  return { zip: zipStore(files, now), topics, files };
}
