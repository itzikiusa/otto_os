// Storm Royale map painter: the island image (painted once per match) and the
// minimap / full-screen map drawn over it each HUD tick.
import type { ShooterState } from '../sim';
import { mapFor, HALF, waterLevel } from '../map';
import { teamColour } from '../catalog';
import { POINT_NAMES, WAVE_BOARD, WAVE_HELI } from '../modes/index';
import { buyWallsFor, LZ_R } from '../zombies';

export const hex = (n: number) => `#${n.toString(16).padStart(6, '0')}`;
/** The painted island, kept for the match (cleared when the game stops). */
export interface MapCache { image: HTMLCanvasElement | null }

function islandImage(state: ShooterState, cache: MapCache): HTMLCanvasElement {
  if (cache.image) return cache.image;
  const map = mapFor(state.map);
  const c = document.createElement('canvas');
  c.width = c.height = map.size;
  const g = c.getContext('2d')!;
  const img = g.createImageData(map.size, map.size);
  const step = (HALF * 2) / (map.size - 1);
  for (let i = 0; i < map.size * map.size; i++) {
    const h = map.heights[i], x = -HALF + (i % map.size) * step, z = -HALF + Math.floor(i / map.size) * step;
    let r: number, gg: number, b: number;
    if (h < waterLevel(map, x, z)) { r = 46; gg = 128; b = 196; }
    else if (map.theme === 'isle') { if (h < 1.6) { r = 232; gg = 211; b = 154; } else { const k = Math.min(1, h / 18); r = 110 - k * 30; gg = 185 - k * 40; b = 80 - k * 20; } }
    else { const k = Math.min(1, h / 32); r = 215 - k * 40; gg = 150 - k * 50; b = 92 - k * 30; }
    img.data[i * 4] = r; img.data[i * 4 + 1] = gg; img.data[i * 4 + 2] = b; img.data[i * 4 + 3] = 255;
  }
  g.putImageData(img, 0, 0);
  g.fillStyle = 'rgba(60,60,70,0.85)';
  for (const b of map.boxes) { const s = map.size / (HALF * 2); g.fillRect((b.minX + HALF) * s, (b.minZ + HALF) * s, Math.max(1, (b.maxX - b.minX) * s), Math.max(1, (b.maxZ - b.minZ) * s)); }
  cache.image = c;
  return c;
}
/** Island map with the storm, flight path, POIs, structures, supply drops, launch pads and teammates; `big` is the full-screen view. */
export function drawMap(c: HTMLCanvasElement | undefined, state: ShooterState, me: number, big: boolean, yaw: number, cache: MapCache) {
  if (!c) return;
  const g = c.getContext('2d'); if (!g) return;
  const w = c.width, s = w / (HALF * 2);
  // Mirror X so the map reads like the third-person view (+X is to the left facing +Z).
  const mx = (x: number) => w - (x + HALF) * s, mz = (z: number) => (z + HALF) * s;
  g.save();
  g.clearRect(0, 0, w, w);
  g.translate(w, 0); g.scale(-1, 1); g.drawImage(islandImage(state, cache), 0, 0, w, w); g.restore();
  g.fillStyle = 'rgba(255,170,60,0.9)';
  for (const b of state.builds) { const a = b.aabb; g.fillRect(mx(a.maxX), mz(a.minZ), Math.max(1.5, (a.maxX - a.minX) * s), Math.max(1.5, (a.maxZ - a.minZ) * s)); }
  const st = state.storm;
  g.fillStyle = 'rgba(120,60,200,0.35)';
  g.beginPath(); g.rect(0, 0, w, w); g.arc(mx(st.cx), mz(st.cz), Math.max(0, st.r) * s, 0, Math.PI * 2, true); g.fill();
  g.strokeStyle = '#ffffff'; g.lineWidth = big ? 3 : 2; g.setLineDash([5, 4]);
  g.beginPath(); g.arc(mx(st.toX), mz(st.toZ), Math.max(0, st.toR) * s, 0, Math.PI * 2); g.stroke(); g.setLineDash([]);
  if (state.phase === 'bus') { g.strokeStyle = 'rgba(255,255,255,0.8)'; g.lineWidth = 3; g.beginPath(); g.moveTo(mx(state.bus.ax), mz(state.bus.az)); g.lineTo(mx(state.bus.bx), mz(state.bus.bz)); g.stroke(); }
  g.font = big ? '700 15px system-ui, sans-serif' : '600 10px system-ui, sans-serif'; g.textAlign = 'center';
  for (const poi of mapFor(state.map).pois) {
    g.lineWidth = 3; g.strokeStyle = 'rgba(0,0,0,0.55)'; g.strokeText(poi.name, mx(poi.x), mz(poi.z));
    g.fillStyle = 'rgba(255,255,255,0.95)'; g.fillText(poi.name, mx(poi.x), mz(poi.z));
  }
  for (const d of state.drops) {
    g.fillStyle = '#4aa8ff'; g.strokeStyle = '#ffffff'; g.lineWidth = 1.5;
    const r = big ? 7 : 4.5; g.fillRect(mx(d.x) - r, mz(d.z) - r, r * 2, r * 2); g.strokeRect(mx(d.x) - r, mz(d.z) - r, r * 2, r * 2);
  }
  g.fillStyle = 'rgba(98,224,255,0.95)';
  for (const pad of state.pads) { g.beginPath(); g.arc(mx(pad.x), mz(pad.z), big ? 4 : 2.5, 0, Math.PI * 2); g.fill(); }
  const self = state.players[me];
  // Domination points (owner colour, letter), Zombies' buy-walls and landing zone.
  state.obj.points.forEach((pt, i) => {
    g.fillStyle = pt.own >= 0 ? hex(teamColour(pt.own)) : 'rgba(255,255,255,0.85)'; g.strokeStyle = '#000'; g.lineWidth = 1.5;
    g.beginPath(); g.arc(mx(pt.x), mz(pt.z), big ? 11 : 7, 0, Math.PI * 2); g.fill(); g.stroke();
    g.fillStyle = '#000'; g.fillText(POINT_NAMES[i], mx(pt.x), mz(pt.z) + (big ? 5 : 3.5));
  });
  if (state.mode === 'zombies') {
    g.fillStyle = '#ffc23a';
    for (const w of buyWallsFor(mapFor(state.map))) g.fillRect(mx(w.x) - 2.5, mz(w.z) - 2.5, 5, 5);
    const o = state.obj;
    if (o.wave === WAVE_HELI || o.wave === WAVE_BOARD || o.round >= o.lastRound - 1) {
      g.strokeStyle = '#62e0ff'; g.lineWidth = big ? 3 : 2;
      g.beginPath(); g.arc(mx(o.lzX), mz(o.lzZ), Math.max(4, LZ_R * s), 0, Math.PI * 2); g.stroke();
      g.fillStyle = '#62e0ff'; g.fillText('LZ', mx(o.lzX), mz(o.lzZ) + 4);
    }
  }
  // The infected (Zombies: every one; Apocalypse: those close by).
  g.fillStyle = '#8fd13a';
  for (const z of state.infected) { if (state.mode === 'apoc' && self && Math.hypot(z.x - self.x, z.z - self.z) > 70) continue; g.fillRect(mx(z.x) - 1.5, mz(z.z) - 1.5, 3, 3); }
  // Enemies on radar: the team's UAV shows everyone; unsuppressed fire pings the shooter for a moment.
  const uav = !!self && (state.uav[self.team] ?? 0) > 0;
  g.fillStyle = '#ff4a3a'; g.strokeStyle = '#fff'; g.lineWidth = 1;
  for (const q of state.players) {
    if (q.mode === 'dead' || q.mode === 'bus' || !self || q.team === self.team || (!uav && q.spotted <= 0)) continue;
    g.beginPath(); g.arc(mx(q.x), mz(q.z), big ? 5 : 3.2, 0, Math.PI * 2); g.fill(); g.stroke();
  }
  // Teammates in the team colour (team modes), other people in yellow (solo).
  for (const p of state.players) {
    if (p.mode === 'dead' || p.id === me) continue;
    const mate = state.squad > 1 && p.team === self?.team;
    if (!mate && (state.squad > 1 || !p.human)) continue;
    g.fillStyle = mate ? hex(teamColour(p.team)) : '#ffd23f'; g.strokeStyle = p.mode === 'down' ? '#ff5a4a' : '#000'; g.lineWidth = p.mode === 'down' ? 2.5 : 1;
    g.beginPath(); g.arc(mx(p.x), mz(p.z), big ? 6 : 4, 0, Math.PI * 2); g.fill(); g.stroke();
  }
  const p = self;
  if (p && p.mode !== 'dead') {
    const k = big ? 1.6 : 1;
    g.save(); g.translate(mx(p.x), mz(p.z)); g.rotate(yaw); g.scale(k, k);
    g.fillStyle = '#ffffff'; g.strokeStyle = '#000'; g.lineWidth = 1.5;
    g.beginPath(); g.moveTo(0, 7); g.lineTo(5, -5); g.lineTo(0, -2); g.lineTo(-5, -5); g.closePath(); g.fill(); g.stroke();
    g.restore();
    // Outside the next circle: a line to safety.
    if (Math.hypot(p.x - st.toX, p.z - st.toZ) > st.toR) { g.strokeStyle = 'rgba(255,255,255,0.7)'; g.lineWidth = 2; g.beginPath(); g.moveTo(mx(p.x), mz(p.z)); g.lineTo(mx(st.toX), mz(st.toZ)); g.stroke(); }
  }
}
