// The IDE side of agent debugging: binds the store to lib/uiCommands/idePort.ts
// for the `otto.ui_ide_debug_*` handlers. Every action is the toolbar's or the
// gutter's own (open the project, gutter Debug, breakpoints, step), so the
// person watches the real IDE doing it. The agent-driven session is tracked in
// `uiControl.agentDebug` (driving bar narration, Proceed, "stopped driving").

import { untrack } from 'svelte';
import { api } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { agentName, uiControl } from '../../../lib/stores/uiControl.svelte';
import { UiCommandError } from '../../../lib/uiCommands';
import { ideDbWatchPort } from '../../../lib/uiCommands/ideDbWatchPort';
import { dismissOnAbort } from '../../../lib/uiCommands/pagePort';
import {
  registerIdePort, type IdeAgent, type IdeAgentBreakpointSpec, type IdeDebugCommand, type IdePort, type IdeResolvedLaunch,
} from '../../../lib/uiCommands/idePort';
import { IDE_OPEN_THREAD_EVENT } from '../agent/ideAgentState.svelte';
import { ideDbWatch } from '../dbwatch/ideDbWatchState.svelte';
import { ide } from '../ideStore.svelte';
import { mutatePathBreakpoints } from '../shell/ideProjectHelpers';
import { ideDebugLaunch, isDebugLaunchedHere } from '../store/ideDebugLaunch.svelte';
import { mergeGoBuildTagsIntoFlags } from '../store/ideRepoTestAndBuildTags';
import { ideBasename, type IdeDebugSessionSnapshot, type IdeGutterItem, type IdeVariable } from '../store/ideTypes';
import {
  agentBreakpointPaths, agentBreakpointsOf, withAgentBreakpoints, withoutAgentBreakpoints,
} from './ideAgentBreakpoints';
import { IdeTargetError, launchPreview, pickMainTarget, pickTestTarget } from './ideAgentTarget';

type Plan = Pick<NonNullable<Parameters<typeof ide.startDebug>[0]>, 'configId' | 'language' | 'launch'>;

type Launch = { language: string; launch: { program: string; args: string[]; build_flags?: string[]; cwd: string; test_name?: string; env?: Record<string, string> } };

const listeners = new Set<(s: IdeDebugSessionSnapshot) => void>();
let bpSeq = 0;
const newBpId = (): string => `bp_${Date.now().toString(36)}_${++bpSeq}`;
const norm = (p: string): string => p.replace(/\/+$/, '');
const live = (s: IdeDebugSessionSnapshot | null): s is IdeDebugSessionSnapshot => !!s && s.state !== 'terminated' && s.state !== 'error';
const plain = (s: IdeDebugSessionSnapshot | null): IdeDebugSessionSnapshot | null => (s ? $state.snapshot(s) as IdeDebugSessionSnapshot : null);

function asUi(e: unknown): never {
  if (e instanceof IdeTargetError) throw new UiCommandError(e.code, e.message);
  throw e;
}

/** `root` is (inside) a folder of the open local project. */
function shows(root: string): boolean {
  return ide.target.kind === 'local' && ide.projectRoots.some((r) => root === norm(r.path) || root.startsWith(`${norm(r.path)}/`));
}

function requireSession(): IdeDebugSessionSnapshot {
  const s = ide.activeDebug;
  if (!s) throw new UiCommandError('not_found', 'No debug session is open in this IDE');
  return s;
}

/** The uiControl record follows the store's snapshot of the agent's session. */
function followAgentSession(snap: IdeDebugSessionSnapshot | null): void {
  const a = uiControl.agentDebug;
  if (!a) return;
  if (!snap || snap.id !== a.debugId) uiControl.debugState(a.debugId, null, true);
  else uiControl.debugState(a.debugId, snap.state === 'paused' ? (snap.stop_seq ?? 0) : null, !live(snap));
}

