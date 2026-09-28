window.renderReport=function(D, opts){
opts=opts||{};
if(window.__viewer){window.__viewer.dispose();}
const $=id=>document.getElementById(id);
const COL={Structural:0x8d8f96,Architectural:0xd9c9ad,Mechanical:0x4f7fd8,Electrical:0xe0b23a,Plumbing:0x3aa0a8,Site:0x6f9c5a};
if(!opts.keepTitle)$('title').textContent=D.project; $('subtitle').textContent=`${D.rule_set} · ${D.elements.length} elements · ${D.total_days} days`;
$('total').textContent=D.total_days; $('slider').max=D.total_days;
$('legend').innerHTML=Object.entries(COL).map(([k,v])=>`<span style="--c:#${v.toString(16).padStart(6,'0')}">${k}</span>`).join('');
$('phases').innerHTML=D.phases.map(p=>{const l=100*p.start_day/Math.max(1,D.total_days),w=100*(p.end_day-p.start_day)/Math.max(1,D.total_days);
 return `<div class="row${p.critical?' crit':''}"><span>${p.name}</span><span>d${p.start_day}–${p.end_day}</span></div>
 <div class="gantt"><i style="left:${l}%;width:${Math.max(w,1)}%;background:${p.critical?'var(--accent)':'var(--muted)'}"></i></div>`}).join('')
 +`<div class="gantt now"><i id="ganttNow" style="left:0%;width:2px;background:var(--err)"></i></div>`;
$('costs').innerHTML=D.phases.map(p=>`<div class="row"><span>${p.name}</span><span>$${p.cost_usd.toLocaleString(undefined,{minimumFractionDigits:2})}</span></div>`).join('')+`<div class="row"><b>Total</b><b>$${D.phases.reduce((a,p)=>a+p.cost_usd,0).toLocaleString(undefined,{minimumFractionDigits:2})}</b></div>`;
$('nadv').textContent=D.advisories.length; $('nclash').textContent=D.clashes.length;
$('advs').innerHTML=D.advisories.length?D.advisories.map(a=>`<div class="adv ${a.severity.toLowerCase()}"><b>${a.subject}</b>: ${a.message}<div class="cite">${a.cite}</div></div>`).join(''):'<div class="row"><span>None raised by this rule set</span></div>';
$('disc').textContent=`ADVISORY ONLY — not a compliance determination. Rule set ${D.rule_set} (${D.code_edition}); verification: ${D.rules_verified}. Cost figures are LOD-200 estimates from sample unit rates.`;

const canvas=$('c'), R=new THREE.WebGLRenderer({canvas,antialias:true}); R.setPixelRatio(Math.min(devicePixelRatio,2));
const S=new THREE.Scene(); S.background=new THREE.Color(getComputedStyle(document.documentElement).getPropertyValue('--bg').trim()||'#f4f2ee');
const cam=new THREE.PerspectiveCamera(45,1,0.1,500);
S.add(new THREE.HemisphereLight(0xffffff,0x8a8070,0.9)); const sun=new THREE.DirectionalLight(0xffffff,0.8); sun.position.set(20,30,10); S.add(sun);
const bb=new THREE.Box3(); const meshes=[];
for(const e of D.elements){
  const g=new THREE.BufferGeometry();
  g.setAttribute('position',new THREE.Float32BufferAttribute(e.positions,3));
  g.setAttribute('normal',new THREE.Float32BufferAttribute(e.normals,3));
  g.setIndex(e.indices);
  const m=new THREE.Mesh(g,new THREE.MeshLambertMaterial({color:COL[e.discipline]||0x999999}));
  m.userData=e; m.visible=false; S.add(m); meshes.push(m); g.computeBoundingBox(); bb.union(g.boundingBox);
}
const dark=matchMedia('(prefers-color-scheme: dark)').matches&&document.documentElement.dataset.theme!=='light';
const grid=new THREE.GridHelper(40,40,dark?0x4a463f:0xbbb5aa,dark?0x2e2b27:0xdcd7cd); grid.position.y=-0.01; S.add(grid);
const clashGroup=new THREE.Group(); clashGroup.visible=false; S.add(clashGroup);
const spaceGroup=new THREE.Group(); S.add(spaceGroup);
for(const sp of (D.spaces||[])){ if(!sp.bounds) continue; const [x,z,sx,sz]=sp.bounds;
  const g=new THREE.BufferGeometry().setFromPoints([[x,0.02,z],[x+sx,0.02,z],[x+sx,0.02,z+sz],[x,0.02,z+sz],[x,0.02,z]].map(p=>new THREE.Vector3(...p)));
  spaceGroup.add(new THREE.Line(g,new THREE.LineBasicMaterial({color:0x3f7a3a})));}
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
function place(){cam.position.set(ctr.x+radius*Math.sin(phi)*Math.cos(theta),ctr.y+radius*Math.cos(phi),ctr.z+radius*Math.sin(phi)*Math.sin(theta));cam.lookAt(ctr);}
const ray=new THREE.Raycaster(), ndc=new THREE.Vector2(), ground=new THREE.Plane(new THREE.Vector3(0,1,0),0);
function toNdc(cx,cy){const r=canvas.getBoundingClientRect();ndc.set(((cx-r.left)/r.width)*2-1,-((cy-r.top)/r.height)*2+1);ray.setFromCamera(ndc,cam);}
function pickHit(cx,cy){toNdc(cx,cy);return ray.intersectObjects(meshes.filter(m=>m.visible),false)[0]||null;}
function pick(cx,cy){const h=pickHit(cx,cy);return h?h.object.userData.id:null;}
function groundPoint(cx,cy){toNdc(cx,cy);const v=new THREE.Vector3();return ray.ray.intersectPlane(ground,v)?[v.x,v.z]:null;}
let selected=null; function highlight(id){selected=id;for(const m of meshes){m.material.emissive.setHex(m.userData.id===id?0x8a3a10:0x000000);}if(opts.handlesFor)showHandles(opts.handlesFor(id));}
const marker=new THREE.Mesh(new THREE.SphereGeometry(0.15,12,12),new THREE.MeshBasicMaterial({color:0xb5541c}));marker.visible=false;S.add(marker);
let drag=null, moved=false, moving=null;
canvas.addEventListener('pointerdown',e=>{
  drag={x:e.clientX,y:e.clientY}; moved=false; canvas.setPointerCapture(e.pointerId);
  const hIdx=pickHandle(e.clientX,e.clientY);
  if(hIdx!==null && opts.onHandle){ const g=groundPoint(e.clientX,e.clientY); if(g){ moving={handle:hIdx,from:g,meshes:[]}; return; } }
  if(selected && opts.onMove && pick(e.clientX,e.clientY)===selected){
    const g=groundPoint(e.clientX,e.clientY);
    if(g) moving={from:g,meshes:meshes.filter(m=>m.userData.id===selected).map(m=>({m,p:m.position.clone()}))};
  }
});
canvas.addEventListener('pointermove',e=>{if(!drag)return;if(Math.hypot(e.clientX-drag.x,e.clientY-drag.y)>3)moved=true;
  if(moving){const g=groundPoint(e.clientX,e.clientY);if(g&&moving.handle!==undefined){const h=handleGroup.children[moving.handle];if(h)h.position.set(g[0],h.position.y,g[1]);return;}if(g){const dx=g[0]-moving.from[0],dz=g[1]-moving.from[1];for(const o of moving.meshes)o.m.position.set(o.p.x+dx,o.p.y,o.p.z+dz);}return;}
  theta-=(e.clientX-drag.x)*0.006;phi=Math.min(1.5,Math.max(0.15,phi-(e.clientY-drag.y)*0.006));drag={x:e.clientX,y:e.clientY};place()});
canvas.addEventListener('pointerup',e=>{
  const wasDrag=moved, mv=moving; drag=null; moving=null;
  if(mv){ const g=groundPoint(e.clientX,e.clientY);
    if(wasDrag&&g&&mv.handle!==undefined){ opts.onHandle(selected,mv.handle,g[0],g[1]); return; }
    if(wasDrag&&g){ opts.onMove(selected,g[0]-mv.from[0],g[1]-mv.from[1]); return; }
    for(const o of mv.meshes)o.m.position.copy(o.p); }
  if(!wasDrag&&opts.onClick){const h=pickHit(e.clientX,e.clientY);
    opts.onClick({id:h?h.object.userData.id:null,point:h?[h.point.x,h.point.y,h.point.z]:null,ground:groundPoint(e.clientX,e.clientY),shift:e.shiftKey});}
}); canvas.addEventListener('wheel',e=>{e.preventDefault();radius=Math.min(200,Math.max(3,radius*(1+Math.sign(e.deltaY)*0.1)));place()},{passive:false});
let pinch=null; canvas.addEventListener('touchmove',e=>{if(e.touches.length===2){const d=Math.hypot(e.touches[0].clientX-e.touches[1].clientX,e.touches[0].clientY-e.touches[1].clientY);if(pinch)radius=Math.min(200,Math.max(3,radius*pinch/d));pinch=d;place();e.preventDefault()}},{passive:false});
canvas.addEventListener('touchend',()=>pinch=null);
function resize(){const w=canvas.clientWidth,h=canvas.clientHeight;R.setSize(w,h,false);cam.aspect=w/h;cam.updateProjectionMatrix()}
new ResizeObserver(resize).observe(canvas.parentElement); resize(); place();

let day=0, playing=false, last=0;
function setDay(d){day=Math.max(0,Math.min(D.total_days,d));$('slider').value=day;$('day').textContent=day;
  let cost=0; for(const m of meshes){const on=day>=m.userData.start_day;m.visible=on;if(on)cost+=m.userData.cost_usd;}
  $('cost').textContent=cost.toLocaleString(undefined,{minimumFractionDigits:2});
  const n=$('ganttNow'); if(n)n.style.left=(100*day/Math.max(1,D.total_days))+'%';
  if(opts.onDay)opts.onDay(day);}
$('slider').oninput=e=>setDay(+e.target.value);
$('play').onclick=()=>{playing=!playing;$('play').textContent=playing?'Pause':'Play';if(playing&&day>=D.total_days)setDay(0)};
let alive=true;
function frame(t){if(!alive)return;loop(t);}
function loop(t){if(playing&&t-last>120){last=t;setDay(day+1);if(day>=D.total_days){playing=false;$('play').textContent='Play'}}R.render(S,cam);requestAnimationFrame(frame)}
setDay(opts.day||0); requestAnimationFrame(frame);
window.__viewer={dispose(){alive=false;R.dispose();for(const m of meshes){m.geometry.dispose();m.material.dispose();}},get day(){return day},highlight,pick,groundPoint,setMarker(p){if(p){marker.position.set(p[0],0.15,p[1]);marker.visible=true;}else marker.visible=false;},get selected(){return selected},pickHit,showHandles,setSpacesVisible(v){spaceGroup.visible=v}};
if(opts.selected)highlight(opts.selected);
};
