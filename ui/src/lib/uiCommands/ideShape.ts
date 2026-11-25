// Pure shaping for the `otto.ui_ide_debug_*` results (./ide.ts): what an agent
// reads at a stop, capped (a Stop stays ≤ 16 KB of JSON) and keyed by NAME —
// locals are `{name: value}` and inspected paths are keyed by their last
// segment — so the bridge's redaction (`otto_core::redact`, run on every UI
// command result) masks `password`, `token`, `dsn`, … by key. What the
// PERSON sees in the IDE is never redacted. Kept free of stores for unit tests
// (ui/unit/uiCommandsIde.test.ts).

import type { ApiResponse, IdeDebugOutput, IdeDebugSessionSnapshot, IdeVariable } from '../api/types';
import type { AgentDbRead } from './ideDbWatchPort';
import type { IdeAgentBreakpoint } from './idePort';

export const STOP_MAX_CHARS = 16 * 1024;
const LOCALS_MAX = 40;
const VALUE_MAX = 200;
const FRAMES_MAX = 10;
const LINE_MAX = 200;
const TAIL_LINES = 40;
const TAIL_CHARS = 4096;
export const SOURCE_RADIUS = 6;
export const INSPECT_VALUE_MAX = 2048;
export const EVAL_MAX = 4096;
export const BODY_MAX = 16 * 1024;

/** What the handlers remember per agent session between calls. */
export interface DebugMemory {
  debugId: string;
  /** The project root the session runs in (paths are shown relative to it). */
  root: string;
  /** `stop_seq` and state of the last result this agent got. */
  stopSeq: number;
  state: string;
  /** Debug-console output the agent has seen, by the entries' `seq`. */
  outSeq: number;
  /** Locals of the last returned stop (for `changed`). */
  locals: Record<string, string>;
  /** The agent's own action moved execution (start / continue / step /
   *  request): the stop or run that follows is not the person's. */
  expectMove: boolean;
  /** A debug request's response that arrived after that call returned. */
  late?: Record<string, unknown> | null;
}

export function newMemory(debugId: string, root: string, outSeq = 0): DebugMemory {
  return { debugId, root, stopSeq: 0, state: 'starting', outSeq, locals: {}, expectMove: true, late: null };
}

