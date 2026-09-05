// Pure helpers for the conversation view: durations, per-tool-kind chrome,
// unified-patch → `DiffResp` (so `git/DiffViewer` renders edit results), and
// the turn → render-item grouping that turns the parser's flat `Turn[]` into
// the app-style "user bubble / assistant response" rhythm.
import type { Block, DiffLine, DiffResp, FileDiff, Hunk, ToolKind, Turn, SystemNote } from '../../../lib/api/types';

/** "21m 17s" / "4.2s" / "850ms". */
export function fmtDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return '';
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${(ms / 1000).toFixed(s < 10 ? 1 : 0)}s`;
  const m = Math.floor(s / 60);
  const rs = s % 60;
  if (m < 60) return rs ? `${m}m ${rs}s` : `${m}m`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return rm ? `${h}h ${rm}m` : `${h}h`;
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function fmtCost(usd: number | null | undefined): string {
  if (usd == null) return '';
  return usd < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

export function fmtTokens(n: number | null | undefined): string {
  if (n == null) return '';
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** Clock time for a turn timestamp ("14:07"), or '' when unknown. */
export function fmtClock(ts: string | null): string {
  if (!ts) return '';
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}

/** Icon name (lib/components/Icon) + label per tool kind. */
export const TOOL_CHROME: Record<ToolKind, { icon: string; label: string }> = {
  shell: { icon: 'terminal', label: 'Ran' },
  read: { icon: 'eye', label: 'Read' },
  edit: { icon: 'edit', label: 'Edited' },
  write: { icon: 'file', label: 'Wrote' },
  search: { icon: 'search', label: 'Searched' },
  agent: { icon: 'radar', label: 'Delegated' },
  mcp: { icon: 'plug', label: 'Called' },
  skill: { icon: 'zap', label: 'Skill' },
  web: { icon: 'globe', label: 'Fetched' },
  ask: { icon: 'comment', label: 'Asked' },
  task: { icon: 'check', label: 'Planned' },
  other: { icon: 'box', label: 'Used' },
};

/** Short command/path preview for a tool row when the parser's `title` is bare. */
export function toolSubtitle(b: Extract<Block, { kind: 'tool_call' }>): string {
  const inp = (b.input ?? {}) as Record<string, unknown>;
  const pick = (...keys: string[]): string => {
    for (const k of keys) {
      const v = inp[k];
      if (typeof v === 'string' && v.trim()) return v.trim();
    }
    return '';
  };
  switch (b.tool) {
    case 'shell':
      return pick('command', 'cmd');
    case 'read':
    case 'edit':
    case 'write':
      return pick('file_path', 'path', 'notebook_path');
    case 'search':
      return pick('pattern', 'query', 'glob');
    case 'web':
      return pick('url', 'query');
    case 'agent':
      return pick('description', 'prompt');
    case 'skill':
      return pick('skill', 'name');
    default:
      return '';
  }
}

// ---------------------------------------------------------------------------
// Unified patch → DiffResp
// ---------------------------------------------------------------------------

const HUNK_RE = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/;

/** Parse a unified diff (one or more files) into the shape `git/DiffViewer`
 *  takes. Tolerant: a bare hunk stream with no `---/+++` headers becomes one
 *  file named `fallbackPath`. */
export function patchToDiff(patch: string, fallbackPath: string | null): DiffResp {
  const files: FileDiff[] = [];
  let cur: FileDiff | null = null;
  let hunk: Hunk | null = null;
  let oldLn = 0;
  let newLn = 0;
  let added = 0;
  let deleted = 0;

  const flushFile = (): void => {
    if (cur) {
      cur.added = added;
      cur.deleted = deleted;
      files.push(cur);
    }
    cur = null;
    hunk = null;
    added = 0;
    deleted = 0;
  };
  const ensureFile = (): FileDiff => {
    if (!cur) cur = { path: fallbackPath ?? 'file', old_path: null, is_binary: false, hunks: [] };
    return cur;
  };
  const stripPrefix = (p: string): string => p.replace(/^[ab]\//, '').trim();

  for (const raw of patch.replace(/\r\n/g, '\n').split('\n')) {
    if (raw.startsWith('diff --git ')) {
      flushFile();
      const m = /^diff --git a\/(.+?) b\/(.+)$/.exec(raw);
      cur = { path: m ? m[2] : (fallbackPath ?? 'file'), old_path: null, is_binary: false, hunks: [] };
      continue;
    }
    if (raw.startsWith('--- ')) {
      if (cur && cur.hunks.length) flushFile();
      const f = ensureFile();
      const p = raw.slice(4);
      if (!p.startsWith('/dev/null')) f.old_path = stripPrefix(p);
      continue;
    }
    if (raw.startsWith('+++ ')) {
      const f = ensureFile();
      const p = raw.slice(4);
      if (!p.startsWith('/dev/null')) f.path = stripPrefix(p);
      else if (f.old_path) f.path = f.old_path;
      if (f.old_path === f.path) f.old_path = null;
      continue;
    }
    if (raw.startsWith('Binary files')) {
      ensureFile().is_binary = true;
      continue;
    }
    const hm = HUNK_RE.exec(raw);
    if (hm) {
      const f = ensureFile();
      oldLn = Number(hm[1]);
      newLn = Number(hm[3]);
      hunk = { header: raw, lines: [] };
      f.hunks.push(hunk);
      continue;
    }
    if (!hunk) continue; // preamble (index lines, mode changes, …)
    if (raw.startsWith('\\')) continue; // "\ No newline at end of file"
    const c = raw[0];
    const content = raw.slice(1);
    let line: DiffLine;
    if (c === '+') {
      line = { origin: 'add', content, old_line: null, new_line: newLn++ };
      added++;
    } else if (c === '-') {
      line = { origin: 'del', content, old_line: oldLn++, new_line: null };
      deleted++;
    } else if (c === ' ' || raw === '') {
      line = { origin: 'context', content, old_line: oldLn++, new_line: newLn++ };
    } else {
      continue;
    }
    hunk.lines.push(line);
  }
  flushFile();
  return { files };
}

// ---------------------------------------------------------------------------
// Turn grouping
// ---------------------------------------------------------------------------

/** What the list renders: one row per user prompt, one per assistant
 *  RESPONSE (all consecutive assistant turns between two prompts — the
 *  tool-call loop of one request is many `requestId` turns on disk but one
 *  response to the reader). Consecutive tool-ish blocks inside a response are
 *  later collapsed into a "Worked for … · N steps" group by the renderer. */
export interface RenderItem {
  /** Stable key: the first turn's id. */
  id: string;
  role: 'user' | 'assistant';
  turns: Turn[];
  blocks: Block[];
  system: SystemNote[];
  /** Sum of the member turns' durations (assistant) — null if none recorded. */
  duration_ms: number | null;
  ts: string | null;
  model: string | null;
  /** Codex reasoning items in the member turns (never recorded → per-response footer). */
  reasoning_steps: number;
}

function isToolish(b: Block): boolean {
  return b.kind === 'tool_call' || b.kind === 'subagent' || b.kind === 'thinking' || b.kind === 'tasks';
}

/** Merge consecutive same-role turns into render items. User turns that carry
 *  prose each stay their own bubble; empty user turns (pure tool_result /
 *  system carriers) fold their notes into the surrounding response. */
export function groupTurns(turns: Turn[]): RenderItem[] {
  const out: RenderItem[] = [];
  const push = (t: Turn, role: 'user' | 'assistant'): void => {
    out.push({
      id: t.id,
      role,
      turns: [t],
      blocks: [...t.blocks],
      system: [...t.system],
      duration_ms: t.duration_ms,
      ts: t.ts,
      model: t.model,
      reasoning_steps: t.reasoning_steps ?? 0,
    });
  };
  const extend = (item: RenderItem, t: Turn): void => {
    item.turns.push(t);
    item.blocks.push(...t.blocks);
    item.system.push(...t.system);
    if (t.duration_ms != null) item.duration_ms = (item.duration_ms ?? 0) + t.duration_ms;
    if (!item.model && t.model) item.model = t.model;
    item.reasoning_steps += t.reasoning_steps ?? 0;
  };
  for (const t of turns) {
    const last = out[out.length - 1];
    const hasProse = t.blocks.some((b) => !isToolish(b) && b.kind !== 'queued' && b.kind !== 'notice');
    if (t.role === 'user') {
      if (hasProse || !last) push(t, 'user');
      else extend(last, t); // tool_result / reminder carrier → stays with the response
      continue;
    }
    if (last && last.role === 'assistant') extend(last, t);
    else push(t, 'assistant');
  }
  return out;
}

/** Split a response's blocks into prose / step-group segments for rendering. */
export type Segment =
  | { kind: 'block'; block: Block }
  | { kind: 'steps'; steps: Extract<Block, { kind: 'tool_call' | 'subagent' | 'thinking' | 'tasks' }>[] };

export function segment(blocks: Block[]): Segment[] {
  const segs: Segment[] = [];
  for (const b of blocks) {
    if (isToolish(b)) {
      const last = segs[segs.length - 1];
      if (last && last.kind === 'steps') last.steps.push(b as Extract<Block, { kind: 'tool_call' }>);
      else segs.push({ kind: 'steps', steps: [b as Extract<Block, { kind: 'tool_call' }>] });
    } else {
      segs.push({ kind: 'block', block: b });
    }
  }
  return segs;
}

/** Live queue state: `enqueue` chips that no later `dequeue`/`remove` of the
 *  same text cancelled. */
export function activeQueued(turns: Turn[]): Extract<Block, { kind: 'queued' }>[] {
  const live: Extract<Block, { kind: 'queued' }>[] = [];
  for (const t of turns) {
    for (const b of t.blocks) {
      if (b.kind !== 'queued') continue;
      if (b.op === 'enqueue') live.push(b);
      else {
        const i = live.findIndex((q) => q.text === b.text);
        if (i >= 0) live.splice(i, 1);
      }
    }
  }
  return live;
}
