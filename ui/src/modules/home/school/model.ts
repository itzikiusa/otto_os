// Otto School — the PURE layout half of the Home "Classrooms" widget.
// Sessions + workspaces in, a school out: one corridor, one door per
// workspace, and behind every door a classroom whose desks seat that
// workspace's agent sessions as kids. No three.js, no DOM, no runes — node:test
// covers it (unit/school-model.test.ts); scene.ts only draws what this decides
// and life.ts only moves characters along the lanes defined here.
//
// World frame (metres, Y up — matches ui/assets-src/school/CONTRACT.md):
//
//            north (−Z)
//   ┌──────── room 0 ────────┐┌──────── room 1 ────────┐
//   │  board        (front)  ││                        │
//   │  desks … kids face −Z  ││                        │
//   │                  [door]││                  [door]│   ← back wall (z = ROOM_BACK_Z)
//   ═══════════════════[door]════════════════════[door]═══  corridor north wall (z = −CORRIDOR_HALF)
//      corridor (runs along +X, x ∈ [0, corridorLength])
//   ═══════════════════════════════════════════════════════  lockers (z = +CORRIDOR_HALF)
//
// A room hangs north of its corridor door, door in its back wall's east-most
// segment. Rooms may be wider than the door pitch and overlap their
// neighbours on paper — only the room you are in is ever built.

import { sessionState, type SessionStateKey } from '../../../lib/status.ts';
import { isForeground, SCRATCH_ID } from '../../../lib/stores/sessionBuckets.ts';
import type { Session } from '../../../lib/api/types';

// ── Geometry (metres) ──────────────────────────────────────────────────────

/** Modular wall / floor segment (every kit piece is 2 m wide). */
export const SEG = 2;
/** Half the corridor's width. */
export const CORRIDOR_HALF = 2;
/** Wall thickness of the kit's wall pieces. */
export const WALL_T = 0.15;
/** Inside face of a room's back wall (behind the corridor's north wall). */
export const ROOM_BACK_Z = -CORRIDOR_HALF - 2 * WALL_T;
/** Door pitch along the corridor (one door per workspace). */
export const DOOR_PITCH = 6;
/** First door's x. */
export const DOOR0_X = 3;
/** Desk pitch across / front-to-back. */
export const COL_PITCH = 1.7;
export const ROW_PITCH = 1.9;
/** Kid (chair origin) → workstation origin, toward the board. */
export const DESK_OFFSET = 0.55;
/** Board wall → first row's kid. */
export const FRONT_STRIP = 3.2;
/** Last row's kid → back wall (lane + detention bench + door clearance). */
export const BACK_STRIP = 3.0;
/** Wall → side aisle centre. */
export const AISLE_IN = 0.72;
/** Kid → the lane behind its row (between the chair back and the next desk). */
export const LANE_BEHIND = 0.65;
/** Desks per row bounds. */
export const MIN_COLS = 3;
export const MAX_COLS = 6;
/** Seats per room before the rest collapse into a "+N" count. */
export const MAX_FRONT = 36;
/** Engine back-row seats per room (live engines only). */
export const MAX_BACK = 6;
/** Detention bench slots (most recently archived first). */
export const BENCH_SLOTS = 3;

// ── Types ──────────────────────────────────────────────────────────────────

/** How a kid behaves at the desk. */
export type Pose = 'working' | 'needs-you' | 'idle' | 'away' | 'stale';
/** Character file key (`kid-<key>.glb`). */
export type CharacterKey = 'claude' | 'codex' | 'grok' | 'agy' | 'shell' | 'custom';

export interface P2 {
  x: number;
  z: number;
}

export interface Kid {
  id: string;
  title: string;
  provider: string;
  character: CharacterKey;
  stateKey: SessionStateKey;
  stateLabel: string;
  pose: Pose;
  /** Engine-spawned (workflow / swarm / review…) — sits in the back row. */
  background: boolean;
  source: string | null;
  workspaceId: string;
  workspaceName: string;
  lastActiveAt: string;
  cwd: string;
  branch: string | null;
  /** The caller may delete / archive it (role in its workspace). */
  canManage: boolean;
  /** Archived — sits on the detention bench (`seat` is a bench slot). */
  detention: boolean;
  row: number;
  col: number;
  /** Chair origin, ROOM-local (room centre = 0,0; the board is at −Z). */
  seat: P2;
}

