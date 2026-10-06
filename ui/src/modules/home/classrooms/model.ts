// Classrooms — the PURE half of the Home "Classrooms" widget: sessions and
// workspaces in, a scene model out. Workspaces are classrooms laid out on a
// floor grid; sessions are students at desks. No three.js, no DOM, no runes —
// node:test covers it (unit/classrooms.test.ts) and scene.ts only draws what
// this decides.
//
//  • Seating: foreground sessions fill the front rows (needs-you first, then
//    working, then most recently active); background engine sessions (workflow
//    steps, swarm, review agents, scheduled tasks…) sit in a separate BACK ROW,
//    live ones only and capped, so the room stays readable.
//  • Status → visual: the one session state (lib/status.ts) mapped onto how a
//    student looks — typing (working), hand up (needs you), still (idle), ghost
//    at an empty-ish desk (suspended / ended / failed), dim (reconnecting).
//  • Layout: rooms sized by their seat grid, placed row-major on a uniform
//    cell so the overview reads as a campus; the current workspace comes first.

import { sessionState, type SessionStateKey } from '../../../lib/status.ts';
import { isForeground, SCRATCH_ID } from '../../../lib/stores/sessionBuckets.ts';
import type { Session } from '../../../lib/api/types';

// ── Geometry (world units ≈ metres) ─────────────────────────────────────────

/** Desk pitch across / front-to-back. */
export const SEAT_X = 1.5;
export const SEAT_Z = 1.6;
/** Margin between the outermost desks and the walls. */
export const ROOM_PAD = 1.1;
/** Strip at the front of the room (door + headmaster's spot). */
export const FRONT_STRIP = 1.6;
/** Gap between neighbouring rooms. */
export const ROOM_GAP = 2.2;
/** Desks per row bounds. */
export const MIN_COLS = 3;
export const MAX_COLS = 6;
/** Seats per room before the rest collapse into a "+N" count. */
export const MAX_FRONT = 36;
/** Background back-row seats per room (live engines only). */
export const MAX_BACK = 6;

// ── Types ───────────────────────────────────────────────────────────────────

/** How a student is drawn. */
export type StudentVisual = 'working' | 'needs-you' | 'idle' | 'away' | 'stale';

export interface Seat {
  /** Desk row (0 = front) and column. */
  row: number;
  col: number;
  /** World position relative to the room centre. */
  x: number;
  z: number;
}

export interface Student {
  id: string;
  title: string;
  provider: string;
  /** Two-letter identifier on the badge (CL, CX, AG, SH, custom monogram). */
  initials: string;
  /** Category token (`--cat-N`) the provider is coloured with. */
  colorToken: string;
  stateKey: SessionStateKey;
  stateLabel: string;
  visual: StudentVisual;
  /** Engine-spawned (workflow / swarm / review…) — sits in the back row. */
  background: boolean;
  /** `meta.source` of a background session ("workflow"), else null. */
  source: string | null;
  workspaceId: string;
  workspaceName: string;
  lastActiveAt: string;
  cwd: string;
  /** Branch when the session's meta names one (worktree sessions). */
  branch: string | null;
  /** The caller may delete / archive it (role in its workspace). */
  canManage: boolean;
  seat: Seat;
}

export interface Classroom {
  id: string;
  name: string;
  current: boolean;
  scratch: boolean;
  /** Front rows (foreground sessions). */
  students: Student[];
  /** Back row (live background sessions). */
  backRow: Student[];
  /** Sessions not seated (over MAX_FRONT / MAX_BACK). */
  overflow: number;
  cols: number;
  /** Front rows (excl. the back row). */
  rows: number;
  /** Room centre on the floor and its footprint. */
  x: number;
  z: number;
  width: number;
  depth: number;
  counts: { working: number; needsYou: number; idle: number; away: number; total: number };
}

