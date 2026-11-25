// Agent UI control — IDE agent debugging (`otto.ui_ide_debug_*`,
// docs/features/agent-ui-control.md §5 "IDE"). Only when the person asked an
// agent to "use Otto to debug …": the agent drives the IDE's OWN debugger in
// the person's window — the same store actions as the gutter Debug button,
// the breakpoints gutter and the toolbar — so everything it does is on screen.
//
// Risk (docs/contracts/ui-commands.json): `start` and `evaluate` are
// `execute` — they run code on the person's Mac outside the agent's sandbox,
// so each one is an attributed confirm showing exactly what runs, never
// remembered. `request` is a rememberable `local_write` (loopback only, the
// daemon refuses anything else). The rest read or step the session.
//
// Scope rule: control / breakpoints / inspect / evaluate / request / db act
// only on the debug session THIS agent session started (`idePort.agentDebugId`).
// The person's own clicks stay theirs: the next result says `user_moved` and
// a moving action is not done. Results are shaped in ./ideShape.ts (keyed by
// name, capped) and redacted by the bridge before the agent sees them.

import { api } from '../api/client';
import type { ApiResponse, IdeDbWatchDetectResp, IdeDebugPortResp, IdeDebugSessionSnapshot, IdeVariable } from '../api/types';
import { agentName } from '../stores/uiControl.svelte';
import { ws } from '../stores/workspace.svelte';
import { registerUiCommands, UiCommandError, type UiCommandCtx } from '../uiCommands';
import type { IdeDbWatch, IdeDbWatchTargetRef } from '../api/types/ide-db-watch';
import { ideDbWatchPort, type IdeDbWatchAgentOps } from './ideDbWatchPort';
import { idePort, type IdeAgentBreakpointSpec, type IdeDebugCommand, type IdeDebugTarget, type IdePort } from './idePort';
import {
  childNamed, clip, EVAL_MAX, newMemory, pathSegments, relPath, shapeBreakpoints, shapeDbRead,
  shapeResponse, shapeResult, shapeVariable, SOURCE_RADIUS, startConfirmText, uniqueKey, userMoved, type DebugMemory,
} from './ideShape';
import { errText } from './pagePort';

/** Every long call returns before the bridge's deadline (120 s from dispatch,
 *  also the cap after a confirm) with this margin. */
const BUDGET_MS = 110_000;
/** A debug request outlives a breakpoint pause (the engine default is 60 s). */
const REQUEST_TIMEOUT_MS = 600_000;

const memory = new Map<string, DebugMemory>();

const ended = (s: IdeDebugSessionSnapshot): boolean => s.state === 'terminated' || s.state === 'error';
const basename = (p: string): string => p.replace(/\/+$/, '').split('/').pop() || p;
const declined = (what: string): UiCommandError => new UiCommandError('cancelled_by_user', `The user declined ${what}.`);

/** The project root: the agent's own folder or one inside it (D-008). */
export function checkRoot(raw: unknown, cwd: string): string {
  const home = cwd.replace(/\/+$/, '');
  const root = (typeof raw === 'string' && raw.trim() ? raw.trim() : home).replace(/\/+$/, '');
  if (!root.startsWith('/') || root.split('/').includes('..')) {
    throw new UiCommandError('invalid_args', '`root` must be an absolute project folder.');
  }
  if (!home || (root !== home && !root.startsWith(`${home}/`))) {
    throw new UiCommandError('forbidden', `You can debug your session’s folder (${home || 'unknown'}) or a folder inside it.`);
  }
  return root;
}