export interface Room {
  id: string;
  name: string;
  current: boolean;
  scratch: boolean;
  /** Seated kids: front rows then the engine back row. */
  kids: Kid[];
  /** Archived sessions on the detention bench (≤ BENCH_SLOTS). */
  bench: Kid[];
  overflow: number;
  cols: number;
  rows: number;
  backRows: number;
  /** Footprint (multiples of SEG) and centre in world space. */
  width: number;
  depth: number;
  cx: number;
  cz: number;
  /** World x of this room's corridor door. */
  doorX: number;
  counts: { working: number; needsYou: number; idle: number; away: number; total: number; detention: number };
}

export interface School {
  rooms: Room[];
  kids: Kid[];
  corridorLength: number;
  total: number;
}

export interface WorkspaceLike {
  id: string;
  name: string;
  my_role?: 'viewer' | 'editor' | 'admin' | string | null;
  archived?: boolean;
}

export interface SchoolInput {
  workspaces: readonly WorkspaceLike[];
  currentId: string | null;
  sessions: readonly Session[];
  /** Recently archived sessions (detention bench), newest first. */
  archived?: readonly Session[];
  statusOf?: (id: string) => string | null | undefined;
  needsYou?: (id: string) => boolean;
  stale?: boolean;
  canEditAgents?: boolean;
}

// ── Provider identity ──────────────────────────────────────────────────────

const KNOWN: Record<string, CharacterKey> = { claude: 'claude', codex: 'codex', grok: 'grok', agy: 'agy', gemini: 'agy', antigravity: 'agy', shell: 'shell', bash: 'shell', zsh: 'shell' };
const LABELS: Record<CharacterKey, string> = { claude: 'Claude', codex: 'Codex', grok: 'Grok', agy: 'Antigravity', shell: 'Shell', custom: '' };

export function characterFor(provider: string): CharacterKey {
  return KNOWN[(provider ?? '').toLowerCase()] ?? 'custom';
}

export function providerLabel(provider: string): string {
  const p = (provider ?? '').toLowerCase();
  const l = LABELS[characterFor(p)];
  return l || (p ? p[0].toUpperCase() + p.slice(1) : 'Unknown');
}

export function poseFor(key: SessionStateKey): Pose {
  switch (key) {
    case 'working':
      return 'working';
    // `running` = the process is alive but producing nothing (lib/status.ts):
    // for a kid that's sitting idle, not typing.
    case 'needs-you':
      return 'needs-you';
    case 'running':
    case 'idle':
      return 'idle';
    case 'stale':
      return 'stale';
    default:
      return 'away';
  }
}

// ── Layout math ────────────────────────────────────────────────────────────

/** Desks per row for `n` kids: a little wider than deep, within bounds. */
export function columnsFor(n: number): number {
  return Math.min(MAX_COLS, Math.max(MIN_COLS, Math.ceil(Math.sqrt(Math.max(1, n) * 1.3))));
}

const roundSeg = (v: number): number => Math.ceil(v / SEG - 1e-9) * SEG;

export function roomWidth(cols: number): number {
  // Desks 1.2 m wide + a 1.3 m aisle on both sides.
  return Math.max(4 * SEG, roundSeg((cols - 1) * COL_PITCH + 1.2 + 2 * 1.3));
}

export function roomDepth(rowsTotal: number): number {
  return Math.max(4 * SEG, roundSeg(FRONT_STRIP + (Math.max(1, rowsTotal) - 1) * ROW_PITCH + BACK_STRIP));
}

/** Chair origin of (row, col), room-local. */
export function seatAt(row: number, col: number, cols: number, depth: number): P2 {
  return { x: (col - (cols - 1) / 2) * COL_PITCH, z: -depth / 2 + FRONT_STRIP + row * ROW_PITCH };
}

/** Lane (walkway) z behind a row, room-local. */
export function laneZ(room: Pick<Room, 'depth'>, row: number): number {
  return -room.depth / 2 + FRONT_STRIP + row * ROW_PITCH + LANE_BEHIND;
}

/** The lane between the board and the first row. */
export function frontLaneZ(room: Pick<Room, 'depth'>): number {
  return -room.depth / 2 + 1.7;
}

/** The lane in front of the back wall. */
export function backLaneZ(room: Pick<Room, 'depth'>): number {
  return room.depth / 2 - 1.1;
}