export interface ClassroomModel {
  rooms: Classroom[];
  /** Every seated student, front rows then back rows, room by room. */
  students: Student[];
  /** Floor extent (centred on the origin). */
  width: number;
  depth: number;
  total: number;
}

export interface WorkspaceLike {
  id: string;
  name: string;
  my_role?: 'viewer' | 'editor' | 'admin' | string | null;
  archived?: boolean;
}

export interface ModelInput {
  workspaces: readonly WorkspaceLike[];
  currentId: string | null;
  sessions: readonly Session[];
  /** Events-fed live status (`ws.statusMap`). */
  statusOf?: (id: string) => string | null | undefined;
  needsYou?: (id: string) => boolean;
  /** Event socket down: live claims are stale. */
  stale?: boolean;
  /** Caller holds Agents:Edit (the app-wide gate before any role check). */
  canEditAgents?: boolean;
}

// ── Provider identity ───────────────────────────────────────────────────────

const PROVIDER_TOKENS: Record<string, string> = {
  claude: '--cat-2',
  codex: '--cat-1',
  agy: '--cat-4',
  shell: '--cat-6',
};
const CUSTOM_TOKENS = ['--cat-3', '--cat-5'];
const PROVIDER_INITIALS: Record<string, string> = { claude: 'CL', codex: 'CX', agy: 'AG', shell: 'SH' };
const PROVIDER_LABELS: Record<string, string> = { claude: 'Claude', codex: 'Codex', agy: 'Antigravity', shell: 'Shell' };

/** Category token for a provider: the built-ins are fixed, a custom slug
 *  hashes onto the two remaining categories (stable across reloads). */
export function providerToken(provider: string): string {
  const p = provider.toLowerCase();
  if (PROVIDER_TOKENS[p]) return PROVIDER_TOKENS[p];
  let h = 0;
  for (let i = 0; i < p.length; i++) h = (h * 31 + p.charCodeAt(i)) >>> 0;
  return CUSTOM_TOKENS[h % CUSTOM_TOKENS.length];
}

export function providerInitials(provider: string): string {
  const p = provider.toLowerCase();
  return PROVIDER_INITIALS[p] ?? ((p.match(/[a-z0-9]/g) ?? []).slice(0, 2).join('').toUpperCase() || '?');
}

export function providerLabel(provider: string): string {
  const p = provider.toLowerCase();
  return PROVIDER_LABELS[p] ?? (p ? p[0].toUpperCase() + p.slice(1) : 'Unknown');
}

// ── Status → visual ─────────────────────────────────────────────────────────

export function visualFor(key: SessionStateKey): StudentVisual {
  switch (key) {
    case 'working':
    case 'running':
      return 'working';
    case 'needs-you':
      return 'needs-you';
    case 'idle':
      return 'idle';
    case 'stale':
      return 'stale';
    default:
      // suspended / ended / failed: nobody home.
      return 'away';
  }
}

/** Front-row order: blocked on you, then busy, then idle, then away. */
const VISUAL_RANK: Record<StudentVisual, number> = { 'needs-you': 0, working: 1, stale: 2, idle: 3, away: 4 };

// ── Layout math ─────────────────────────────────────────────────────────────

/** Desks per row for `n` students: roughly 3:2 landscape, within bounds. */
export function columnsFor(n: number): number {
  return Math.min(MAX_COLS, Math.max(MIN_COLS, Math.ceil(Math.sqrt(Math.max(1, n) * 1.5))));
}

/** Seat (row, col) → room-relative position. Rows run from the front strip
 *  (+z, the door / camera side) toward the back wall (−z). */
export function seatPosition(row: number, col: number, cols: number, rowsTotal: number): Seat {
  const x = (col - (cols - 1) / 2) * SEAT_X;
  const depth = roomDepth(rowsTotal);
  const frontEdge = depth / 2 - FRONT_STRIP;
  const z = frontEdge - ROOM_PAD / 2 - row * SEAT_Z;
  return { row, col, x, z };
}