/** The debug session this agent started and still drives (the scope rule). */
function mine(ide: IdePort, ctx: UiCommandCtx): { snap: IdeDebugSessionSnapshot; mem: DebugMemory } {
  const snap = ide.debug;
  // The IDE lets go of a session once it ends; its last result is still the agent's.
  const last = memory.get(ctx.agent.session_id)?.debugId;
  const id = ide.agentDebugId(ctx.agent) ?? (snap && snap.id === last && ended(snap) ? last : null);
  if (!id || !snap || snap.id !== id) {
    throw new UiCommandError(
      'forbidden',
      'You have no debug session in this IDE — start one with ide_debug_start. A session the user started is theirs (they can use Ask Agent while paused).',
    );
  }
  let mem = memory.get(ctx.agent.session_id);
  if (!mem || mem.debugId !== id) {
    // Another IDE window ran the earlier calls: start from this snapshot.
    mem = { ...newMemory(id, ide.root), stopSeq: snap.stop_seq ?? 0, state: snap.state, expectMove: false };
    memory.set(ctx.agent.session_id, mem);
  }
  return { snap, mem };
}

/** Resolve with the first snapshot `done` accepts, else the current one when
 *  `ms` passes, `poll` says yes (every 500 ms) or `settled` settles. */
function waitFor(
  ide: IdePort,
  ms: number,
  signal: AbortSignal,
  done: (s: IdeDebugSessionSnapshot) => boolean,
  opts: { poll?: () => Promise<boolean>; settled?: Promise<unknown> } = {},
): Promise<IdeDebugSessionSnapshot | null> {
  return new Promise((resolve, reject) => {
    let over = false;
    let unsub = (): void => {};
    let poller: ReturnType<typeof setTimeout> | undefined;
    const cleanup = (): boolean => {
      if (over) return false;
      over = true;
      unsub();
      clearTimeout(timer);
      clearTimeout(poller);
      signal.removeEventListener('abort', onAbort);
      return true;
    };
    const finish = (s: IdeDebugSessionSnapshot | null = ide.debug): void => {
      if (cleanup()) resolve(s);
    };
    const onAbort = (): void => {
      if (cleanup()) reject(signal.reason);
    };
    const timer = setTimeout(() => finish(), Math.max(0, ms));
    const poll = opts.poll;
    const tick = (): void => {
      poller = setTimeout(() => void poll!().then((ok) => (ok ? finish() : over || tick()), () => over || tick()), 500);
    };
    if (poll) tick();
    signal.addEventListener('abort', onAbort, { once: true });
    if (signal.aborted) return onAbort();
    unsub = ide.onSnapshot((s) => {
      if (done(s)) finish(s);
    });
    opts.settled?.then(() => finish(), () => finish());
    const now = ide.debug;
    if (now && done(now)) finish(now);
  });
}

/** A stop newer than `after`, or the end of session `id`. */
const nextStop = (id: string, after: number) => (s: IdeDebugSessionSnapshot): boolean =>
  s.id === id && (ended(s) || (s.state === 'paused' && (s.stop_seq ?? 0) > after));

/** The IDE no longer shows session `id` (closed, or replaced by another). */
const gone = (ide: IdePort, id: string) => async (): Promise<boolean> => ide.debug?.id !== id;

/** The agent's result for `snap` (a Stop when paused); updates the memory. */
async function result(ide: IdePort, ctx: UiCommandCtx, snap: IdeDebugSessionSnapshot, request?: unknown): Promise<Record<string, unknown>> {
  const mem = memory.get(ctx.agent.session_id) ?? newMemory(snap.id, ide.root);
  let source: string[] | undefined;
  let sourceFrom: number | undefined;
  const line = snap.paused_line ?? 0;
  if (snap.state === 'paused' && snap.paused_file && line > 0) {
    sourceFrom = Math.max(1, line - SOURCE_RADIUS);
    source = await ide.readLines(snap.paused_file, sourceFrom, line + SOURCE_RADIUS).catch(() => undefined);
  }
  const db = snap.state === 'paused' ? await stopDb(snap.id) : undefined;
  const late = mem.late ?? undefined;
  const shaped = shapeResult(snap, mem, { breakpoints: ide.agentBreakpoints(ctx.agent), source, sourceFrom, db, request: request ?? late });
  // A late response rides on this result unless the call brought its own.
  memory.set(ctx.agent.session_id, { ...shaped.mem, late: request ? mem.late : null });
  return shaped.result;
}

/** The debug session a watch follows, or null. */
const boundTo = (w: IdeDbWatch | null | undefined): string | null =>
  w?.bind && 'debug_session_id' in w.bind ? w.bind.debug_session_id : null;

