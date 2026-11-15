// Pure context-chip builders, workspace/project root helpers, and unified
// workspace/project thread discovery for `ideAgentController.svelte.ts`.

import { api } from '../../../lib/api/client';
import type { RuntimeAccessMode, RuntimeSession, Session, SessionStatus } from '../../../lib/api/types';
import {
  harnessLabel,
  isPlainShellSession,
  sessionProviderSlug,
} from '../../../lib/providers';
import { inThreadsScope } from '../../../lib/stores/sessionBuckets';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import {
  ide,
  ideBasename,
  type IdeDebugSessionSnapshot,
  type IdeDiagnostic,
  type IdeFileReadResp,
  type IdeRunState,
  type IdeSymbolNode,
  type IdeTestResult,
  type IdeVariable,
} from '../ideStore.svelte';
import {
  detectProposalLanguage,
  matchesIdeProjectThread,
  offsetsForLineRange,
  type IdeContextChip,
} from '../store/ideDiffApplier';
import { buildPureSymbolDraftChip } from './ideSymbolChip';

function isIdePtyThreadSession(s: Session, pool: readonly Session[]): boolean {
  if (s.kind === 'connection') return false;
  const meta = s.meta as Record<string, unknown> | undefined;
  if (typeof meta?.side_of === 'string' && meta.side_of.trim() !== '') return false;
  if (meta?.threads_terminal === true) return false;
  return inThreadsScope(s, pool);
}

// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07/g;
const trimSlash = (p: string): string => p.replace(/\/+$/, '');

export interface IdeThreadEntry {
  id: string;
  title: string;
  provider: string;
  kind: 'runtime' | 'pty';
  status: string;
  workspace_id: string;
  cwd: string | null;
  model: string | null;
  access_mode?: RuntimeAccessMode;
  closed: boolean;
  dormant: boolean;
  working: boolean;
  needsYou: boolean;
  updatedAt: string;
}

export function collectIdeProjectRoots(): string[] {
  if (ide.target.kind !== 'local') return [];
  const roots: string[] = [];
  const push = (r?: string | null): void => {
    const c = (r ?? '').trim().replace(/\/+$/, '');
    if (c && c !== '~' && !roots.includes(c)) roots.push(c);
  };
  push(ide.target.root);
  const storeRoots = (ide as { additionalRoots?: string[] }).additionalRoots;
  if (Array.isArray(storeRoots)) for (let i = 0; i < storeRoots.length; i++) push(storeRoots[i]);
  const extraTarget = (ide.target as { additional_roots?: string[] }).additional_roots;
  if (Array.isArray(extraTarget)) for (let i = 0; i < extraTarget.length; i++) push(extraTarget[i]);
  const extraEnv = (ide.projectEnv as { additional_roots?: string[] } | null)?.additional_roots;
  if (Array.isArray(extraEnv)) for (let i = 0; i < extraEnv.length; i++) push(extraEnv[i]);
  return roots;
}

export function resolveIdeWorkspaceId(): string {
  if (ide.target.workspace_id) return ide.target.workspace_id;
  if (ide.target.kind === 'local' && ide.target.root) {
    const root = trimSlash(ide.target.root);
    const exact = ws.workspaces.find((w) => w.root_path && trimSlash(w.root_path) === root);
    if (exact) return exact.id;
    const parent = ws.workspaces.find((w) => w.root_path && root.startsWith(`${trimSlash(w.root_path)}/`));
    if (parent) return parent.id;
  }
  return ws.currentId ?? ws.workspaces[0]?.id ?? SCRATCH_WORKSPACE_ID;
}

export function resolveIdeProjectCwd(): string | null {
  return ide.target.kind === 'local' && ide.target.root && ide.target.root !== '~' ? ide.target.root : null;
}

export function currentIdeTargetKey(): string {
  const r = (ide.target.root ?? '').trim().replace(/\/+$/, '');
  return r ? `${ide.target.kind}:${r}` : '';
}

export async function buildFileDraftChip(
  path: string,
  readBuf: (fullPath: string) => string,
): Promise<Omit<IdeContextChip, 'id'>> {
  const fullPath = ide.resolvePath(path);
  const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
  const openTab = ide.tabs.find((t) => t.path === fullPath && !t.loading && !t.error);
  let content = '';
  let language = openTab?.language ?? '';
  if (openTab) {
    content = readBuf(fullPath);
  } else {
    const res = await api.get<IdeFileReadResp>(`/ide/fs/file?${ide.targetQuery({ path: fullPath })}`);
    content = res.content ?? '';
    language = res.language ?? '';
  }
  const lines = content ? content.split('\n').length : 1;
  return {
    key: `file:${fullPath}`,
    kind: 'file',
    label: `@${relPath}`,
    sublabel: `${lines} ${lines === 1 ? 'line' : 'lines'}`,
    path: fullPath,
    startLine: 1,
    endLine: Math.max(1, lines),
    language: detectProposalLanguage(relPath, language),
    snippet: content,
  };
}