/** Side aisle x (−1 = west / windows, +1 = east / bookshelf). */
export function aisleX(room: Pick<Room, 'width'>, side: -1 | 1): number {
  return side * (room.width / 2 - AISLE_IN);
}

/** Inside the door (room-local) and just outside it in the corridor. */
export function doorInside(room: Pick<Room, 'width' | 'depth'>): P2 {
  return { x: room.width / 2 - SEG / 2, z: room.depth / 2 - 0.6 };
}
export function doorOutside(room: Pick<Room, 'width' | 'depth'>): P2 {
  return { x: room.width / 2 - SEG / 2, z: room.depth / 2 + 2 * WALL_T + 1.4 };
}

/** Detention bench slot `i` (room-local seat origin): the front corner by
 *  the board, in full view of the class — kids on it face the room (+Z). */
export function benchSlot(room: Pick<Room, 'width' | 'depth'>, i: number): P2 {
  return { x: room.width / 2 - 1.6 + (i - 1) * 0.55, z: -room.depth / 2 + 1.0 };
}

/** Room-local → world. */
export function toWorld(room: Pick<Room, 'cx' | 'cz'>, p: P2): P2 {
  return { x: room.cx + p.x, z: room.cz + p.z };
}

/**
 * Walk path between two room-local points along the lanes: leave `a` to its
 * lane, follow it to the side aisle that makes the shorter trip, walk the
 * aisle to `b`'s lane, then in to `b`. Every segment is axis-aligned and
 * stays in walkways, so nobody walks through a desk.
 */
export function lanePath(room: Pick<Room, 'width' | 'depth'>, a: P2, aLane: number, b: P2, bLane: number): P2[] {
  const pts: P2[] = [a, { x: a.x, z: aLane }];
  if (Math.abs(aLane - bLane) > 1e-6) {
    const w = aisleX(room, -1);
    const e = aisleX(room, 1);
    const side = Math.abs(a.x - w) + Math.abs(b.x - w) <= Math.abs(a.x - e) + Math.abs(b.x - e) ? w : e;
    pts.push({ x: side, z: aLane }, { x: side, z: bLane });
  }
  pts.push({ x: b.x, z: bLane }, b);
  // Drop zero-length hops.
  return pts.filter((p, i) => i === 0 || Math.hypot(p.x - pts[i - 1].x, p.z - pts[i - 1].z) > 1e-3);
}

export function pathLength(path: readonly P2[]): number {
  let d = 0;
  for (let i = 1; i < path.length; i++) d += Math.hypot(path[i].x - path[i - 1].x, path[i].z - path[i - 1].z);
  return d;
}

/** Point `dist` metres along `path`, plus the heading of that segment
 *  (three.js Y rotation; 0 = facing +Z). */
export function along(path: readonly P2[], dist: number): { p: P2; heading: number; done: boolean } {
  if (path.length === 0) return { p: { x: 0, z: 0 }, heading: 0, done: true };
  let left = Math.max(0, dist);
  for (let i = 1; i < path.length; i++) {
    const dx = path[i].x - path[i - 1].x;
    const dz = path[i].z - path[i - 1].z;
    const seg = Math.hypot(dx, dz);
    if (left <= seg) {
      const k = seg === 0 ? 0 : left / seg;
      return { p: { x: path[i - 1].x + dx * k, z: path[i - 1].z + dz * k }, heading: Math.atan2(dx, dz), done: false };
    }
    left -= seg;
  }
  const n = path.length;
  const last = path[n - 1];
  const prev = path[Math.max(0, n - 2)];
  return { p: { ...last }, heading: n > 1 ? Math.atan2(last.x - prev.x, last.z - prev.z) : 0, done: true };
}

// ── Model ──────────────────────────────────────────────────────────────────

function metaString(s: Session, key: string): string | null {
  const v = (s.meta as Record<string, unknown> | null)?.[key];
  return typeof v === 'string' && v.trim() ? v : null;
}

function canManageIn(w: WorkspaceLike | undefined, workspaceId: string, canEditAgents: boolean): boolean {
  if (!canEditAgents) return false;
  if (workspaceId === SCRATCH_ID) return true;
  return !!w && w.my_role !== 'viewer';
}

type Draft = Omit<Kid, 'row' | 'col' | 'seat'> & { at: number; born: number };