/** `Stop.db`: the bound watch's rows changed since the previous stop (≤ 5 per object). */
async function stopDb(debugId: string): Promise<unknown> {
  const db = ideDbWatchPort.peek();
  const w = db?.watch;
  if (!db || !w || boundTo(w) !== debugId) return undefined;
  try {
    // The daemon checkpoints on the stop itself; give a running one a moment.
    for (let i = 0; i < 30 && db.watch?.busy; i++) await new Promise((r) => setTimeout(r, 100));
    return shapeDbRead(await db.diffSince('previous_stop', { forAgent: true }), 5);
  } catch (e) {
    return { error: errText(e) };
  }
}

/** Start (or replace) the project's DB Watch for the agent's session. */
async function armDbWatch(
  db: IdeDbWatchAgentOps,
  snap: IdeDebugSessionSnapshot,
  root: string,
  asker: string,
  args: { objects?: { object: string; filter?: { column: string; value: string | number | boolean } }[]; connection_id?: string },
  launchText = '',
): Promise<Record<string, unknown>> {
  const det = await api.post<IdeDbWatchDetectResp>('/ide/db-watch/detect', {
    root,
    active_file: snap.paused_file ?? null,
    debug_session_id: snap.id,
  });
  let target: IdeDbWatchTargetRef;
  let engine: string | undefined;
  let local = true;
  if (args.connection_id) {
    const key = args.connection_id.trim();
    const c = det.saved_connections.find((x) => x.connection_id === key) ??
      det.saved_connections.find((x) => x.name.toLowerCase() === key.toLowerCase());
    if (!c) {
      const known = det.saved_connections.slice(0, 20).map((x) => x.name).join(', ');
      throw new UiCommandError('not_found', `No saved connection “${key}”.${known ? ` Known: ${known}` : ''}`);
    }
    target = { kind: 'saved', connection_id: c.connection_id };
    engine = c.engine;
    local = c.local;
  } else {
    // The run's own local database, never a saved one, of the engine the code
    // uses first: a container whose port the launch names (a DSN in env /
    // args), else the newest local one of that engine (`auto`).
    const ports = det.ephemeral_targets.filter((x) => new RegExp(`[:=]${x.port}(?!\\d)`).test(launchText));
    const own = det.ephemeral_targets.filter((x) => x.source !== 'container');
    const pool = ports.length ? ports : own.length ? own : det.ephemeral_targets;
    const t = det.detected_objects.map((o) => pool.find((x) => x.engine === o.engine)).find(Boolean) ?? pool[0];
    const e = t?.engine ?? det.detected_objects[0]?.engine;
    if (!e) throw new UiCommandError('not_found', 'No local database found for this run — pass `connection_id` (a saved connection).');
    target = t && ports.includes(t) ? { kind: 'ephemeral', ephemeral_id: t.ephemeral_id } : { kind: 'auto', engine: e };
    engine = e;
  }
  const items = args.objects?.length
    ? args.objects
    : det.detected_objects.filter((o) => o.engine === engine).slice(0, local ? 8 : 3).map((o) => ({ object: o.object_name }));
  if (!items.length) throw new UiCommandError('not_found', 'No tables found in the code near this stop — pass `objects`.');
  if (!(await db.start({ target, items, bind: { debug_session_id: snap.id } }, { asker }))) {
    throw db.error ? new UiCommandError('failed', db.error) : declined('watching that database');
  }
  return { watching: true, target: db.watch?.target.label ?? null, objects: items.map((i) => i.object) };
}

