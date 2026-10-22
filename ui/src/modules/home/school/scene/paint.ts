// Otto School — canvas painters for the kit's live labels: a corridor door's
// name plate and status sign, and the room's board title. Each repaints only
// when its key (the text it shows) changed.

import { roomSummary, type Room } from '../model.ts';
import type { Painted } from './shared.ts';

export function paintPlate(plate: Painted, r: Room): void {
  const key = r.name;
  if (plate.key !== key) {
    plate.key = key;
    const { ctx, canvas: c } = plate;
    ctx.fillStyle = '#f6f1e3';
    ctx.fillRect(0, 0, c.width, c.height);
    ctx.strokeStyle = '#8a6a3c';
    ctx.lineWidth = 10;
    ctx.strokeRect(5, 5, c.width - 10, c.height - 10);
    ctx.fillStyle = '#2b2118';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    let size = 64;
    ctx.font = `700 ${size}px ui-rounded, -apple-system, system-ui, sans-serif`;
    while (size > 26 && ctx.measureText(r.name).width > c.width - 50) {
      size -= 4;
      ctx.font = `700 ${size}px ui-rounded, -apple-system, system-ui, sans-serif`;
    }
    ctx.fillText(r.name, c.width / 2, c.height / 2 + 2);
    plate.tex.needsUpdate = true;
  }
}

export function paintSign(sign: Painted, r: Room): void {
  const c = r.counts;
  const key = JSON.stringify(c);
  if (sign.key !== key) {
    sign.key = key;
    const { ctx, canvas: cv } = sign;
    // A room that needs you lights its sign amber; the rows are big
    // enough to read from across the corridor.
    ctx.fillStyle = c.needsYou ? '#3d2a00' : c.error ? '#3a1414' : '#18212e';
    ctx.fillRect(0, 0, cv.width, cv.height);
    if (c.needsYou || c.error) {
      ctx.strokeStyle = c.needsYou ? '#ffb020' : '#ff4d4d';
      ctx.lineWidth = 12;
      ctx.strokeRect(6, 6, cv.width - 12, cv.height - 12);
    }
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const rows: [string, string][] = [];
    if (c.needsYou) rows.push(['#ffc246', `✋ ${c.needsYou} ${c.needsYou === 1 ? 'needs' : 'need'} you`]);
    if (c.error) rows.push(['#ff8080', `✕ ${c.error} failed`]);
    if (c.working) rows.push(['#5fe08a', `● ${c.working} working`]);
    if (c.idle) rows.push(['#b8c7da', `○ ${c.idle} idle`]);
    if (c.done) rows.push(['#8cc4ff', `✓ ${c.done} done`]);
    if (!rows.length) rows.push(['#b8c7da', c.total ? `${c.total} asleep` : 'empty']);
    const shown = rows.slice(0, 3);
    const step = (cv.height - 24) / shown.length;
    ctx.font = `700 ${shown.length > 2 ? 44 : 52}px -apple-system, system-ui, sans-serif`;
    shown.forEach(([col, txt], i) => {
      ctx.fillStyle = col;
      ctx.fillText(txt, 22, 12 + step * (i + 0.5), cv.width - 40);
    });
    sign.tex.needsUpdate = true;
  }
}

export function paintBoardTitle(b: Painted | null, r: Room): void {
  if (!b || b.key === `${r.name}|${roomSummary(r)}`) return;
  b.key = `${r.name}|${roomSummary(r)}`;
  const { ctx, canvas: c } = b;
  ctx.fillStyle = '#2f5d46';
  ctx.fillRect(0, 0, c.width, c.height);
  ctx.fillStyle = 'rgba(255,255,255,0.92)';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  let size = 70;
  ctx.font = `600 ${size}px "Chalkboard SE", "Comic Sans MS", ui-rounded, system-ui, sans-serif`;
  while (size > 28 && ctx.measureText(r.name).width > c.width - 40) {
    size -= 4;
    ctx.font = `600 ${size}px "Chalkboard SE", "Comic Sans MS", ui-rounded, system-ui, sans-serif`;
  }
  ctx.fillText(r.name, c.width / 2, c.height * 0.42);
  ctx.font = '30px "Chalkboard SE", "Comic Sans MS", ui-rounded, system-ui, sans-serif';
  ctx.fillStyle = 'rgba(255,255,255,0.7)';
  ctx.fillText(roomSummary(r), c.width / 2, c.height * 0.82);
  b.tex.needsUpdate = true;
}