async function openProject(agent: IdeAgent, rawRoot: string, signal?: AbortSignal): Promise<void> {
  const root = norm(rawRoot.trim());
  if (!root.startsWith('/')) throw new UiCommandError('invalid_args', 'The project root must be an absolute path');
  if (!shows(root)) {
    if (ide.target.root) {
      // This IDE shows another project: the person was using it — ask first.
      const s = ide.activeDebug;
      const stops = live(s) && isDebugLaunchedHere(s.id);
      const pick = await dismissOnAbort(signal ?? new AbortController().signal, confirmer.choose(
        `${agentName(agent)} wants to open ${ideBasename(root)} in this IDE window, which shows ${ideBasename(ide.target.root)}, to debug it.${stops ? ` The debug session running here (${s.title}) stops.` : ''}`,
        { title: 'Open another project here?', options: [{ label: `Open ${ideBasename(root)}`, value: 'open', kind: 'primary' }] },
      ));
      if (pick.value !== 'open') throw new UiCommandError('cancelled_by_user', `The person kept this IDE on ${ideBasename(ide.target.root)}`);
    }
    const named = ide.namedProjects.find((p) => p.folders.some((f) => root === norm(f) || root.startsWith(`${norm(f)}/`)));
    const ok = named ? (await ide.activateProject(named.id), true) : await ide.setLocalTarget(root, null);
    if (!ok || !shows(root)) throw new UiCommandError('cancelled_by_user', `The IDE did not switch to ${ideBasename(root)}`);
  }
  if (agent.runtime === 'structured' && typeof window !== 'undefined') {
    // The thread docks beside the editor (the IDE agent panel, same pool as Threads).
    window.dispatchEvent(new CustomEvent(IDE_OPEN_THREAD_EVENT, { detail: { root, sessionId: agent.session_id } }));
  }
}

async function resolveTarget(
  target: Parameters<IdePort['resolveTarget']>[0],
  opts: Parameters<IdePort['resolveTarget']>[1] = {},
): Promise<IdeResolvedLaunch> {
  if (!ide.target.root) throw new UiCommandError('failed', 'No project is open in this IDE');
  if (ide.target.kind !== 'local') throw new UiCommandError('failed', 'Agent debugging runs local projects only');
  const tags = opts.build_tags ?? [];
  const env = opts.env ?? {};
  if (target.kind === 'config') {
    const k = target.name.trim().toLowerCase();
    const cfg = ide.runConfigs.find((c) => c.id === target.name) ?? ide.runConfigs.find((c) => c.name.toLowerCase() === k);
    if (!cfg) {
      const names = ide.runConfigs.slice(0, 20).map((c) => c.name).join(', ');
      throw new UiCommandError('not_found', `No run configuration “${target.name}”.${names ? ` Known: ${names}` : ''}`);
    }
    if (tags.length || opts.args?.length || Object.keys(env).length) {
      throw new UiCommandError('invalid_args', 'A run configuration brings its own build tags, arguments and env — debug the test or main instead');
    }
    return {
      root: ide.target.root, label: cfg.name, language: cfg.runner, cwd: cfg.cwd || ide.target.root,
      command: [cfg.command || cfg.runner, ...cfg.args].join(' '), build_flags: cfg.build_flags ?? [], env: {},
      plan: { configId: cfg.id } satisfies Plan,
    };
  }
  const path = ide.resolvePath(target.file);
  await ide.openFile(path, { preview: false });
  const tab = ide.tabs.find((t) => t.path === path);
  if (!tab || tab.error) throw new UiCommandError('not_found', `Can’t open ${target.file}${tab?.error ? `: ${tab.error}` : ''}`);
  const content = ide.getBuffer(path);
  const { items = [] } = await api.post<{ items: IdeGutterItem[] }>('/ide/gutter', { target: ide.targetPayload, path, content });
  const language = items[0]?.language || tab.language || '';
  let item: IdeGutterItem;
  try {
    if (target.kind === 'main') item = pickMainTarget(items);
    else {
      const picked = pickTestTarget(items, content, language, target.name);
      const base = picked.item ?? { line: 1, kind: 'test', label: picked.testName, language, run_spec: { target_path: path, debug_program: path } };
      const natural = base.run_spec.debug_test_name || base.run_spec.test_name || base.label;
      item = natural === picked.testName ? base : { ...base, run_spec: { ...base.run_spec, debug_test_name: picked.testName } };
    }
  } catch (e) {
    asUi(e);
  }
  // Exactly what a gutter click would launch (build tags, cwd, test name) — the
  // toolbar target the person chose is left as it was.
  const m = await import('../store/ideDebugAndEnvController');
  const keep = { gutter: ide.envState.gutterTarget, selected: ide.selectedConfigId };
  let got: Launch | null = null;
  m.launchGutterAction(ide, item, 'debug', () => {}, (o) => { got = o as Launch; });
  ide.envState.gutterTarget = keep.gutter;
  ide.selectedConfigId = keep.selected;
  const l = got as Launch | null;
  if (!l) throw new UiCommandError('failed', `Can’t debug ${item.label}`);
  if (tags.length && l.language !== 'go') throw new UiCommandError('invalid_args', '`build_tags` are for Go');
  const launch = {
    ...l.launch,
    build_flags: tags.length ? mergeGoBuildTagsIntoFlags(l.launch.build_flags, tags) : (l.launch.build_flags ?? []),
    args: opts.args ? [...opts.args] : l.launch.args,
    env: { ...env },
  };
  return {
    root: ide.target.root,
    label: launch.test_name || ide.relativePath(launch.program),
    language: l.language,
    command: launchPreview({ language: l.language, ...launch, root: ide.target.root }),
    cwd: launch.cwd,
    build_flags: launch.build_flags,
    env: launch.env,
    plan: { language: l.language, launch } satisfies Plan,
  };
}