/** Walk `path` from the paused frame's variables (read-only DAP `variables`). */
async function resolvePath(ide: IdePort, snap: IdeDebugSessionSnapshot, path: string): Promise<IdeVariable> {
  const segs = pathSegments(path);
  const all = snap.scopes.flatMap((s) => s.variables);
  let cur = all.find((v) => v.name === segs[0]);
  if (!cur) {
    const names = all.slice(0, 30).map((v) => v.name).join(', ');
    throw new UiCommandError('not_found', `No variable “${segs[0] ?? path}” in this frame.${names ? ` In scope: ${names}` : ''}`);
  }
  for (const seg of segs.slice(1)) {
    if (cur.variables_reference <= 0) throw new UiCommandError('not_found', `${cur.name} has no fields (${clip(cur.value, 80)}).`);
    let kids = await ide.variables(cur.variables_reference);
    let next = childNamed(kids, seg);
    if (!next && kids.length === 1 && kids[0].name.startsWith('*') && kids[0].variables_reference > 0) {
      kids = await ide.variables(kids[0].variables_reference); // a pointer: look through it
      next = childNamed(kids, seg);
    }
    if (!next) throw new UiCommandError('not_found', `No “${seg}” in ${cur.name}. It has: ${kids.slice(0, 30).map((k) => k.name).join(', ')}`);
    cur = next;
  }
  return cur;
}

type BpSpec = IdeAgentBreakpointSpec;