export function collectIdeThreadEntries(opts: {
  runtimeAll: readonly RuntimeSession[];
  ptySessions: readonly Session[];
  statusMap: Readonly<Record<string, SessionStatus>>;
  needsYouMap: Readonly<Record<string, boolean>>;
  activeSessionId: string | null;
  targetWorkspaceId: string;
  projectRoots: readonly string[];
}): IdeThreadEntry[] {
  const list: IdeThreadEntry[] = [];
  const seen = new Set<string>();

  for (let i = 0; i < opts.runtimeAll.length; i++) {
    const r = opts.runtimeAll[i];
    if (
      !matchesIdeProjectThread({
        sessionId: r.id,
        activeSessionId: opts.activeSessionId,
        workspaceId: r.workspace_id,
        targetWorkspaceId: opts.targetWorkspaceId,
        cwd: r.cwd,
        projectRoots: opts.projectRoots,
      })
    ) {
      continue;
    }
    seen.add(r.id);
    const provSlug = sessionProviderSlug(r);
    list.push({
      id: r.id,
      title: r.title?.trim() || `${harnessLabel(provSlug)} thread`,
      provider: provSlug,
      kind: 'runtime',
      status: r.status,
      workspace_id: r.workspace_id,
      cwd: r.cwd ?? null,
      model: r.model ?? null,
      access_mode: r.access_mode,
      closed: r.status === 'closed',
      dormant: r.status === 'dormant',
      working: r.status === 'working' || r.status === 'starting',
      needsYou: r.status === 'awaiting_permission',
      updatedAt: r.last_active_at || r.created_at,
    });
  }

  for (let i = 0; i < opts.ptySessions.length; i++) {
    const s = opts.ptySessions[i];
    if (s.archived || seen.has(s.id) || isPlainShellSession(s) || !isIdePtyThreadSession(s, opts.ptySessions)) continue;
    if (
      !matchesIdeProjectThread({
        sessionId: s.id,
        activeSessionId: opts.activeSessionId,
        workspaceId: s.workspace_id,
        targetWorkspaceId: opts.targetWorkspaceId,
        cwd: s.cwd,
        projectRoots: opts.projectRoots,
      })
    ) {
      continue;
    }
    const rawStatus = opts.statusMap[s.id] ?? s.status ?? 'idle';
    const metaModel = typeof s.meta?.model === 'string' && s.meta.model.trim() ? s.meta.model.trim() : null;
    list.push({
      id: s.id,
      title: s.title?.trim() || `${harnessLabel(s.provider)} thread`,
      provider: s.provider,
      kind: 'pty',
      status: rawStatus,
      workspace_id: s.workspace_id,
      cwd: s.cwd ?? null,
      model: metaModel,
      closed: rawStatus === 'exited',
      dormant: false,
      working: rawStatus === 'running' || rawStatus === 'working',
      needsYou: opts.needsYouMap[s.id] === true,
      updatedAt: s.last_active_at ?? s.created_at,
    });
  }

  return list.sort((a, b) => {
    if (a.id === opts.activeSessionId) return -1;
    if (b.id === opts.activeSessionId) return 1;
    if (a.closed !== b.closed) return a.closed ? 1 : -1;
    return Date.parse(b.updatedAt || '') - Date.parse(a.updatedAt || '');
  });
}

export function buildSymbolDraftChip(
  fullPath: string,
  relPath: string,
  buf: string,
  sym: IdeSymbolNode,
): Omit<IdeContextChip, 'id'> {
  return buildPureSymbolDraftChip(fullPath, relPath, buf, sym);
}

export function buildDiagnosticDraftChip(
  fullPath: string,
  relPath: string,
  buf: string,
  diag: IdeDiagnostic,
): Omit<IdeContextChip, 'id'> {
  const startLine = Math.max(1, diag.line - 5);
  const endLine = Math.max(startLine, (diag.end_line ?? diag.line) + 5);
  let contextFrame = '';
  if (buf) {
    const slice = offsetsForLineRange(buf, startLine, endLine);
    const numbered = slice.text
      .split('\n')
      .map((ln, idx) => `${slice.startLine + idx === diag.line ? '>' : ' '} ${slice.startLine + idx} | ${ln}`)
      .join('\n');
    contextFrame = `\n\n${numbered}`;
  }
  return {
    key: `diag:${fullPath}:${diag.line}:${diag.message}`,
    kind: 'diagnostic',
    label: `⚠ ${relPath}:${diag.line}`,
    sublabel: `${diag.severity}: ${diag.message}`,
    path: fullPath,
    startLine: diag.line,
    endLine: diag.end_line ?? diag.line,
    language: detectProposalLanguage(relPath),
    snippet: `[${diag.severity.toUpperCase()}] ${relPath}:${diag.line}${diag.column ? `:${diag.column}` : ''} — ${diag.message}${contextFrame}`,
  };
}