/** Build the whole school. Background rows are kept only while live; an
 *  exited engine step is history, not a kid. Sessions of unknown workspaces
 *  are dropped; scratch sessions get a "Scratch" room. */
export function buildSchool(input: SchoolInput): School {
  const statusOf = input.statusOf ?? (() => null);
  const needsYou = input.needsYou ?? (() => false);
  const canEdit = input.canEditAgents ?? true;
  const wsById = new Map<string, WorkspaceLike>();
  for (const w of input.workspaces) if (!w.archived && w.id !== SCRATCH_ID) wsById.set(w.id, w);

  const front = new Map<string, Draft[]>();
  const back = new Map<string, Draft[]>();
  const bench = new Map<string, Draft[]>();
  const seen = new Set<string>();

  const draft = (s: Session, detention: boolean): Draft | null => {
    if (s.kind !== 'agent') return null; // ssh / db connections aren't kids
    const isScratch = s.workspace_id === SCRATCH_ID;
    const w = wsById.get(s.workspace_id);
    if (!w && !isScratch) return null;
    const live = detention ? s.status : (statusOf(s.id) ?? s.status);
    const st = sessionState(s, live, !detention && needsYou(s.id), { stale: input.stale });
    const pose = detention ? 'away' : poseFor(st.key);
    return {
      id: s.id,
      title: s.title?.trim() || 'Untitled session',
      provider: s.provider,
      character: characterFor(s.provider),
      stateKey: st.key,
      stateLabel: detention ? 'In detention (archived)' : st.label,
      pose,
      background: !isForeground(s),
      source: metaString(s, 'source'),
      workspaceId: s.workspace_id,
      workspaceName: isScratch ? 'Scratch' : (w?.name ?? ''),
      lastActiveAt: s.last_active_at ?? s.created_at ?? '',
      cwd: s.cwd ?? '',
      branch: metaString(s, 'branch') ?? metaString(s, 'worktree_branch'),
      canManage: canManageIn(w, s.workspace_id, canEdit),
      detention,
      at: Date.parse(s.last_active_at ?? s.created_at ?? '') || 0,
      born: Date.parse(s.created_at ?? '') || 0,
    };
  };
  const push = (m: Map<string, Draft[]>, d: Draft) => {
    const l = m.get(d.workspaceId);
    if (l) l.push(d);
    else m.set(d.workspaceId, [d]);
  };

  for (const s of input.sessions) {
    if (s.archived || seen.has(s.id)) continue;
    seen.add(s.id);
    const d = draft(s, false);
    if (!d) continue;
    if (d.background) {
      if (d.pose !== 'away') push(back, d);
    } else push(front, d);
  }
  for (const s of input.archived ?? []) {
    if (!s.archived || seen.has(s.id)) continue;
    seen.add(s.id);
    const d = draft(s, true);
    if (d && !d.background) push(bench, d);
  }

  // Rooms: every workspace (empty ones too — an empty classroom is still a
  // door), the current one first, then the busiest; scratch only when used.
  type RoomDraft = { id: string; name: string; current: boolean; scratch: boolean };
  const drafts: RoomDraft[] = [...wsById.values()].map((w) => ({ id: w.id, name: w.name, current: w.id === input.currentId, scratch: false }));
  if (front.has(SCRATCH_ID) || back.has(SCRATCH_ID)) drafts.push({ id: SCRATCH_ID, name: 'Scratch', current: input.currentId === SCRATCH_ID, scratch: true });
  const busy = (id: string) => {
    const all = [...(front.get(id) ?? []), ...(back.get(id) ?? [])];
    return { needs: all.filter((d) => d.pose === 'needs-you').length, work: all.filter((d) => d.pose === 'working').length, n: all.length };
  };
  const stats = new Map(drafts.map((r) => [r.id, busy(r.id)]));
  drafts.sort((a, b) => {
    if (a.current !== b.current) return a.current ? -1 : 1;
    const x = stats.get(a.id)!;
    const y = stats.get(b.id)!;
    return y.needs - x.needs || y.work - x.work || y.n - x.n || a.name.localeCompare(b.name);
  });

  // STABLE seating (oldest session first): a status change never shuffles
  // the room — the raised hand / ❗ shows who needs you, and life.ts walks a
  // kid to a new desk only when someone joins or leaves.
  const order = (a: Draft, b: Draft) => a.born - b.born || a.id.localeCompare(b.id);
  const strip = ({ at: _a, born: _b, ...k }: Draft) => k;
  const rooms: Room[] = drafts.map((r, i) => {
    const f = [...(front.get(r.id) ?? [])].sort(order);
    const b = [...(back.get(r.id) ?? [])].sort(order);
    const seatedF = f.slice(0, MAX_FRONT);
    const seatedB = b.slice(0, MAX_BACK);
    const cols = Math.max(columnsFor(seatedF.length), Math.min(MAX_COLS, seatedB.length));
    const rows = Math.max(2, Math.ceil(seatedF.length / cols));
    const backRows = seatedB.length > 0 ? Math.ceil(seatedB.length / cols) : 0;
    const width = roomWidth(cols);
    const depth = roomDepth(rows + backRows);
    const place = (d: Draft, idx: number, rowOffset: number): Kid => {
      const row = rowOffset + Math.floor(idx / cols);
      const col = idx % cols;
      return { ...strip(d), row, col, seat: seatAt(row, col, cols, depth) };
    };
    const doorX = DOOR0_X + i * DOOR_PITCH;
    const geo = { width, depth };
    const benchKids = [...(bench.get(r.id) ?? [])]
      .sort((a, z) => z.at - a.at)
      .slice(0, BENCH_SLOTS)
      .map((d, k): Kid => ({ ...strip(d), row: -1, col: k, seat: benchSlot(geo, k) }));
    const kids = [...seatedF.map((d, k) => place(d, k, 0)), ...seatedB.map((d, k) => place(d, k, rows))];
    const all = [...f, ...b];
    return {
      id: r.id,
      name: r.name,
      current: r.current,
      scratch: r.scratch,
      kids,
      bench: benchKids,
      overflow: f.length - seatedF.length + (b.length - seatedB.length),
      cols,
      rows,
      backRows,
      width,
      depth,
      cx: doorX + SEG / 2 - width / 2,
      cz: ROOM_BACK_Z - depth / 2,
      doorX,
      counts: {
        working: all.filter((d) => d.pose === 'working').length,
        needsYou: all.filter((d) => d.pose === 'needs-you').length,
        idle: all.filter((d) => d.pose === 'idle' || d.pose === 'stale').length,
        away: all.filter((d) => d.pose === 'away').length,
        total: all.length,
        detention: benchKids.length,
      },
    };
  });

  const kids = rooms.flatMap((r) => [...r.kids, ...r.bench]);
  return {
    rooms,
    kids,
    corridorLength: Math.max(3, rooms.length) * DOOR_PITCH,
    total: rooms.reduce((n, r) => n + r.counts.total, 0),
  };
}