async function launch(agent: IdeAgent, resolved: IdeResolvedLaunch, opts: { break_on_exceptions?: boolean } = {}): Promise<{ debug_id: string }> {
  const p = resolved.plan as Plan | undefined;
  if (!p || (!p.configId && !p.launch)) throw new UiCommandError('invalid_args', 'Resolve the target first');
  // The debugger's visible switch, as the person would flip it; never turned off for them.
  if (opts.break_on_exceptions) ide.breakOnExceptions = true;
  const before = ide.activeDebug?.id ?? null;
  // The store's own path: prod gate, "Stop and debug again", autosave, Debug tab.
  await ide.startDebug(p.configId ? { configId: p.configId } : { language: p.language, launch: p.launch });
  const snap = ide.activeDebug;
  if (snap && snap.id !== before) {
    if (snap.state === 'error') throw new UiCommandError('failed', snap.error || 'The debug session couldn’t start');
    uiControl.startDebug(agent, snap.id);
    followAgentSession(snap);
    return { debug_id: snap.id };
  }
  if (ideDebugLaunch.startError) throw new UiCommandError('failed', ideDebugLaunch.startError.message);
  throw new UiCommandError('cancelled_by_user', 'The debug run was not started (declined in Otto)');
}

async function command(action: IdeDebugCommand, extra: { thread_id?: number; frame_id?: number; path?: string; line?: number } = {}): Promise<IdeDebugSessionSnapshot | null> {
  const s = requireSession();
  if (!live(s)) throw new UiCommandError('failed', 'The debug session has ended — start it again');
  const resp = await api.post<{ session: IdeDebugSessionSnapshot }>(`/ide/debug/sessions/${encodeURIComponent(s.id)}/command`, {
    action,
    thread_id: extra.thread_id ?? s.active_thread_id ?? undefined,
    frame_id: extra.frame_id ?? (action === 'select_frame' && extra.thread_id != null ? undefined : s.active_frame_id ?? undefined),
    path: extra.path ? ide.resolvePath(extra.path) : undefined,
    line: extra.line,
  });
  ide.applyDebugSnapshot(resp.session);
  return plain(ide.activeDebug);
}

/** A line in the Debug console's REPL lane, after the output printed so far. */
function consoleEntry(expression: string, result: string, varType: string | null, ref: number): void {
  const s = ide.activeDebug;
  const afterSeq = s?.output?.length ? (s.output[s.output.length - 1].seq ?? 0) : 0;
  ide.replEntries = [
    ...ide.replEntries,
    { id: `agent-${Date.now()}-${ide.replEntries.length}`, expression, result, var_type: varType, variables_reference: ref, afterSeq },
  ].slice(-100);
}

