// Build a valid, correctly-positioned Excalidraw scene from the agent's SIMPLIFIED
// element list — WITHOUT @excalidraw/convertToExcalidrawElements, which scatters a
// shape's label into a bound-text element at (0,0). We place every shape, attach its
// label as a CENTERED bound text (Excalidraw re-derives a bound text's position from
// its container, so it can't collapse), route arrows between shape centres with real
// bindings, and keep standalone text. The result is fed through restoreElements to
// fill any remaining internals.
//
// Simplified input (what the agent writes):
//   shape:  {type:'rectangle'|'ellipse'|'diamond', id, x, y, width, height,
//            backgroundColor?, strokeColor?, fillStyle?, roundness?, label?:{text,fontSize?,fontFamily?}}
//   arrow:  {type:'arrow', start:{id}, end:{id}, strokeColor?, label?:{text}}
//   text:   {type:'text', x, y, text, fontSize?}

/* eslint-disable @typescript-eslint/no-explicit-any */

const SHAPE_TYPES = new Set(['rectangle', 'ellipse', 'diamond']);

function rid(p = 'el'): string {
  return p + Math.random().toString(36).slice(2, 10) + Math.random().toString(36).slice(2, 6);
}
function rnd(): number {
  return Math.floor(Math.random() * 2 ** 31);
}
function num(v: any, d = 0): number {
  const n = Number(v);
  return Number.isFinite(n) ? n : d;
}
function clean(s: any): string {
  return String(s ?? '')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&');
}
function labelText(s: any): string {
  return clean(s.label?.text ?? s.label ?? '');
}

function common(extra: any): any {
  return {
    angle: 0,
    strokeWidth: 2,
    strokeStyle: 'solid',
    roughness: 1,
    opacity: 100,
    groupIds: [],
    frameId: null,
    seed: rnd(),
    versionNonce: rnd(),
    version: 1,
    isDeleted: false,
    boundElements: [],
    updated: Date.now(),
    link: null,
    locked: false,
    ...extra,
  };
}

function makeShape(s: any, textId: string | null): any {
  const width = Math.max(20, num(s.width, 160));
  const height = Math.max(20, num(s.height, 60));
  return common({
    id: typeof s.id === 'string' ? s.id : rid('n'),
    type: s.type,
    x: num(s.x),
    y: num(s.y),
    width,
    height,
    strokeColor: s.strokeColor || '#1e1e1e',
    backgroundColor: s.backgroundColor || '#ffffff',
    fillStyle: s.fillStyle || 'solid',
    roundness: s.type === 'rectangle' ? { type: 3 } : null,
    boundElements: textId ? [{ type: 'text', id: textId }] : [],
  });
}

// A bound text centred on its container. We set x/y to the container centre so it's
// correct even before Excalidraw recomputes; textAlign/verticalAlign + containerId
// make the editor keep it centred.
function makeBoundText(text: string, container: any, fontFamily: number, fontSize: number): any {
  const lines = text.split(/\n/);
  const longest = lines.reduce((m, l) => Math.max(m, l.length), 1);
  const w = Math.min(Math.max(20, container.width - 10), Math.max(20, longest * fontSize * 0.55));
  const h = Math.max(fontSize, lines.length * fontSize * 1.25);
  return common({
    id: rid('t'),
    type: 'text',
    x: container.x + container.width / 2 - w / 2,
    y: container.y + container.height / 2 - h / 2,
    width: w,
    height: h,
    strokeColor: '#1e1e1e',
    backgroundColor: 'transparent',
    fillStyle: 'solid',
    roundness: null,
    fontSize,
    fontFamily: fontFamily || 2,
    text,
    textAlign: 'center',
    verticalAlign: 'middle',
    containerId: container.id,
    originalText: text,
    lineHeight: 1.25,
    autoResize: true,
  });
}

function makeText(t: any): any {
  const text = clean(t.text ?? t.label?.text ?? '');
  const fontSize = num(t.fontSize, 20);
  const lines = text.split(/<br\s*\/?>|\n/).map(clean);
  const longest = lines.reduce((m, l) => Math.max(m, l.length), 1);
  return common({
    id: typeof t.id === 'string' ? t.id : rid('t'),
    type: 'text',
    x: num(t.x),
    y: num(t.y),
    width: Math.max(20, longest * fontSize * 0.55),
    height: Math.max(fontSize, lines.length * fontSize * 1.25),
    strokeColor: t.strokeColor || '#1e1e1e',
    backgroundColor: 'transparent',
    fillStyle: 'solid',
    roundness: null,
    fontSize,
    fontFamily: num(t.fontFamily, 2),
    text: lines.join('\n'),
    textAlign: t.textAlign || 'left',
    verticalAlign: 'top',
    containerId: null,
    originalText: lines.join('\n'),
    lineHeight: 1.25,
    autoResize: true,
  });
}