// ── Copy ───────────────────────────────────────────────────────────────────

/** "1 needs you · 2 working · 3 idle" (empty → "Empty classroom"). */
export function roomSummary(r: Pick<Room, 'counts'>): string {
  const c = r.counts;
  if (c.total === 0) return 'Empty classroom';
  const parts: string[] = [];
  if (c.needsYou) parts.push(c.needsYou === 1 ? '1 needs you' : `${c.needsYou} need you`);
  if (c.working) parts.push(`${c.working} working`);
  if (c.idle) parts.push(`${c.idle} idle`);
  if (c.away) parts.push(`${c.away} away`);
  return parts.join(' · ');
}

export function shortPath(cwd: string, keep = 2): string {
  const parts = cwd.split('/').filter(Boolean);
  if (parts.length <= keep) return cwd || '';
  return `…/${parts.slice(-keep).join('/')}`;
}

export function sourceLabel(source: string | null | undefined): string {
  return source ? source.replace(/[_-]+/g, ' ').trim() : 'engine';
}

/** Card / list copy for a kid. */
export function kidLines(k: Kid, ago: string): string[] {
  const lines = [`${providerLabel(k.provider)} · ${k.stateLabel}`];
  if (k.background) lines.push(`Back row · ${sourceLabel(k.source)}`);
  if (ago) lines.push(`Last active ${ago}`);
  const where = [k.cwd ? shortPath(k.cwd) : '', k.branch ? `⎇ ${k.branch}` : ''].filter(Boolean).join(' · ');
  if (where) lines.push(where);
  return lines;
}

export function kidAriaLabel(k: Kid, ago: string): string {
  return [k.title, ...kidLines(k, ago)].join(', ');
}