export function buildTestFailureDraftChip(
  test: IdeTestResult,
  outputLines: readonly string[],
  fullPath: string | undefined,
  relPath: string | undefined,
): Omit<IdeContextChip, 'id'> {
  const clean = outputLines.map((l) => l.replace(ANSI_RE, ''));
  const hitIdx = clean.findIndex((l) => l.includes(test.name));
  const winLines =
    hitIdx >= 0 ? clean.slice(Math.max(0, hitIdx - 4), Math.min(clean.length, hitIdx + 36)) : clean.slice(-40);
  const header = [
    `Test: ${test.name}${test.suite ? ` (${test.suite})` : ''}`,
    fullPath && relPath ? `Location: ${relPath}${test.line ? `:${test.line}` : ''}` : '',
    test.message ? `Failure: ${test.message.replace(ANSI_RE, '').trim()}` : '',
  ]
    .filter(Boolean)
    .join('\n');
  return {
    key: `test:${test.suite ?? ''}:${test.name}`,
    kind: 'test_failure',
    label: `✗ ${test.name}`,
    sublabel: test.suite || 'test failure',
    path: fullPath,
    startLine: test.line ?? undefined,
    snippet: winLines.length > 0 ? `${header}\n\n${winLines.join('\n')}` : header,
  };
}

export function buildFailedRunDraftChip(run: IdeRunState): Omit<IdeContextChip, 'id'> {
  const clean = (run.output ?? []).map((l) => l.replace(ANSI_RE, '')).slice(-50);
  const title = run.title || 'Run';
  const exitText = run.exit_code !== null && run.exit_code !== undefined ? `, exit ${run.exit_code}` : '';
  return {
    key: `run:${run.id}`,
    kind: 'run_output',
    label: `✗ ${title}`,
    sublabel: run.exit_code !== null && run.exit_code !== undefined ? `exit ${run.exit_code}` : run.status,
    snippet: [`Run: ${title} (${run.status}${exitText})`, clean.join('\n')].filter(Boolean).join('\n\n'),
  };
}

export function buildDebugFrameDraftChip(
  dbg: IdeDebugSessionSnapshot,
  expandedVars: Record<number, IdeVariable[]> | undefined,
  resolvePath: (p: string) => string,
  relativePath: (p: string) => string,
): { chip: Omit<IdeContextChip, 'id'>; shortFile: string; line: number } {
  const topFrame = dbg.stack_frames.find((f) => f.id === dbg.active_frame_id) ?? dbg.stack_frames[0];
  const rawFile = dbg.paused_file ?? topFrame?.path ?? '';
  const fullPath = rawFile ? resolvePath(rawFile) : undefined;
  const shortFile = rawFile ? ideBasename(rawFile) : dbg.title || 'debug';
  const line = dbg.paused_line ?? topFrame?.line ?? 1;
  const fnName = topFrame?.name || 'paused';
  const stackText = dbg.stack_frames
    .slice(0, 8)
    .map((f, i) => `  #${i} ${f.name}${f.path ? ` (${relativePath(resolvePath(f.path))}:${f.line})` : ''}`)
    .join('\n');
  const scopeLines: string[] = [];
  for (let i = 0; i < dbg.scopes.length; i++) {
    const sc = dbg.scopes[i];
    const vars = sc.variables?.length ? sc.variables : (expandedVars?.[sc.variables_reference] ?? []);
    if (vars.length === 0) continue;
    scopeLines.push(`${sc.name}:`);
    for (let j = 0; j < Math.min(vars.length, 24); j++) {
      scopeLines.push(`  ${vars[j].name}${vars[j].var_type ? ` (${vars[j].var_type})` : ''} = ${vars[j].value}`);
    }
  }
  if (scopeLines.length === 0 && dbg.inline_values.length > 0) {
    scopeLines.push('Inline values:', ...dbg.inline_values.map((iv) => `  L${iv.line} ${iv.name} = ${iv.value}`));
  }
  const parts = [
    `Paused: ${fnName} @ ${fullPath ? relativePath(fullPath) : shortFile}:${line}${dbg.stopped_reason ? ` (${dbg.stopped_reason})` : ''}`,
    stackText ? `Call stack:\n${stackText}` : '',
    scopeLines.length > 0 ? scopeLines.join('\n') : '',
  ].filter(Boolean);
  return {
    shortFile,
    line,
    chip: {
      key: `debug:${dbg.id}:${fullPath ?? shortFile}:${line}`,
      kind: 'debug_frame',
      label: `⏸ ${fnName} @ ${shortFile}:${line}`,
      sublabel: dbg.stopped_reason || 'paused frame',
      path: fullPath,
      startLine: line,
      endLine: line,
      language: fullPath ? detectProposalLanguage(fullPath, dbg.language) : dbg.language,
      snippet: parts.join('\n\n'),
    },
  };
}