function center(e: any): { x: number; y: number } {
  return { x: e.x + e.width / 2, y: e.y + e.height / 2 };
}

/** Build the full Excalidraw element list (pre-restore) from a simplified scene. */
export function buildExcalidrawElements(raw: any[]): any[] {
  const out: any[] = [];
  const byId = new Map<string, any>();

  // 1. Shapes (+ centred bound-text labels).
  for (const s of raw) {
    if (!s || typeof s.type !== 'string' || !SHAPE_TYPES.has(s.type)) continue;
    const text = labelText(s);
    const textId = text ? rid('t') : null;
    const shape = makeShape(s, textId);
    out.push(shape);
    byId.set(shape.id, shape);
    if (textId && text) {
      const bt = makeBoundText(
        text.replace(/<br\s*\/?>/g, '\n'),
        shape,
        num(s.label?.fontFamily, 2),
        num(s.label?.fontSize, 16),
      );
      bt.id = textId;
      out.push(bt);
    }
  }

  // 2. Standalone text (lane labels, captions) — keep its own position.
  for (const t of raw) {
    if (!t || t.type !== 'text') continue;
    const el = makeText(t);
    out.push(el);
    byId.set(el.id, el);
  }

  // 3. Arrows routed between shape centres, with bindings + (optional) a label.
  for (const a of raw) {
    if (!a || a.type !== 'arrow') continue;
    const sid = a.start?.id ?? a.startBinding?.elementId;
    const eid = a.end?.id ?? a.endBinding?.elementId;
    const s = sid ? byId.get(sid) : null;
    const t = eid ? byId.get(eid) : null;
    if (!s && !t) continue; // nothing to route between
    const sc = s ? center(s) : { x: num(a.x), y: num(a.y) };
    const tc = t ? center(t) : { x: sc.x + 140, y: sc.y };
    const dx = Math.round(tc.x - sc.x);
    const dy = Math.round(tc.y - sc.y);
    const text = clean(a.label?.text ?? a.label ?? '');
    const arrowId = typeof a.id === 'string' ? a.id : rid('a');
    const textId = text ? rid('t') : null;
    const arrow = common({
      id: arrowId,
      type: 'arrow',
      x: Math.round(sc.x),
      y: Math.round(sc.y),
      width: Math.abs(dx),
      height: Math.abs(dy),
      strokeColor: a.strokeColor || '#6b7280',
      backgroundColor: 'transparent',
      fillStyle: 'solid',
      roundness: { type: 2 },
      points: [
        [0, 0],
        [dx, dy],
      ],
      startBinding: s ? { elementId: s.id, focus: 0, gap: 4 } : null,
      endBinding: t ? { elementId: t.id, focus: 0, gap: 4 } : null,
      startArrowhead: null,
      endArrowhead: 'arrow',
      elbowed: false,
      boundElements: textId ? [{ type: 'text', id: textId }] : [],
    });
    // bind arrow endpoints back on the shapes
    if (s) s.boundElements = [...(s.boundElements ?? []), { type: 'arrow', id: arrowId }];
    if (t) t.boundElements = [...(t.boundElements ?? []), { type: 'arrow', id: arrowId }];
    out.push(arrow);
    if (textId && text) {
      const bt = makeBoundText(text, { ...arrow, width: Math.abs(dx) || 80, height: Math.abs(dy) || 24 }, 2, 14);
      bt.id = textId;
      bt.x = Math.round(sc.x + dx / 2 - bt.width / 2);
      bt.y = Math.round(sc.y + dy / 2 - bt.height / 2);
      out.push(bt);
    }
  }

  return out;
}

/** True when the scene is the agent's simplified form (no Excalidraw internals). */
export function isSimplified(els: any[]): boolean {
  if (!els.length) return false;
  return !els.some((e) => e && (e.versionNonce != null || e.seed != null));
}