export function clip(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

/** `file` relative to `root` (absolute when outside it). */
export function relPath(root: string, file: string | null | undefined): string {
  const f = file ?? '';
  const r = root.replace(/\/+$/, '');
  return r && f.startsWith(`${r}/`) ? f.slice(r.length + 1) : f;
}

/** The person stepped / continued / restarted since the agent's last result. */
export function userMoved(snap: IdeDebugSessionSnapshot, mem: DebugMemory): boolean {
  return !mem.expectMove && ((snap.stop_seq ?? 0) !== mem.stopSeq || snap.state !== mem.state);
}

/** The first scope's variables as `{name: value}` (≤ 40, values ≤ 200 chars),
 *  plus the `ref` of each structured one (expand it with ide_debug_inspect). */
export function localsOf(snap: IdeDebugSessionSnapshot): { locals: Record<string, string>; refs: Record<string, number>; more: number } {
  const vars = snap.scopes[0]?.variables ?? [];
  const locals: Record<string, string> = {};
  const refs: Record<string, number> = {};
  let n = 0;
  for (const v of vars) {
    if (v.name in locals) continue; // a shadowed outer variable: the inner one wins
    if (n === LOCALS_MAX) break;
    locals[v.name] = clip(v.value, VALUE_MAX);
    if (v.variables_reference > 0) refs[v.name] = v.variables_reference;
    n += 1;
  }
  return { locals, refs, more: Math.max(0, vars.length - n) };
}

/** Names whose value differs from the previous stop (new names count once
 *  there was a previous stop). */
export function changedNames(prev: Record<string, string>, now: Record<string, string>): string[] {
  if (Object.keys(prev).length === 0) return [];
  return Object.keys(now).filter((k) => prev[k] !== now[k]);
}

/** Console lines after `afterSeq` (≤ 40 lines / 4 KB, the newest kept) and
 *  the seq they reach. */
export function outputTail(output: readonly IdeDebugOutput[], afterSeq: number): { lines: string[]; seq: number } {
  let seq = afterSeq;
  let text = '';
  output.forEach((o, i) => {
    const s = o.seq ?? i + 1;
    if (s <= afterSeq) return;
    seq = Math.max(seq, s);
    text += o.text;
  });
  if (text.length > TAIL_CHARS) text = `…${text.slice(text.length - TAIL_CHARS)}`;
  const lines = text.split('\n');
  if (lines[lines.length - 1] === '') lines.pop();
  return { lines: lines.slice(-TAIL_LINES).map((l) => clip(l, LINE_MAX)), seq };
}

/** The exit code a terminated session printed ("Process exited with code 1"). */
export function exitCodeOf(output: readonly IdeDebugOutput[]): number | null {
  for (let i = output.length - 1; i >= Math.max(0, output.length - 20); i--) {
    const m = /exited with (?:code|status) (-?\d+)/i.exec(output[i].text);
    if (m) return Number(m[1]);
  }
  return null;
}

export function shapeBreakpoints(bps: readonly IdeAgentBreakpoint[], root: string) {
  return bps.map((b) => ({
    file: relPath(root, b.file),
    line: b.line,
    verified: b.verified === true,
    ...(b.message ? { message: clip(b.message, 300) } : {}),
  }));
}

/** ±6 lines around `line` (`lines` starts at line `from`), the paused one marked. */
export function sourceWindow(lines: readonly string[], from: number, line: number): string[] {
  return lines.map((l, i) => `${from + i === line ? '→' : ' '} ${from + i} | ${clip(l, LINE_MAX)}`);
}

export interface StopExtras {
  breakpoints: readonly IdeAgentBreakpoint[];
  /** Lines from `sourceFrom` (see {@link sourceWindow}). */
  source?: readonly string[];
  sourceFrom?: number;
  db?: unknown;
  request?: unknown;
}

/** What the agent reads about the session now: a Stop when paused, else the
 *  state and the new output. Returns the memory to keep after it. */
export function shapeResult(snap: IdeDebugSessionSnapshot, mem: DebugMemory, x: StopExtras): { result: Record<string, unknown>; mem: DebugMemory } {
  const moved = userMoved(snap, mem);
  const tail = outputTail(snap.output, mem.outSeq);
  const base: Record<string, unknown> = { debug_id: snap.id, state: snap.state, user_moved: moved };
  const next: DebugMemory = { ...mem, stopSeq: snap.stop_seq ?? 0, state: snap.state, outSeq: tail.seq, expectMove: false };
  const bps = shapeBreakpoints(x.breakpoints, mem.root);
  if (snap.state !== 'paused') {
    if (snap.state === 'running' || snap.state === 'starting') next.expectMove = mem.expectMove || moved;
    const result: Record<string, unknown> = { ...base, output_tail: tail.lines, breakpoints: bps };
    if (snap.state === 'terminated') result.exit_code = exitCodeOf(snap.output);
    if (snap.error) result.error = clip(snap.error, 2000);
    if (x.request) result.request = x.request;
    return { result, mem: next };
  }
  const { locals, refs, more } = localsOf(snap);
  next.locals = locals;
  const top = snap.stack_frames[0];
  const file = snap.paused_file ?? top?.path ?? '';
  const line = snap.paused_line ?? top?.line ?? 0;
  const hit = snap.breakpoints
    .filter((b) => b.path === file && b.line === line)
    .map((b) => ({ file: relPath(mem.root, b.path), line: b.line }));
  const result: Record<string, unknown> = {
    ...base,
    stop_seq: snap.stop_seq ?? 0,
    reason: snap.stopped_reason ?? null,
    location: { file: relPath(mem.root, file), line, function: top?.name ?? null },
    ...(x.source && x.sourceFrom ? { source: sourceWindow(x.source, x.sourceFrom, line) } : {}),
    frames: snap.stack_frames.slice(0, FRAMES_MAX).map((f) => ({ id: f.id, name: f.name, file: relPath(mem.root, f.path), line: f.line })),
    locals,
    ...(Object.keys(refs).length ? { refs } : {}),
    ...(more ? { more_locals: more } : {}),
    changed: changedNames(mem.locals, locals),
    threads: { count: snap.threads.length, stopped: snap.active_thread_id ?? null },
    output_tail: tail.lines,
    breakpoints: bps,
    breakpoints_hit: hit,
    ...(x.db !== undefined ? { db: x.db } : {}),
    ...(x.request ? { request: x.request } : {}),
  };
  return { result: fitStop(result), mem: next };
}

/** Trim a Stop to {@link STOP_MAX_CHARS}: source first, then frames, the
 *  console tail and the locals' values. */
export function fitStop(r: Record<string, unknown>): Record<string, unknown> {
  const size = (): number => JSON.stringify(r).length;
  if (size() <= STOP_MAX_CHARS) return r;
  const steps: (() => void)[] = [
    () => void delete r.source,
    () => (r.frames = (r.frames as unknown[]).slice(0, 3)),
    () => (r.output_tail = (r.output_tail as string[]).slice(-10)),
    () => (r.locals = Object.fromEntries(Object.entries(r.locals as Record<string, string>).map(([k, v]) => [k, clip(v, 60)]))),
    () => (r.locals = Object.fromEntries(Object.entries(r.locals as Record<string, string>).slice(0, 15))),
    () => (r.output_tail = []),
  ];
  for (const step of steps) {
    step();
    if (size() <= STOP_MAX_CHARS) break;
  }
  return r;
}

/** `cfg.DB.Host`, `items[0]`, `req.Header["X-Id"]` → its segments. */
export function pathSegments(path: string): string[] {
  const out: string[] = [];
  for (const m of path.matchAll(/\[([^\]]*)\]|([^.[\]]+)/g)) {
    const seg = (m[1] ?? m[2] ?? '').trim().replace(/^["'](.*)["']$/, '$1');
    if (seg) out.push(seg);
  }
  return out;
}

/** The child named `seg` (a field, an index `[0]`, a map key `["k"]`),
 *  looking through a lone pointer dereference (`*r`). */
export function childNamed(children: readonly IdeVariable[], seg: string): IdeVariable | null {
  const names = [seg, `[${seg}]`, `"${seg}"`, `["${seg}"]`];
  return children.find((c) => names.includes(c.name)) ?? null;
}

/** `name`, made unique among `taken` (`x`, `x#2`: a shadowed variable). */
export function uniqueKey(taken: Set<string>, name: string): string {
  let key = name;
  for (let i = 2; taken.has(key); i++) key = `${name}#${i}`;
  taken.add(key);
  return key;
}

/** One variable as the agent reads it. */
export function shapeVariable(v: IdeVariable, max = INSPECT_VALUE_MAX) {
  return {
    value: clip(v.value, max),
    ...(v.var_type ? { type: v.var_type } : {}),
    ...(v.variables_reference > 0 ? { ref: v.variables_reference } : {}),
  };
}

/** An HTTP response for the agent: headers as an object (so header names
 *  redact by key), the body ≤ 16 KB. */
export function shapeResponse(resp: ApiResponse): Record<string, unknown> {
  const headers: Record<string, string> = {};
  for (const h of resp.headers.slice(0, 100)) {
    const k = h.key.toLowerCase();
    headers[k] = k in headers ? `${headers[k]}, ${h.value}` : h.value;
  }
  const body = resp.body ?? '';
  return {
    status: resp.status,
    status_text: resp.status_text,
    duration_ms: resp.duration_ms,
    headers,
    body: body.length > BODY_MAX ? body.slice(0, BODY_MAX) : body,
    body_truncated: body.length > BODY_MAX || !!resp.truncated || !!resp.too_large,
  };
}

/** The start confirm's text: exactly what will run. */
export function startConfirmText(p: {
  label: string; command: string; cwd: string; build_flags: readonly string[];
  env: Record<string, string>; breakpoints: number; root: string;
}): string {
  const env = Object.entries(p.env);
  return [
    `Debug ${p.label}`,
    `Command: ${p.command}`,
    `Working directory: ${relPath(p.root, p.cwd) || '.'}`,
    p.build_flags.length ? `Build flags: ${p.build_flags.join(' ')}` : '',
    env.length ? `Extra environment: ${env.map(([k, v]) => `${k}=${clip(v, 80)}`).join(', ')}` : 'Environment: the project environment',
    `Breakpoints: ${p.breakpoints}`,
    'It runs on your Mac as you, outside the agent’s sandbox.',
  ]
    .filter(Boolean)
    .join('\n');
}

/** DB Watch rows for the agent: per object the counts and ≤ `maxRows` rows,
 *  each row an object keyed by COLUMN (so it masks by name) — a changed row
 *  lists its changed cells only. */
export function shapeDbRead(read: AgentDbRead | null, maxRows: number): Record<string, unknown> {
  if (!read) return { objects: [] };
  return {
    from_seq: read.from_seq,
    to_seq: read.to_seq,
    objects: read.items.map((i) => ({
      object: i.object,
      added: i.added_count,
      changed: i.modified_count,
      removed: i.removed_count,
      rows: i.diffs.slice(0, maxRows).map((d) =>
        d.kind === 'modified'
          ? { kind: d.kind, key: d.key, changes: Object.fromEntries(d.changes.map((c) => [c.column, { before: c.before, after: c.after }])) }
          : { kind: d.kind, key: d.key, row: Object.fromEntries(i.columns.map((c, n) => [c, d.row[n]])) },
      ),
      more: i.more_count + Math.max(0, i.diffs.length - maxRows),
      window: i.window_note,
      ...(i.error ? { error: i.error } : {}),
    })),
  };
}