registerUiCommands('ide', {
  async ide_debug_start(
    args: {
      root?: string; target: IdeDebugTarget; build_tags?: string[]; args?: string[]; env?: Record<string, string>;
      breakpoints?: BpSpec[]; break_on_exceptions?: boolean; ready_pattern?: string; ready_port?: number;
      db_watch?: boolean; wait_ms?: number; say?: string;
    },
    ctx: UiCommandCtx,
  ) {
    const t0 = Date.now();
    const root = checkRoot(args.root, ctx.agent.cwd ?? '');
    for (const [k, v] of Object.entries(args.env ?? {})) {
      if (typeof v !== 'string') throw new UiCommandError('invalid_args', `env ${k} must be a string.`);
    }
    const ide = await idePort.get(ctx.signal, 20_000);
    if (args.say) ide.say(ctx.agent, args.say);
    ctx.progress(`Opening ${basename(root)} in the IDE`);
    await ide.openProject(ctx.agent, root, ctx.signal);
    const plan = await ide.resolveTarget(args.target, { build_tags: args.build_tags, args: args.args, env: args.env });
    const bps = args.breakpoints ?? [];
    const ok = await ctx.confirmWrite({
      what: startConfirmText({ ...plan, breakpoints: bps.length }),
      where: basename(plan.root),
      verb: 'Debug',
    });
    if (!ok) throw declined('the debug run');
    // Restarting: the agent's own earlier session ends first (a session the
    // person started asks "Stop and debug again" in the IDE).
    const prev = ide.agentDebugId(ctx.agent);
    if (prev && ide.debug?.id === prev && !ended(ide.debug)) {
      await ide.command('stop').catch(() => null);
      await waitFor(ide, 5_000, ctx.signal, (s) => s.id === prev && ended(s), { poll: gone(ide, prev) });
    }
    if (bps.length) await ide.setAgentBreakpoints(ctx.agent, bps);
    ctx.progress(`Debugging ${plan.label}`);
    const { debug_id } = await ide.launch(ctx.agent, plan, { break_on_exceptions: args.break_on_exceptions }).catch(async (e: unknown) => {
      if (bps.length) await ide.clearAgentBreakpoints(ctx.agent, 'all').catch(() => 0); // nothing ran: leave no red dots
      throw e;
    });
    ide.consoleNote(`▶ ${plan.command} (${agentName(ctx.agent)})`);
    memory.set(ctx.agent.session_id, newMemory(debug_id, plan.root));
    let ready = false;
    const pattern = args.ready_pattern;
    const probe = args.ready_port
      ? async () => (ready ||= (await api.get<IdeDebugPortResp>(`/ide/debug/sessions/${encodeURIComponent(debug_id)}/port?port=${args.ready_port}`)).open)
      : undefined;
    const wait = Math.min(args.wait_ms ?? 60_000, t0 + BUDGET_MS - Date.now());
    const snap = await waitFor(ide, wait, ctx.signal, (s) => {
      if (s.id !== debug_id) return false;
      if (pattern && s.output.map((o) => o.text).join('').includes(pattern)) ready = true;
      return ready || ended(s) || s.state === 'paused';
    }, { poll: probe });
    const now = snap?.id === debug_id ? snap : ide.debug?.id === debug_id ? ide.debug : null;
    if (!now) throw new UiCommandError('failed', 'The debug session closed before it started (another session replaced it).');
    const out = await result(ide, ctx, now);
    if (now.state === 'running' || now.state === 'starting') out.ready = ready;
    if (args.db_watch && !ended(now)) {
      const db = ideDbWatchPort.peek();
      if (!db) out.db = { error: 'DB Watch isn’t available in this Otto window.' };
      else out.db = await armDbWatch(db, now, plan.root, agentName(ctx.agent), {}, [...Object.values(plan.env), ...(args.args ?? [])].join(' ')).catch((e) => ({ error: errText(e) }));
    }
    return out;
  },

  async ide_debug_control(
    args: { action: string; file?: string; line?: number; thread_id?: number; frame_id?: number; wait_ms?: number; keep_breakpoints?: boolean; say?: string },
    ctx: UiCommandCtx,
  ) {
    const t0 = Date.now();
    const ide = await idePort.get(ctx.signal, 20_000);
    const { snap, mem } = mine(ide, ctx);
    // A move starts a new stop: the last stop's narration would be wrong there.
    if (args.say || args.action !== 'status') ide.say(ctx.agent, args.say ?? '');
    const sid = ctx.agent.session_id;
    const wait = (): number => Math.min(args.wait_ms ?? 30_000, t0 + BUDGET_MS - Date.now());
    const a = args.action;
    if (a === 'status' || (ended(snap) && a !== 'stop')) return result(ide, ctx, snap);
    if (a === 'stop') {
      if (!ended(snap)) await ide.command('stop');
      const removed = args.keep_breakpoints ? 0 : await ide.clearAgentBreakpoints(ctx.agent, 'all');
      const end = await waitFor(ide, 5_000, ctx.signal, (s) => s.id === snap.id && ended(s), { poll: gone(ide, snap.id) });
      const out = await result(ide, ctx, end?.id === snap.id ? end : { ...snap, state: 'terminated' });
      memory.delete(sid);
      return { ...out, breakpoints_removed: removed };
    }
    if (userMoved(snap, mem)) {
      const out = await result(ide, ctx, snap);
      return { ...out, user_moved: true, note: 'The user moved the debugger since your last result, so nothing was done. Describe where it is now and ask before moving.' };
    }
    const after = snap.stop_seq ?? 0;
    if (a !== 'wait') {
      if (a === 'run_to_line' && !(args.file && args.line)) throw new UiCommandError('invalid_args', 'run_to_line needs `file` and `line`.');
      if (a !== 'pause' && snap.state !== 'paused') {
        throw new UiCommandError('failed', 'The program is running — use `wait` for the next stop or `pause`.');
      }
      memory.set(sid, { ...mem, expectMove: true });
      const action = (a === 'run_to_line' ? 'run_to_cursor' : a) as IdeDebugCommand;
      await ide.command(action, { thread_id: args.thread_id, frame_id: args.frame_id, path: args.file, line: args.line });
    }
    const next = await waitFor(ide, wait(), ctx.signal, nextStop(snap.id, after), { poll: gone(ide, snap.id) });
    return result(ide, ctx, next?.id === snap.id ? next : (ide.debug?.id === snap.id ? ide.debug : snap));
  },

  async ide_debug_breakpoints(
    args: { set?: BpSpec[]; clear?: { file: string; line: number }[]; clear_mine?: boolean },
    ctx: UiCommandCtx,
  ) {
    const ide = await idePort.get(ctx.signal, 20_000);
    // Adding needs the agent's live session; removing its own never hurts.
    const root = args.set?.length ? mine(ide, ctx).mem.root : (memory.get(ctx.agent.session_id)?.root ?? ide.root);
    let removed = 0;
    if (args.clear_mine) removed = await ide.clearAgentBreakpoints(ctx.agent, 'all');
    else if (args.clear?.length) removed = await ide.clearAgentBreakpoints(ctx.agent, args.clear);
    if (args.set?.length) await ide.setAgentBreakpoints(ctx.agent, args.set);
    return { breakpoints: shapeBreakpoints(ide.agentBreakpoints(ctx.agent), root), removed };
  },

  async ide_debug_inspect(
    args: { paths?: string[]; ref?: number; start?: number; count?: number; frame_id?: number; thread_id?: number; threads?: boolean },
    ctx: UiCommandCtx,
  ) {
    const ide = await idePort.get(ctx.signal, 20_000);
    const { snap } = mine(ide, ctx);
    if (args.threads) {
      return { count: snap.threads.length, stopped: snap.active_thread_id ?? null, threads: snap.threads.slice(0, 50).map((t) => ({ id: t.id, name: t.name })) };
    }
    if (!args.paths?.length && !args.ref) throw new UiCommandError('invalid_args', 'Pass `paths`, `ref` or `threads`.');
    if (snap.state !== 'paused') throw new UiCommandError('failed', 'Values can be read only while the program is paused (control `wait` or `pause`).');
    let cur = snap;
    if ((args.frame_id != null && args.frame_id !== snap.active_frame_id) || (args.thread_id != null && args.thread_id !== snap.active_thread_id)) {
      cur = (await ide.command('select_frame', { frame_id: args.frame_id, thread_id: args.thread_id })) ?? snap;
    }
    const out: Record<string, unknown> = {};
    if (args.ref) {
      const kids = await ide.variables(args.ref, { start: args.start, count: args.count ?? 100 });
      const taken = new Set<string>();
      out.children = Object.fromEntries(kids.slice(0, 100).map((k) => [uniqueKey(taken, k.name), shapeVariable(k, 200)]));
    }
    if (args.paths?.length) {
      // Keyed by the path: the bridge's redaction checks every segment of it.
      const values: Record<string, unknown> = {};
      for (const p of args.paths) {
        values[p] = await resolvePath(ide, cur, p).then((v) => shapeVariable(v), (e) => ({ error: errText(e) }));
      }
      out.values = values;
    }
    return out;
  },

  async ide_debug_evaluate(args: { expression: string; frame_id?: number }, ctx: UiCommandCtx) {
    const ide = await idePort.get(ctx.signal, 20_000);
    const { snap, mem } = mine(ide, ctx);
    const expr = String(args.expression ?? '').trim();
    if (!expr) throw new UiCommandError('invalid_args', '`expression` is empty.');
    if (snap.state !== 'paused') throw new UiCommandError('failed', 'Expressions run only while the program is paused.');
    const frame = snap.stack_frames.find((f) => f.id === (args.frame_id ?? snap.active_frame_id)) ?? snap.stack_frames[0];
    const where = frame ? `${frame.name} (${relPath(mem.root, frame.path)}:${frame.line})` : 'the paused frame';
    const ok = await ctx.confirmWrite({
      what: `Evaluate in ${where}:\n\n${expr}\n\nAn expression can call functions and change the program’s state, on your Mac as you.`,
      where: basename(mem.root),
      verb: 'Evaluate',
    });
    if (!ok) throw declined('the expression');
    const r = await ide.evaluate(expr, args.frame_id);
    // A path-like expression is the result's key, so it masks by name.
    const key = /^[\w.[\]"']+$/.test(expr) ? expr : 'result';
    return {
      expression: expr,
      values: { [key]: { value: clip(r.result, EVAL_MAX), ...(r.type ? { type: r.type } : {}), ...(r.variables_reference > 0 ? { ref: r.variables_reference } : {}) } },
    };
  },

  async ide_debug_request(
    args: { method?: string; url: string; headers?: Record<string, string>; body?: string; wait_ms?: number },
    ctx: UiCommandCtx,
  ) {
    const t0 = Date.now();
    const ide = await idePort.get(ctx.signal, 20_000);
    const { snap, mem } = mine(ide, ctx);
    if (ended(snap)) throw new UiCommandError('failed', 'The debug session has ended — start it again with ide_debug_start.');
    const wid = ws.currentId;
    if (!wid) throw new UiCommandError('failed', 'No workspace is open in this Otto window.');
    let url: URL;
    try {
      url = new URL(args.url);
    } catch {
      throw new UiCommandError('invalid_args', `Not a URL: ${clip(args.url, 200)}`);
    }
    const method = args.method ?? 'GET';
    const headers = Object.entries(args.headers ?? {}).map(([key, value]) => ({ key, value: String(value), enabled: true }));
    const what = [
      `${method} ${args.url}`,
      headers.length ? `Headers: ${headers.map((h) => h.key).join(', ')}` : '',
      args.body?.trim() ? `Body: ${clip(args.body.trim(), 300)}` : '',
      `It goes to the app you are debugging, on this Mac (${url.host}).`,
    ].filter(Boolean).join('\n\n');
    const host = url.hostname.replace(/^\[|\]$/g, '').toLowerCase();
    const ok = await ctx.confirmWrite({ what, where: url.host, verb: 'Send', connId: `api:${host}` });
    if (!ok) throw declined('to send the request');
    const sid = ctx.agent.session_id;
    ide.consoleNote(`→ ${method} ${url.host}${url.pathname}${url.search} (${agentName(ctx.agent)})`);
    memory.set(sid, { ...mem, expectMove: true });
    ide.say(ctx.agent, '');
    let response: Record<string, unknown> | null = null;
    let returned = false;
    const sent = api.long
      .post<ApiResponse>(`/workspaces/${encodeURIComponent(wid)}/api-client/execute`, {
        method, url: args.url, headers, query: [], body_mode: args.body ? 'raw' : 'none', body: args.body ?? '',
        timeout_ms: REQUEST_TIMEOUT_MS, follow_redirects: false, verify_ssl: true, debug_session_id: sid,
      })
      .then((r) => shapeResponse(r), (e) => ({ error: errText(e) }))
      .then((r) => {
        response = r;
        // Arrived after this call returned (the app was paused): the next result carries it.
        const m = memory.get(sid);
        if (returned && m?.debugId === snap.id) memory.set(sid, { ...m, late: { ...r, in_flight: false } });
      });
    const wait = Math.min(args.wait_ms ?? 60_000, t0 + BUDGET_MS - Date.now());
    const after = snap.stop_seq ?? 0;
    const next = await waitFor(ide, wait, ctx.signal, nextStop(snap.id, after), { settled: sent });
    const cur = next?.id === snap.id ? next : (ide.debug?.id === snap.id ? ide.debug : snap);
    const stopped = cur.state === 'paused' && (cur.stop_seq ?? 0) > after;
    const shaped = await result(ide, ctx, cur, {});
    returned = true;
    const request = response ? { ...(response as Record<string, unknown>), in_flight: false } : { in_flight: true };
    delete shaped.request;
    return stopped ? { request, stop: shaped } : { request, state: shaped.state, output_tail: shaped.output_tail };
  },

  async ide_debug_db(
    args: {
      action: 'watch' | 'unwatch' | 'diff';
      objects?: { object: string; filter?: { column: string; value: string | number | boolean } }[];
      connection_id?: string;
      since?: 'previous_stop' | 'start';
    },
    ctx: UiCommandCtx,
  ) {
    const ide = await idePort.get(ctx.signal, 20_000);
    const { snap, mem } = mine(ide, ctx);
    const db = await ideDbWatchPort.get(ctx.signal, 5_000).catch(() => {
      throw new UiCommandError('failed', 'DB Watch isn’t available in this Otto window.');
    });
    const ours = boundTo(db.watch) === snap.id;
    if (args.action === 'unwatch') {
      if (ours) await db.stop();
      return { watching: false };
    }
    if (args.action === 'diff') {
      if (!ours) throw new UiCommandError('not_found', 'Nothing is watched for your debug session — call ide_debug_db with action "watch" first.');
      return shapeDbRead(await db.diffSince(args.since ?? 'previous_stop', { forAgent: true }), 50);
    }
    const armed = await armDbWatch(db, snap, mem.root, agentName(ctx.agent), args);
    return { ...armed, ...shapeDbRead(await db.diffSince('start', { forAgent: true }), 50) };
  },
});