export function roomWidth(cols: number): number {
  return (cols - 1) * SEAT_X + 2 * ROOM_PAD;
}

export function roomDepth(rowsTotal: number): number {
  return Math.max(1, rowsTotal) * SEAT_Z + ROOM_PAD + FRONT_STRIP;
}

/** Rooms per floor row for `n` rooms (a squarish campus, wider than deep). */
export function campusColumns(n: number): number {
  if (n <= 1) return 1;
  return Math.min(6, Math.ceil(Math.sqrt(n * 1.4)));
}

// ── Model ───────────────────────────────────────────────────────────────────

function metaString(s: Session, key: string): string | null {
  const v = (s.meta as Record<string, unknown> | null)?.[key];
  return typeof v === 'string' && v.trim() ? v : null;
}

function canManageIn(ws: WorkspaceLike | undefined, workspaceId: string, canEditAgents: boolean): boolean {
  if (!canEditAgents) return false;
  if (workspaceId === SCRATCH_ID) return true;
  return !!ws && ws.my_role !== 'viewer';
}

/** Build the whole scene model. Archived rows are ignored; background rows
 *  are kept only while live; sessions of unknown workspaces are dropped. */
export function buildClassrooms(input: ModelInput): ClassroomModel {
  const statusOf = input.statusOf ?? (() => null);
  const needsYou = input.needsYou ?? (() => false);
  const canEdit = input.canEditAgents ?? true;
  const wsById = new Map<string, WorkspaceLike>();
  for (const w of input.workspaces) if (!w.archived && w.id !== SCRATCH_ID) wsById.set(w.id, w);

  type Draft = { student: Omit<Student, 'seat'>; rank: number; at: number };
  const front = new Map<string, Draft[]>();
  const back = new Map<string, Draft[]>();
  const seen = new Set<string>();
  for (const s of input.sessions) {
    if (s.archived || seen.has(s.id)) continue;
    seen.add(s.id);
    if (s.kind !== 'agent') continue; // connections (ssh/db) aren't students
    const isScratch = s.workspace_id === SCRATCH_ID;
    const ws = wsById.get(s.workspace_id);
    if (!ws && !isScratch) continue;
    const live = statusOf(s.id) ?? s.status;
    const st = sessionState(s, live, needsYou(s.id), { stale: input.stale });
    const fg = isForeground(s);
    const visual = visualFor(st.key);
    // An exited engine step is history, not a student.
    if (!fg && visual === 'away') continue;
    const student: Omit<Student, 'seat'> = {
      id: s.id,
      title: s.title?.trim() || 'Untitled session',
      provider: s.provider,
      initials: providerInitials(s.provider),
      colorToken: providerToken(s.provider),
      stateKey: st.key,
      stateLabel: st.label,
      visual,
      background: !fg,
      source: fg ? null : metaString(s, 'source'),
      workspaceId: s.workspace_id,
      workspaceName: isScratch ? 'No workspace' : (ws?.name ?? ''),
      lastActiveAt: s.last_active_at,
      cwd: s.cwd,
      branch: metaString(s, 'branch') ?? metaString(s, 'worktree_branch'),
      canManage: canManageIn(ws, s.workspace_id, canEdit),
    };
    const bucket = fg ? front : back;
    const list = bucket.get(s.workspace_id) ?? [];
    list.push({ student, rank: VISUAL_RANK[visual], at: Date.parse(s.last_active_at) || 0 });
    bucket.set(s.workspace_id, list);
  }

  // Rooms: the current workspace first, then busiest (needs-you, working,
  // seated), then by name; the scratch room only when it has students.
  type RoomDraft = { id: string; name: string; current: boolean; scratch: boolean; f: Draft[]; b: Draft[] };
  const drafts: RoomDraft[] = [];
  for (const w of wsById.values()) {
    drafts.push({ id: w.id, name: w.name, current: w.id === input.currentId, scratch: false, f: front.get(w.id) ?? [], b: back.get(w.id) ?? [] });
  }
  const sf = front.get(SCRATCH_ID) ?? [];
  const sb = back.get(SCRATCH_ID) ?? [];
  if (sf.length + sb.length > 0) drafts.push({ id: SCRATCH_ID, name: 'No workspace', current: input.currentId === null, scratch: true, f: sf, b: sb });
  const busy = (r: RoomDraft) => {
    let needs = 0;
    let work = 0;
    for (const d of [...r.f, ...r.b]) {
      if (d.student.visual === 'needs-you') needs++;
      else if (d.student.visual === 'working') work++;
    }
    return { needs, work, n: r.f.length + r.b.length };
  };
  const stats = new Map(drafts.map((r) => [r.id, busy(r)]));
  drafts.sort((a, b) => {
    if (a.current !== b.current) return a.current ? -1 : 1;
    const x = stats.get(a.id)!;
    const y = stats.get(b.id)!;
    return y.needs - x.needs || y.work - x.work || y.n - x.n || a.name.localeCompare(b.name);
  });

  const order = (a: Draft, b: Draft) => a.rank - b.rank || b.at - a.at || a.student.id.localeCompare(b.student.id);
  const rooms: Classroom[] = drafts.map((r) => {
    const f = [...r.f].sort(order);
    const b = [...r.b].sort(order);
    const seatedF = f.slice(0, MAX_FRONT);
    const seatedB = b.slice(0, MAX_BACK);
    const cols = Math.max(columnsFor(seatedF.length), Math.min(MAX_COLS, seatedB.length));
    const rows = Math.max(2, Math.ceil(seatedF.length / cols));
    const backRows = seatedB.length > 0 ? Math.ceil(seatedB.length / cols) : 0;
    const rowsTotal = rows + backRows;
    const place = (d: Draft, i: number, rowOffset: number): Student => ({
      ...d.student,
      seat: seatPosition(rowOffset + Math.floor(i / cols), i % cols, cols, rowsTotal),
    });
    const students = seatedF.map((d, i) => place(d, i, 0));
    const backRow = seatedB.map((d, i) => place(d, i, rows));
    const all = [...f, ...b];
    return {
      id: r.id,
      name: r.name,
      current: r.current,
      scratch: r.scratch,
      students,
      backRow,
      overflow: f.length - seatedF.length + (b.length - seatedB.length),
      cols,
      rows,
      x: 0,
      z: 0,
      width: roomWidth(cols),
      depth: roomDepth(rowsTotal),
      counts: {
        working: all.filter((d) => d.student.visual === 'working').length,
        needsYou: all.filter((d) => d.student.visual === 'needs-you').length,
        idle: all.filter((d) => d.student.visual === 'idle').length,
        away: all.filter((d) => d.student.visual === 'away').length,
        total: all.length,
      },
    };
  });

  // Campus: a uniform cell (the biggest room + gap), row-major, centred.
  const ccols = campusColumns(rooms.length);
  const crows = Math.max(1, Math.ceil(rooms.length / ccols));
  const cellW = Math.max(roomWidth(MIN_COLS), ...rooms.map((r) => r.width)) + ROOM_GAP;
  const cellD = Math.max(roomDepth(2), ...rooms.map((r) => r.depth)) + ROOM_GAP;
  rooms.forEach((r, i) => {
    const c = i % ccols;
    const row = Math.floor(i / ccols);
    r.x = (c - (ccols - 1) / 2) * cellW;
    r.z = (row - (crows - 1) / 2) * cellD;
  });

  const students = rooms.flatMap((r) => [...r.students, ...r.backRow]);
  return {
    rooms,
    students,
    width: ccols * cellW,
    depth: crows * cellD,
    total: students.length,
  };
}

