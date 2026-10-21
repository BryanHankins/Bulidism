window.renderReport=function(D, opts){
opts=opts||{};
if(window.__viewer){window.__viewer.dispose();}
const $=id=>document.getElementById(id);
const esc=v=>String(v).replace(/[&<>"']/g,c=>'&#'+c.charCodeAt(0)+';');
// citation link-outs come from cites.js when the page loaded it; plain escaped text otherwise
const citeHtml=(window.BuildsimCites&&window.BuildsimCites.citeHtml)||esc;
const COL={Structural:0x8d8f96,Architectural:0xd9c9ad,Mechanical:0x4f7fd8,Electrical:0xe0b23a,Plumbing:0x3aa0a8,Site:0x6f9c5a};
if(!opts.keepTitle)$('title').textContent=D.project; $('subtitle').textContent=`${D.rule_set} · ${D.elements.length} elements · ${D.total_days} days`;
$('total').textContent=D.total_days; $('slider').max=D.total_days;
$('legend').innerHTML=Object.entries(COL).map(([k,v])=>`<span style="--c:#${v.toString(16).padStart(6,'0')}">${k}</span>`).join('');
const pct=d=>100*d/Math.max(1,D.total_days);
// element bars (start_day..finish_day) for elements with their own duration, when the engine reports finish_day
const timed=D.elements.filter(e=>e.finish_day!=null&&(e.duration_days!=null?e.duration_days>0:e.finish_day>e.start_day));
const elBars=p=>{const es=timed.filter(e=>e.phase===p.id), MAX=30;
 return es.slice(0,MAX).map(e=>`<div class="row sub"><span>${esc(e.name)}</span><span>d${e.start_day}–${e.finish_day}</span></div><div class="gantt el"><i style="left:${pct(e.start_day)}%;width:${Math.max(pct(e.finish_day-e.start_day),1)}%"></i></div>`).join('')
  +(es.length>MAX?`<div class="row sub"><span>+${es.length-MAX} more timed elements</span></div>`:'');};
$('phases').innerHTML=D.phases.map(p=>{const l=pct(p.start_day),w=pct(p.end_day-p.start_day);
 return `<div class="row${p.critical?' crit':''}"><span>${esc(p.name)}</span><span>d${p.start_day}–${p.end_day}</span></div>
 <div class="gantt"><i style="left:${l}%;width:${Math.max(w,1)}%;background:${p.critical?'var(--accent)':'var(--muted)'}"></i></div>`+elBars(p)}).join('')
 +`<div class="gantt now"><i id="ganttNow" style="left:0%;width:2px;background:var(--err)"></i></div>`;
$('costs').innerHTML=D.phases.map(p=>`<div class="row"><span>${esc(p.name)}</span><span>$${p.cost_usd.toLocaleString(undefined,{minimumFractionDigits:2})}</span></div>`).join('')+`<div class="row"><b>Total</b><b>$${D.phases.reduce((a,p)=>a+p.cost_usd,0).toLocaleString(undefined,{minimumFractionDigits:2})}</b></div>`;
$('nadv').textContent=D.advisories.length; $('nclash').textContent=D.clashes.length;
$('advs').innerHTML=D.advisories.length?D.advisories.map(a=>`<div class="adv ${esc(a.severity.toLowerCase())}"><b>${esc(a.subject)}</b>: ${esc(a.message)}<div class="cite">${citeHtml(a.cite)}</div></div>`).join(''):'<div class="row"><span>None raised by this rule set</span></div>';
$('disc').textContent=`ADVISORY ONLY — not a compliance determination. Rule set ${D.rule_set} (${D.code_edition}); verification: ${D.rules_verified}. Cost figures are LOD-200 estimates from sample unit rates.`;

const canvas=$('c'), R=new THREE.WebGLRenderer({canvas,antialias:true}); R.setPixelRatio(Math.min(devicePixelRatio,2));
// every rerun builds a new viewer on the same canvas; listeners must die with it
const ac=new AbortController(), on=(t,f,o)=>canvas.addEventListener(t,f,{...o,signal:ac.signal});
const S=new THREE.Scene(); S.background=new THREE.Color(getComputedStyle(document.documentElement).getPropertyValue('--bg').trim()||'#f4f2ee');
const cam=new THREE.PerspectiveCamera(45,1,0.1,500);
S.add(new THREE.HemisphereLight(0xffffff,0x8a8070,0.9)); const sun=new THREE.DirectionalLight(0xffffff,0.8); sun.position.set(20,30,10); S.add(sun);
const bb=new THREE.Box3(); const meshes=[];
const HI=0x8a3a10, elGeom=e=>{const g=new THREE.BufferGeometry();
  g.setAttribute('position',new THREE.Float32BufferAttribute(e.positions,3)); g.setAttribute('normal',new THREE.Float32BufferAttribute(e.normals,3)); g.setIndex(e.indices); return g;};
// Large models: one merged mesh per discipline colour instead of one draw call per element. Elements in a
// group are sorted by start_day, so the 4D day is a draw range; level filters / selection rebuild the index.
const MERGE=opts.merge!=null?!!opts.merge:D.elements.length>2000;
const groups=[], overlays=new Map(); // overlays: selected elements drawn individually (merged mode)
if(MERGE){
  const gm=new Map();
  for(const e of D.elements){ if(!e.indices.length)continue; const c=COL[e.discipline]||0x999999; if(!gm.has(c))gm.set(c,[]); gm.get(c).push(e); }
  const lo=[Infinity,Infinity,Infinity], hi=[-Infinity,-Infinity,-Infinity];
  for(const [color,els] of gm){
    els.sort((a,b)=>a.start_day-b.start_day);
    let nv=0,ni=0; for(const e of els){nv+=e.positions.length;ni+=e.indices.length;}
    const pos=new Float32Array(nv),nor=new Float32Array(nv),full=new Uint32Array(ni),istart=new Uint32Array(els.length+1),starts=new Float64Array(els.length);
    let pv=0,pi=0; const box=new Float32Array(els.length*6); // per-element AABB for picking
    els.forEach((e,k)=>{ const P=e.positions,I=e.indices,base=pv/3; pos.set(P,pv); nor.set(e.normals,pv); istart[k]=pi; starts[k]=e.start_day;
      for(let j=0;j<I.length;j++)full[pi+j]=I[j]+base;
      const b=k*6; box.fill(Infinity,b,b+3); box.fill(-Infinity,b+3,b+6);
      for(let j=0;j<P.length;j+=3)for(let a=0;a<3;a++){const v=P[j+a];if(v<box[b+a])box[b+a]=v;if(v>box[b+3+a])box[b+3+a]=v;}
      for(let a=0;a<3;a++){if(box[b+a]<lo[a])lo[a]=box[b+a];if(box[b+3+a]>hi[a])hi[a]=box[b+3+a];}
      pv+=P.length; pi+=I.length; });
    istart[els.length]=pi;
    const g=new THREE.BufferGeometry(); g.setAttribute('position',new THREE.BufferAttribute(pos,3)); g.setAttribute('normal',new THREE.BufferAttribute(nor,3));
    const cur=new THREE.BufferAttribute(full.slice(),1); cur.setUsage(THREE.DynamicDrawUsage); g.setIndex(cur); g.computeBoundingSphere(); g.computeBoundingBox();
    const m=new THREE.Mesh(g,new THREE.MeshLambertMaterial({color})); S.add(m);
    const grp={color,els,full,istart,starts,cur,mesh:m,prefix:true,box,pos,n:0}; groups.push(grp);
  }
  if(groups.length)bb.set(new THREE.Vector3(...lo),new THREE.Vector3(...hi));
}else for(const e of D.elements){
  if(!e.indices.length) continue; // external-mesh elements: costed/scheduled, nothing to draw
  const g=elGeom(e), m=new THREE.Mesh(g,new THREE.MeshLambertMaterial({color:COL[e.discipline]||0x999999}));
  m.userData=e; m.visible=false; S.add(m); meshes.push(m); g.computeBoundingBox(); bb.union(g.boundingBox);
}
const dark=matchMedia('(prefers-color-scheme: dark)').matches&&document.documentElement.dataset.theme!=='light';
const grid=new THREE.GridHelper(40,40,dark?0x4a463f:0xbbb5aa,dark?0x2e2b27:0xdcd7cd); grid.position.y=(opts.groundY||0)-0.01; S.add(grid);
const clashGroup=new THREE.Group(); clashGroup.visible=false; S.add(clashGroup);
const spaceGroup=new THREE.Group(); S.add(spaceGroup);
const levels=D.levels||[], elevOf=id=>{const l=levels.find(l=>l.id===id);return l?l.elevation_m:0;};
for(const sp of (D.spaces||[])){ if(!sp.bounds) continue; const [x,z,sx,sz]=sp.bounds, y=elevOf(sp.level)+0.02;
  const g=new THREE.BufferGeometry().setFromPoints([[x,y,z],[x+sx,y,z],[x+sx,y,z+sz],[x,y,z+sz],[x,y,z]].map(p=>new THREE.Vector3(...p)));
  const ln=new THREE.Line(g,new THREE.LineBasicMaterial({color:0x3f7a3a})); ln.userData.level=sp.level; spaceGroup.add(ln);}
// level filter: isolate one storey (elements with no level are site work, shown under "All")
let levelSel=opts.level??'all';
const levelRow=$('levelRow'), levelFilter=$('levelFilter');
if(levelFilter){ levelRow.hidden=levels.length<2;
  levelFilter.innerHTML='<option value="all">All levels</option>'+levels.map(l=>`<option value="${l.id}">${esc(l.name)}</option>`).join('');
  if(![...levelFilter.options].some(o=>o.value===String(levelSel)))levelSel='all';
  levelFilter.value=String(levelSel);
  levelFilter.onchange=e=>{levelSel=e.target.value;setDay(day);if(opts.onLevel)opts.onLevel(levelSel);}; }
const onLevel=lv=>levelSel==='all'||String(lv)===levelSel;
const handleGroup=new THREE.Group(); S.add(handleGroup);
function showHandles(pts){ handleGroup.clear();
  (pts||[]).forEach((p,i)=>{ const h=new THREE.Mesh(new THREE.SphereGeometry(0.18,12,12),new THREE.MeshBasicMaterial({color:i===0?0x2e6fd8:0xb5541c}));
    h.position.set(p[0],p[1],p[2]); h.userData.handle=i; handleGroup.add(h); }); }
function pickHandle(cx,cy){ toNdc(cx,cy); const h=ray.intersectObjects(handleGroup.children,false)[0]; return h?h.object.userData.handle:null; }
for(const c of D.clashes){const s=new THREE.Vector3().subVectors(new THREE.Vector3(...c.max),new THREE.Vector3(...c.min));
  const box=new THREE.Mesh(new THREE.BoxGeometry(s.x+0.02,s.y+0.02,s.z+0.02),new THREE.MeshBasicMaterial({color:0xd0301c,wireframe:true}));
  box.position.copy(new THREE.Vector3(...c.min).add(s.multiplyScalar(0.5))); clashGroup.add(box);}
$('showClash').onchange=e=>clashGroup.visible=e.target.checked;

// orbit camera (no external controls)
const ctr=bb.getCenter(new THREE.Vector3()); let radius=bb.getSize(new THREE.Vector3()).length()*1.25||20, theta=-0.7, phi=1.1;
if(!isFinite(ctr.x)){ctr.set(0,0,0);radius=20;}
cam.far=Math.max(500,radius*4); cam.updateProjectionMatrix();
function place(){cam.position.set(ctr.x+radius*Math.sin(phi)*Math.cos(theta),ctr.y+radius*Math.cos(phi),ctr.z+radius*Math.sin(phi)*Math.sin(theta));cam.lookAt(ctr);cam.updateMatrixWorld();}
// clicks land on the editor's active level (plane y = groundY)
const ray=new THREE.Raycaster(), ndc=new THREE.Vector2(), ground=new THREE.Plane(new THREE.Vector3(0,1,0),-(opts.groundY||0));
function toNdc(cx,cy){const r=canvas.getBoundingClientRect();ndc.set(((cx-r.left)/r.width)*2-1,-((cy-r.top)/r.height)*2+1);ray.setFromCamera(ndc,cam);}
// hit -> {id, point}; merged meshes map the hit triangle back to its element
function pickHit(cx,cy){toNdc(cx,cy);
  const h=ray.intersectObjects(MERGE?[...overlays.values()].filter(m=>m.visible):meshes.filter(m=>m.visible),false)[0];
  let best=h?{id:h.object.userData.id,point:h.point,d:h.distance}:null;
  if(MERGE)best=pickMerged(best);
  return best&&{id:best.id,point:best.point};}
// merged picking: ray vs per-element boxes (nearest first), exact triangles only for box hits
const _b=new THREE.Box3(),_v=new THREE.Vector3(),_a=new THREE.Vector3(),_c=new THREE.Vector3(),_d=new THREE.Vector3(),_p=new THREE.Vector3();
function pickMerged(best){ const r=ray.ray, cand=[];
  for(const g of groups)for(let k=0;k<g.n;k++){ const e=g.els[k]; if(sel.has(e.id)||!onLevel(e.level))continue; const b=k*6,B=g.box;
    _b.min.set(B[b],B[b+1],B[b+2]); _b.max.set(B[b+3],B[b+4],B[b+5]); if(r.intersectBox(_b,_v))cand.push([_v.distanceTo(r.origin),g,k]); }
  cand.sort((x,y)=>x[0]-y[0]);
  for(const [d0,g,k] of cand){ if(best&&d0>best.d)break; const P=g.pos,F=g.full;
    for(let t=g.istart[k];t<g.istart[k+1];t+=3){ _a.fromArray(P,F[t]*3); _c.fromArray(P,F[t+1]*3); _d.fromArray(P,F[t+2]*3);
      if(r.intersectTriangle(_a,_c,_d,false,_p)){const d=_p.distanceTo(r.origin); if(!best||d<best.d)best={id:g.els[k].id,point:_p.clone(),d};} } }
  return best; }
function pick(cx,cy){const h=pickHit(cx,cy);return h?h.id:null;}
function groundPoint(cx,cy){toNdc(cx,cy);const v=new THREE.Vector3();return ray.ray.intersectPlane(ground,v)?[v.x,v.z]:null;}
// selection: any number of element ids
let sel=new Set();
const byId=new Map(D.elements.map(e=>[e.id,e]));
function highlight(ids){ sel=new Set(ids==null?[]:Array.isArray(ids)?ids:[ids]);
  if(MERGE){ for(const [id,m] of overlays)if(!sel.has(id)){S.remove(m);m.geometry.dispose();m.material.dispose();overlays.delete(id);}
    for(const id of sel){const e=byId.get(id); if(!e||!e.indices.length||overlays.has(id))continue;
      const m=new THREE.Mesh(elGeom(e),new THREE.MeshLambertMaterial({color:COL[e.discipline]||0x999999,emissive:HI})); m.userData=e; S.add(m); overlays.set(id,m);}
    setDay(day); }
  else for(const m of meshes){m.material.emissive.setHex(sel.has(m.userData.id)?HI:0x000000);}
  if(opts.handlesFor)showHandles(sel.size===1?opts.handlesFor([...sel][0]):[]);}
const marker=new THREE.Mesh(new THREE.SphereGeometry(0.15,12,12),new THREE.MeshBasicMaterial({color:0xb5541c}));marker.visible=false;S.add(marker);
// rectangle drag (space tool): preview outline on the ground plane
const snapR=opts.snapRect||(v=>Math.round(v/0.5)*0.5), rectPos=new Float32Array(15);
const rectGeo=new THREE.BufferGeometry(); rectGeo.setAttribute('position',new THREE.BufferAttribute(rectPos,3));
const rectLine=new THREE.Line(rectGeo,new THREE.LineBasicMaterial({color:0x3f7a3a})); rectLine.visible=false; rectLine.frustumCulled=false; S.add(rectLine);
function drawRect(a,b){const y=(opts.groundY||0)+0.03,p=[[a[0],a[1]],[b[0],a[1]],[b[0],b[1]],[a[0],b[1]],[a[0],a[1]]];
  p.forEach((q,i)=>{rectPos[i*3]=q[0];rectPos[i*3+1]=y;rectPos[i*3+2]=q[1];}); rectGeo.attributes.position.needsUpdate=true; rectGeo.computeBoundingSphere(); rectLine.visible=true;}
let drag=null, moved=false, moving=null, rect=null;
on('pointerdown',e=>{
  drag={x:e.clientX,y:e.clientY}; moved=false; canvas.setPointerCapture(e.pointerId);
  if(opts.rectMode&&opts.rectMode()){ const g=groundPoint(e.clientX,e.clientY); if(g){ rect={a:[snapR(g[0]),snapR(g[1])]}; rect.b=rect.a; drawRect(rect.a,rect.b); return; } }
  const hIdx=pickHandle(e.clientX,e.clientY);
  if(hIdx!==null && opts.onHandle){ const g=groundPoint(e.clientX,e.clientY); if(g){ moving={handle:hIdx,from:g,meshes:[]}; return; } }
  if(sel.size && opts.onMove && sel.has(pick(e.clientX,e.clientY))){
    const g=groundPoint(e.clientX,e.clientY);
    const ms=MERGE?[...overlays.values()]:meshes.filter(m=>sel.has(m.userData.id));
    if(g) moving={from:g,meshes:ms.map(m=>({m,p:m.position.clone()}))};
  }
});
on('pointermove',e=>{if(!drag)return;if(Math.hypot(e.clientX-drag.x,e.clientY-drag.y)>3)moved=true;
  if(rect){const g=groundPoint(e.clientX,e.clientY);if(g){rect.b=[snapR(g[0]),snapR(g[1])];drawRect(rect.a,rect.b);}return;}
  if(moving){const g=groundPoint(e.clientX,e.clientY);if(g&&moving.handle!==undefined){const h=handleGroup.children[moving.handle];if(h)h.position.set(g[0],h.position.y,g[1]);return;}if(g){const dx=g[0]-moving.from[0],dz=g[1]-moving.from[1];for(const o of moving.meshes)o.m.position.set(o.p.x+dx,o.p.y,o.p.z+dz);}return;}
  theta-=(e.clientX-drag.x)*0.006;phi=Math.min(1.5,Math.max(0.15,phi-(e.clientY-drag.y)*0.006));drag={x:e.clientX,y:e.clientY};place()});
on('pointerup',e=>{
  const wasDrag=moved, mv=moving, rc=rect; drag=null; moving=null; rect=null;
  if(rc){ rectLine.visible=false; if(wasDrag&&opts.onRect)opts.onRect(rc.a,rc.b); return; }
  if(mv){ const g=groundPoint(e.clientX,e.clientY);
    if(wasDrag&&g&&mv.handle!==undefined){ opts.onHandle([...sel][0],mv.handle,g[0],g[1]); return; }
    if(wasDrag&&g){ opts.onMove([...sel],g[0]-mv.from[0],g[1]-mv.from[1]); return; }
    for(const o of mv.meshes)o.m.position.copy(o.p); }
  if(!wasDrag&&opts.onClick){const h=pickHit(e.clientX,e.clientY);
    opts.onClick({id:h?h.id:null,point:h?[h.point.x,h.point.y,h.point.z]:null,ground:groundPoint(e.clientX,e.clientY),shift:e.shiftKey,toggle:e.shiftKey||e.ctrlKey||e.metaKey});}
}); on('wheel',e=>{e.preventDefault();radius=Math.min(Math.max(200,cam.far/4),Math.max(3,radius*(1+Math.sign(e.deltaY)*0.1)));place()},{passive:false});
let pinch=null; on('touchmove',e=>{if(e.touches.length===2){const d=Math.hypot(e.touches[0].clientX-e.touches[1].clientX,e.touches[0].clientY-e.touches[1].clientY);if(pinch)radius=Math.min(Math.max(200,cam.far/4),Math.max(3,radius*pinch/d));pinch=d;place();e.preventDefault()}},{passive:false});
on('touchend',()=>pinch=null);
function resize(){const w=canvas.clientWidth,h=canvas.clientHeight;R.setSize(w,h,false);cam.aspect=w/h;cam.updateProjectionMatrix()}
const ro=new ResizeObserver(resize); ro.observe(canvas.parentElement); resize(); place();

// merged mode: rebuild each group's index for the day / level / selection (selected elements draw as overlays)
function updateGroups(){
  for(const g of groups){
    let n=0; while(n<g.els.length&&g.starts[n]<=day)n++; // els sorted by start_day: visible-by-day is a prefix
    g.n=n;
    if(levelSel==='all'&&!sel.size){ if(!g.prefix){g.cur.array.set(g.full);g.cur.needsUpdate=true;g.prefix=true;} g.mesh.geometry.setDrawRange(0,g.istart[n]); continue; }
    const a=g.cur.array; let w=0;
    for(let k=0;k<n;k++){const e=g.els[k]; if(sel.has(e.id)||!onLevel(e.level))continue; const s=g.istart[k],t=g.istart[k+1]; a.set(g.full.subarray(s,t),w); w+=t-s;}
    g.cur.needsUpdate=true; g.prefix=false; g.mesh.geometry.setDrawRange(0,w);
  }
  for(const m of overlays.values())m.visible=day>=m.userData.start_day&&onLevel(m.userData.level);
}
let day=0, playing=false, last=0;
function setDay(d){day=Math.max(0,Math.min(D.total_days,d));$('slider').value=day;$('day').textContent=day;
  if(MERGE)updateGroups(); else for(const m of meshes)m.visible=day>=m.userData.start_day&&onLevel(m.userData.level);
  for(const l of spaceGroup.children)l.visible=onLevel(l.userData.level);
  let cost=0; for(const e of D.elements)if(day>=e.start_day)cost+=e.cost_usd;
  $('cost').textContent=cost.toLocaleString(undefined,{minimumFractionDigits:2});
  const n=$('ganttNow'); if(n)n.style.left=pct(day)+'%';
  if(opts.onDay)opts.onDay(day);}
$('slider').oninput=e=>setDay(+e.target.value);
$('play').onclick=()=>{playing=!playing;$('play').textContent=playing?'Pause':'Play';if(playing&&day>=D.total_days)setDay(0)};
let alive=true;
function frame(t){if(!alive)return;loop(t);}
function loop(t){if(playing&&t-last>120){last=t;setDay(day+1);if(day>=D.total_days){playing=false;$('play').textContent='Play'}}R.render(S,cam);requestAnimationFrame(frame)}
setDay(opts.day||0); requestAnimationFrame(frame);
const visibleIds=()=>MERGE?D.elements.filter(e=>e.indices.length&&day>=e.start_day&&onLevel(e.level)).map(e=>e.id):meshes.filter(m=>m.visible).map(m=>m.userData.id);
window.__viewer={dispose(){alive=false;ac.abort();ro.disconnect();R.dispose();for(const m of [...meshes,...groups.map(g=>g.mesh),...overlays.values()]){m.geometry.dispose();m.material.dispose();}},
  get day(){return day},highlight,pick,groundPoint,setMarker(p){if(p){marker.position.set(p[0],(opts.groundY||0)+0.15,p[1]);marker.visible=true;}else marker.visible=false;},
  get selected(){return sel.size?[...sel][0]:null},get selectedIds(){return [...sel]},pickHit,showHandles,setSpacesVisible(v){spaceGroup.visible=v},visibleIds,
  get merged(){return MERGE},stats(){return {calls:R.info.render.calls,triangles:R.info.render.triangles,merged:MERGE}},
  // screen position of a world point (tests / automation)
  project(p){const v=new THREE.Vector3(...p).project(cam),r=canvas.getBoundingClientRect();return [r.left+(v.x+1)/2*r.width,r.top+(1-v.y)/2*r.height];}};
if(opts.selected)highlight(opts.selected);
};