async function settleBreakpoints(): Promise<void> {
  await ide.syncBreakpointsToSession();
}

const port: IdePort = {
  get root() { return ide.target.root; },
  get projectRoots() { return ide.projectRoots.map((r) => r.path); },
  get debug() { return plain(ide.activeDebug); },
  agentDebugId(agent) {
    const a = uiControl.agentDebug;
    return a && !a.released && a.agent.session_id === agent.session_id ? a.debugId : null;
  },
  openProject,
  resolveTarget,
  launch,
  command,
  onSnapshot(fn) {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  async setAgentBreakpoints(agent, bps) {
    const byPath = new Map<string, IdeAgentBreakpointSpec[]>();
    for (const b of bps) {
      const path = ide.resolvePath(b.file);
      byPath.set(path, [...(byPath.get(path) ?? []), b]);
    }
    const who = { session_id: agent.session_id, name: agentName(agent) };
    for (const [path, specs] of byPath) mutatePathBreakpoints(ide, path, (p, list) => withAgentBreakpoints(list, p, specs, who, newBpId));
    await settleBreakpoints();
    return agentBreakpointsOf(ide.breakpointsByPath, agent.session_id);
  },
  async clearAgentBreakpoints(agent, which) {
    const resolved = which === 'all' ? which : which.map((w) => ('id' in w ? w : { file: ide.resolvePath(w.file), line: w.line }));
    const before = agentBreakpointsOf(ide.breakpointsByPath, agent.session_id).length;
    for (const path of agentBreakpointPaths(ide.breakpointsByPath, agent.session_id, resolved)) {
      mutatePathBreakpoints(ide, path, (p, list) => withoutAgentBreakpoints(list, p, agent.session_id, resolved));
    }
    await settleBreakpoints();
    return before - agentBreakpointsOf(ide.breakpointsByPath, agent.session_id).length;
  },
  agentBreakpoints: (agent) => agentBreakpointsOf(ide.breakpointsByPath, agent.session_id),
  async variables(ref, opts = {}) {
    const s = requireSession();
    const resp = await api.post<{ variables: IdeVariable[] }>(`/ide/debug/sessions/${encodeURIComponent(s.id)}/variables`, { variables_reference: ref });
    const start = Math.max(0, opts.start ?? 0);
    return (resp.variables ?? []).slice(start, opts.count != null ? start + Math.max(0, opts.count) : undefined);
  },
  async evaluate(expression, frameId) {
    const s = requireSession();
    const resp = await api.post<{ result: string; var_type?: string | null; variables_reference?: number }>(
      `/ide/debug/sessions/${encodeURIComponent(s.id)}/evaluate`,
      { expression, frame_id: frameId ?? s.active_frame_id ?? undefined, context: 'repl' },
    );
    consoleEntry(expression, resp.result, resp.var_type ?? null, resp.variables_reference ?? 0);
    return { result: resp.result, type: resp.var_type ?? null, variables_reference: resp.variables_reference ?? 0 };
  },
  async readLines(file, from, to) {
    const path = ide.resolvePath(file);
    const text = ide.tabs.some((t) => t.path === path)
      ? ide.getBuffer(path)
      : (await api.get<{ content: string }>(`/ide/fs/file?${ide.targetQuery({ path })}`)).content;
    const lines = text.split('\n');
    return lines.slice(Math.max(0, from - 1), Math.max(0, to));
  },
  consoleNote: (text) => consoleEntry(text, '', null, 0),
  say(agent, text) {
    uiControl.debugSay(agent.session_id, text);
  },
};

/** The IDE page binds the ports (debugger, DB Watch) while it is mounted (`$effect(() => bindIdeAgentPort())`). */
export function bindIdeAgentPort(): () => void {
  const unbind = registerIdePort(port);
  const unbindDb = ideDbWatchPort.bind(ideDbWatch);
  const stop = $effect.root(() => {
    $effect(() => {
      const snap = ide.activeDebug;
      untrack(() => {
        followAgentSession(snap);
        const p = plain(snap);
        if (p) for (const fn of listeners) fn(p);
      });
    });
  });
  return () => {
    stop();
    unbind();
    unbindDb();
  };
}