/** World position of a student (room centre + seat). */
export function studentWorld(model: ClassroomModel, id: string): { x: number; z: number; room: Classroom } | null {
  for (const r of model.rooms) {
    const s = r.students.find((x) => x.id === id) ?? r.backRow.find((x) => x.id === id);
    if (s) return { x: r.x + s.seat.x, z: r.z + s.seat.z, room: r };
  }
  return null;
}

/** The door of a room (front wall, end side): where a kicked-out student
 *  leaves and the headmaster stands. */
export function doorPosition(room: Pick<Classroom, 'x' | 'z' | 'width' | 'depth'>): { x: number; z: number } {
  return { x: room.x + room.width / 2 - 0.9, z: room.z + room.depth / 2 };
}

/** Kick-out path: stand up at the desk → walk to the door → out (3 waypoints). */
export function exitPath(from: { x: number; z: number }, door: { x: number; z: number }): { x: number; z: number }[] {
  return [
    { x: from.x, z: from.z },
    { x: door.x, z: door.z - 0.4 },
    { x: door.x, z: door.z + 1.6 },
  ];
}

/** Point along a polyline at `t` ∈ [0, 1] (by length). */
export function alongPath(path: { x: number; z: number }[], t: number): { x: number; z: number } {
  if (path.length === 0) return { x: 0, z: 0 };
  if (path.length === 1 || t <= 0) return { ...path[0] };
  const segs: number[] = [];
  let total = 0;
  for (let i = 1; i < path.length; i++) {
    const d = Math.hypot(path[i].x - path[i - 1].x, path[i].z - path[i - 1].z);
    segs.push(d);
    total += d;
  }
  if (t >= 1 || total === 0) return { ...path[path.length - 1] };
  let left = t * total;
  for (let i = 0; i < segs.length; i++) {
    if (left <= segs[i]) {
      const k = segs[i] === 0 ? 0 : left / segs[i];
      return { x: path[i].x + (path[i + 1].x - path[i].x) * k, z: path[i].z + (path[i + 1].z - path[i].z) * k };
    }
    left -= segs[i];
  }
  return { ...path[path.length - 1] };
}

// ── Copy ────────────────────────────────────────────────────────────────────

/** "2 working · 1 needs you · 3 idle" (empty → "Empty"). */
export function roomSummary(r: Pick<Classroom, 'counts'>): string {
  const c = r.counts;
  if (c.total === 0) return 'Empty';
  const parts: string[] = [];
  if (c.needsYou) parts.push(c.needsYou === 1 ? '1 needs you' : `${c.needsYou} need you`);
  if (c.working) parts.push(`${c.working} working`);
  if (c.idle) parts.push(`${c.idle} idle`);
  if (c.away) parts.push(`${c.away} away`);
  return parts.join(' · ');
}

/** Last path segments of a cwd (`…/otto_os/ui`). */
export function shortPath(cwd: string, keep = 2): string {
  const parts = cwd.split('/').filter(Boolean);
  if (parts.length <= keep) return cwd || '';
  return `…/${parts.slice(-keep).join('/')}`;
}

/** Tooltip lines for a student — the same text the accessible list reads. */
export function tooltipLines(s: Student, ago: string): { title: string; lines: string[] } {
  const lines = [
    `${providerLabel(s.provider)} · ${s.stateLabel}`,
    s.background && s.source ? `${s.workspaceName} · back row (${s.source.replace(/[_-]+/g, ' ')})` : s.workspaceName,
  ];
  if (ago) lines.push(`Last active ${ago}`);
  const where = [s.cwd ? shortPath(s.cwd) : '', s.branch ? `⎇ ${s.branch}` : ''].filter(Boolean).join(' · ');
  if (where) lines.push(where);
  return { title: s.title, lines };
}

/** One-line accessible label for a student button. */
export function studentAriaLabel(s: Student, ago: string): string {
  const t = tooltipLines(s, ago);
  return `${t.title} — ${t.lines.join(', ')}`;
}
