// Pure helpers for the conversation view: durations, per-tool-kind chrome,
// unified-patch → `DiffResp` (so `git/DiffViewer` renders edit results), and
// the turn → render-item grouping that turns the parser's flat `Turn[]` into
// the app-style "user bubble / assistant response" rhythm.
import type { IconName } from '../../../lib/components/Icon.svelte';
import type { Block, DiffLine, DiffResp, FileDiff, Hunk, ToolKind, Turn, TurnUsage, SystemNote } from '../../../lib/api/types';

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
export const TOOL_CHROME: Record<ToolKind, { icon: IconName; label: string }> = {
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
  /** Token usage summed over the member turns that recorded any (null when none
   *  did). Optional: the assistant's index items do not carry it. */
  usage?: TurnUsage | null;
}

/** `a + b` field by field; either side may be missing. */
export function addUsage(a: TurnUsage | null | undefined, b: TurnUsage | null | undefined): TurnUsage | null {
  a ??= null;
  if (!b) return a;
  if (!a) return { ...b };
  return {
    input_tokens: a.input_tokens + b.input_tokens,
    output_tokens: a.output_tokens + b.output_tokens,
    thinking_tokens: a.thinking_tokens + b.thinking_tokens,
    cache_read_tokens: a.cache_read_tokens + b.cache_read_tokens,
    cache_creation_tokens: a.cache_creation_tokens + b.cache_creation_tokens,
  };
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
      usage: t.role === 'assistant' ? addUsage(null, t.usage) : null,
    });
  };
  const extend = (item: RenderItem, t: Turn): void => {
    item.turns.push(t);
    item.blocks.push(...t.blocks);
    item.system.push(...t.system);
    if (t.duration_ms != null) item.duration_ms = (item.duration_ms ?? 0) + t.duration_ms;
    if (!item.model && t.model) item.model = t.model;
    item.reasoning_steps += t.reasoning_steps ?? 0;
    if (t.role === 'assistant') item.usage = addUsage(item.usage, t.usage);
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

/** `groupTurns` that hands back the PREVIOUS render item — the same object —
 *  for every group built from the very same `Turn` objects as last time. A live
 *  delta replaces one turn (the growing response) and appends new ones; every
 *  other item keeps its identity, so the keyed `{#each}` sees no change and its
 *  `TurnItem` never re-segments or re-renders markdown (was O(all mounted
 *  markdown) per delta — ~100 ms at 300 turns). `cache` is the caller's memo,
 *  rewritten in place to hold exactly this call's items. */
export function stableGroupTurns(turns: Turn[], cache: Map<string, RenderItem>): RenderItem[] {
  const fresh = groupTurns(turns);
  const out: RenderItem[] = new Array(fresh.length);
  const kept: RenderItem[] = new Array(fresh.length);
  for (let i = 0; i < fresh.length; i++) {
    const it = fresh[i];
    const old = cache.get(it.id);
    const same = old !== undefined && old.turns.length === it.turns.length && old.turns.every((t, j) => t === it.turns[j]);
    out[i] = kept[i] = same ? old : it;
  }
  cache.clear();
  for (const it of kept) cache.set(it.id, it);
  return out;
}

/** Split a response's blocks into prose / step-group segments for rendering. */
export type Segment =
  | { kind: 'block'; block: Block }
  | { kind: 'steps'; steps: Extract<Block, { kind: 'tool_call' | 'subagent' | 'thinking' | 'tasks' }>[] };

export function segment(blocks: Block[]): Segment[] {
  const segs: Segment[] = [];
  for (let i = 0; i < blocks.length; i++) {
    const b = blocks[i];
    // The plan is shown as its own checklist, in the flow (not hidden inside a
    // collapsed step group); the plan-writing call that produced it is that
    // checklist, so it doesn't get a row of its own too.
    if (b.kind === 'tool_call' && b.tool === 'task' && blocks[i + 1]?.kind === 'tasks' && (b.result == null || b.result.ok)) continue;
    if (isToolish(b) && b.kind !== 'tasks') {
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

// ---------------------------------------------------------------------------
// Chat presentation (rev 5): agent names, one-line tool summaries, the live
// "what is it doing now" line, the jump-to-latest unread count.
// ---------------------------------------------------------------------------

export type ToolCallBlock = Extract<Block, { kind: 'tool_call' }>;
export type StepBlock = Extract<Block, { kind: 'tool_call' | 'subagent' | 'thinking' | 'tasks' }>;

/** Display name of the CLI behind a transcript ("Claude", "Codex", …). */
export function providerName(provider: string | null | undefined): string {
  const p = (provider ?? '').trim();
  switch (p.toLowerCase()) {
    case 'claude':
      return 'Claude';
    case 'codex':
      return 'Codex';
    case 'agy':
      return 'Antigravity';
    case '':
      return 'Agent';
    default:
      return p[0].toUpperCase() + p.slice(1);
  }
}

/** Past / present verb per tool kind: rows read "Edited retry.rs", the live
 *  line reads "Editing retry.rs". */
const VERBS: Record<ToolKind, [past: string, present: string, base: string]> = {
  shell: ['Ran', 'Running', 'run'],
  read: ['Read', 'Reading', 'read'],
  edit: ['Edited', 'Editing', 'edit'],
  write: ['Wrote', 'Writing', 'write'],
  search: ['Searched', 'Searching', 'search'],
  agent: ['Delegated', 'Delegating', 'delegate'],
  mcp: ['Called', 'Calling', 'call'],
  skill: ['Used skill', 'Using skill', 'use skill'],
  web: ['Fetched', 'Fetching', 'fetch'],
  ask: ['Asked', 'Asking', 'ask'],
  task: ['Updated the plan', 'Updating the plan', 'update the plan'],
  other: ['Used', 'Using', 'use'],
};

/** One tool call as a scannable line: a verb, the thing it acted on (`mono`
 *  when it is a command / pattern / path fragment), and a dim detail (the
 *  file's folder, a search scope). `hint` is the human description the agent
 *  gave the call (Bash `description`), for the tooltip / detail header. */
export interface ToolLine {
  verb: string;
  present: string;
  /** "wants to <base> …" — the waiting-for-you card. */
  base: string;
  target: string;
  mono: boolean;
  detail: string;
  hint: string;
}

function str(input: unknown, ...keys: string[]): string {
  if (input == null || typeof input !== 'object') return '';
  const o = input as Record<string, unknown>;
  for (const k of keys) {
    const v = o[k];
    if (typeof v === 'string' && v.trim()) return v.trim();
    if (Array.isArray(v) && v.length && v.every((x) => typeof x === 'string')) return (v as string[]).join(' ');
  }
  return '';
}

function splitPath(p: string): { base: string; dir: string } {
  const clean = p.replace(/\/+$/, '');
  const at = clean.lastIndexOf('/');
  return at < 0 ? { base: clean, dir: '' } : { base: clean.slice(at + 1), dir: clean.slice(0, at) };
}

const firstLine = (s: string): string => {
  const at = s.indexOf('\n');
  return at < 0 ? s : `${s.slice(0, at)} …`;
};

export function toolLine(b: ToolCallBlock): ToolLine {
  const [verb, present, base] = VERBS[b.tool] ?? VERBS.other;
  const title = (b.title ?? '').trim() || b.name;
  const line = (target: string, mono: boolean, detail = '', hint = ''): ToolLine => ({ verb, present, base, target, mono, detail, hint });
  switch (b.tool) {
    case 'shell': {
      const cmd = str(b.input, 'command', 'cmd');
      const desc = str(b.input, 'description');
      return cmd ? line(firstLine(cmd), true, '', desc) : line(title, false);
    }
    case 'read':
    case 'edit':
    case 'write': {
      const p = str(b.input, 'file_path', 'path', 'notebook_path') || b.result?.file_path || '';
      if (!p) return line(title, false);
      const { base, dir } = splitPath(p);
      return line(base, true, dir);
    }
    case 'search': {
      const pat = str(b.input, 'pattern', 'query');
      const scope = str(b.input, 'path', 'glob');
      if (pat) return line(pat, true, scope);
      return scope ? line(scope, true) : line(title, false);
    }
    case 'web':
      return line(str(b.input, 'url', 'query') || title, true);
    case 'agent':
      return line(str(b.input, 'description', 'subagent_type') || title, false);
    case 'skill':
      return line(str(b.input, 'skill', 'command', 'name') || title.replace(/^Skill\s+/, ''), true);
    case 'task':
      return line('', false);
    case 'ask':
      return line('a question', false);
    case 'mcp':
      return line(title, false);
    default:
      return line(title, false);
  }
}

/** Status of one tool call. `none` = no result was ever recorded and the
 *  agent is not working on it any more (interrupted / crashed turn). */
export type StepStatus = 'running' | 'ok' | 'err' | 'none';

export function toolStatus(b: ToolCallBlock, live: boolean): StepStatus {
  if (b.result == null) return live ? 'running' : 'none';
  return b.result.ok ? 'ok' : 'err';
}

const plural = (n: number, one: string, many = `${one}s`): string => `${n} ${n === 1 ? one : many}`;

/** "Ran 2 commands, edited retry.rs, read 3 files" — what a step group did,
 *  in first-seen order, at most `max` phrases (then "and N more"). */
export function stepSummary(steps: StepBlock[], max = 4): string {
  type Acc = { n: number; names: Set<string> };
  const order: string[] = [];
  const acc = new Map<string, Acc>();
  const bump = (key: string, name = ''): void => {
    let a = acc.get(key);
    if (!a) {
      a = { n: 0, names: new Set() };
      acc.set(key, a);
      order.push(key);
    }
    a.n++;
    if (name) a.names.add(name);
  };
  for (const s of steps) {
    if (s.kind === 'subagent') bump('agent', s.description);
    else if (s.kind === 'tool_call') {
      const l = toolLine(s);
      bump(s.tool, s.tool === 'read' || s.tool === 'edit' || s.tool === 'write' ? l.target : '');
    }
  }
  const files = (a: Acc, verb: string): string =>
    a.names.size === 1 ? `${verb} ${[...a.names][0]}` : `${verb} ${plural(a.names.size || a.n, 'file')}`;
  // One reads as words ("ran a command"), more as counts ("ran 3 commands").
  const count = (n: number, one: string, many: string): string => (n === 1 ? one : many.replace('#', String(n)));
  const phrase = (key: string, a: Acc): string => {
    switch (key) {
      case 'shell':
        return count(a.n, 'ran a command', 'ran # commands');
      case 'read':
        return files(a, 'read');
      case 'edit':
        return files(a, 'edited');
      case 'write':
        return files(a, 'wrote');
      case 'search':
        return count(a.n, 'searched the code', 'ran # searches');
      case 'web':
        return count(a.n, 'fetched a page', 'fetched # pages');
      case 'agent':
        return count(a.n, 'delegated to an agent', 'delegated to # agents');
      case 'mcp':
        return count(a.n, 'called a tool', 'called # tools');
      case 'skill':
        return count(a.n, 'used a skill', 'used # skills');
      case 'task':
        return 'updated the plan';
      case 'ask':
        return 'asked you a question';
      default:
        return count(a.n, 'used a tool', 'used # tools');
    }
  };
  const parts = order.map((k) => phrase(k, acc.get(k)!));
  if (!parts.length) return steps.some((s) => s.kind === 'thinking') ? 'Thought' : 'Worked';
  const shown = parts.length > max ? [...parts.slice(0, max - 1), `and ${parts.length - (max - 1)} more`] : parts;
  const text = shown.join(', ');
  return text[0].toUpperCase() + text.slice(1);
}

/** The newest tool call still waiting for its result in a response — what the
 *  agent is doing right now (or blocked on). */
export function pendingTool(item: RenderItem | undefined): ToolCallBlock | null {
  if (!item || item.role !== 'assistant') return null;
  for (let i = item.blocks.length - 1; i >= 0; i--) {
    const b = item.blocks[i];
    if (b.kind === 'tool_call' && b.result == null) return b;
  }
  return null;
}

/** Last non-blank line of a command's output (the "tail" a collapsed row
 *  shows), clipped. Scans from the end — outputs are up to 64 KB. */
export function lastLine(text: string | null | undefined, max = 160): string {
  if (!text) return '';
  let end = text.length;
  while (end > 0) {
    const start = text.lastIndexOf('\n', end - 1) + 1;
    const l = text.slice(start, end).trim();
    if (l) return l.length > max ? `${l.slice(0, max - 1)}…` : l;
    end = start - 1;
  }
  return '';
}

/** Items that arrived after the one the reader last saw at the bottom. A
 *  missing anchor (paged out, or the reader never saw one) counts nothing. */
export function countUnread(ids: string[], lastSeenId: string | null): number {
  if (!lastSeenId) return 0;
  const at = ids.lastIndexOf(lastSeenId);
  return at < 0 ? 0 : ids.length - 1 - at;
}

/** The questions of an AskUserQuestion call, if its input has the shape. */
export interface AgentQuestion {
  question: string;
  options: string[];
}
export function askQuestions(input: unknown): AgentQuestion[] {
  if (input == null || typeof input !== 'object') return [];
  const qs = (input as { questions?: unknown }).questions;
  if (!Array.isArray(qs)) return [];
  const out: AgentQuestion[] = [];
  for (const q of qs) {
    if (q == null || typeof q !== 'object') continue;
    const text = (q as { question?: unknown }).question;
    if (typeof text !== 'string' || !text.trim()) continue;
    const opts = (q as { options?: unknown }).options;
    const options = Array.isArray(opts)
      ? opts
          .map((o) => (typeof o === 'string' ? o : o && typeof o === 'object' ? (o as { label?: unknown }).label : null))
          .filter((l): l is string => typeof l === 'string' && !!l.trim())
      : [];
    out.push({ question: text.trim(), options });
  }
  return out;
}

// ---------------------------------------------------------------------------
// Chat v2 (rev 6): the settled-response fold, changed files, tokens, days,
// paths.
// ---------------------------------------------------------------------------

/** A settled response split for the "Worked for …" fold (t3code-style): the
 *  FIRST and the FINAL message stay visible, the work between them — tool
 *  groups, intermediate narration, plan snapshots — folds into one row. */
export interface ResponseFold {
  head: Segment[];
  body: Segment[];
  tail: Segment[];
}

const isText = (s: Segment | undefined): boolean => s?.kind === 'block' && s.block.kind === 'text';

/** Null when there is nothing worth folding (no tool work, or no message to
 *  keep visible around it). */
export function foldResponse(segs: Segment[]): ResponseFold | null {
  if (!segs.some((s) => s.kind === 'steps' && s.steps.some((b) => b.kind !== 'thinking'))) return null;
  const first = segs.findIndex(isText);
  if (first < 0) return null;
  let last = -1;
  for (let i = segs.length - 1; i >= 0; i--) {
    if (isText(segs[i])) {
      last = i;
      break;
    }
  }
  // The final message counts only when no tool work follows it (a response
  // that ended on a tool call has no final message yet).
  const workAfter = segs.slice(last + 1).some((s) => s.kind === 'steps' || (s.kind === 'block' && s.block.kind === 'tasks'));
  const tailFrom = last > first && !workAfter ? last : -1;
  const head = [segs[first]];
  const tail = tailFrom >= 0 ? segs.slice(tailFrom) : [];
  const body = segs.filter((_, i) => i !== first && (tailFrom < 0 || i < tailFrom));
  if (!body.some((s) => s.kind === 'steps')) return null;
  return { head, body, tail };
}

/** Steps (tool calls + subagents), failures and thinking markers in a fold body. */
export function foldStats(segs: Segment[]): { steps: number; failed: number; thinking: number } {
  let steps = 0;
  let failed = 0;
  let thinking = 0;
  for (const s of segs) {
    if (s.kind !== 'steps') continue;
    for (const b of s.steps) {
      if (b.kind === 'tool_call') {
        steps++;
        if (b.result != null && !b.result.ok) failed++;
      } else if (b.kind === 'subagent') steps++;
      else if (b.kind === 'thinking') thinking += b.count;
    }
  }
  return { steps, failed, thinking };
}

/** +added / −deleted lines of a unified patch, without building a DiffResp. */
export function patchStats(patch: string): { add: number; del: number } {
  let add = 0;
  let del = 0;
  let inHunk = false;
  let at = 0;
  while (at <= patch.length) {
    let end = patch.indexOf('\n', at);
    if (end < 0) end = patch.length;
    const c = patch.charCodeAt(at);
    if (c === 64 /* @ */ && patch.startsWith('@@', at)) inHunk = true;
    // File headers end a hunk (as in `patchToDiff`).
    else if (patch.startsWith('diff --git', at) || patch.startsWith('--- ', at) || patch.startsWith('+++ ', at)) inHunk = false;
    else if (inHunk && c === 43 /* + */) add++;
    else if (inHunk && c === 45 /* - */) del++;
    at = end + 1;
  }
  return { add, del };
}

/** A synthetic −old/+new patch for an Edit call whose result carries no
 *  structured patch (older records, a failed edit). */
export function editInputPatch(b: ToolCallBlock): string | null {
  const input = b.input;
  if (b.tool !== 'edit' || input == null || typeof input !== 'object') return null;
  const o = input as { old_string?: unknown; new_string?: unknown; file_path?: unknown };
  if (typeof o.old_string !== 'string' || typeof o.new_string !== 'string') return null;
  const oldL = o.old_string.split('\n');
  const newL = o.new_string.split('\n');
  const path = typeof o.file_path === 'string' ? o.file_path : 'file';
  return [`--- a/${path}`, `+++ b/${path}`, `@@ -1,${oldL.length} +1,${newL.length} @@`, ...oldL.map((l) => `-${l}`), ...newL.map((l) => `+${l}`)].join('\n');
}

/** The content a Write call put on disk (from its input). */
export function writtenContent(b: ToolCallBlock): string | null {
  if (b.tool !== 'write' || b.input == null || typeof b.input !== 'object') return null;
  const c = (b.input as { content?: unknown }).content;
  return typeof c === 'string' ? c : null;
}

/** The file a read / edit / write call acted on. */
export function toolFilePath(b: ToolCallBlock): string | null {
  if (b.tool !== 'read' && b.tool !== 'edit' && b.tool !== 'write') return null;
  return str(b.input, 'file_path', 'path', 'notebook_path') || b.result?.file_path || null;
}

/** One file a response changed. */
export interface ChangedFile {
  path: string;
  add: number;
  del: number;
  /** Written from scratch (no patch; its whole content is new). */
  created: boolean;
  /** Unified patches in call order (a Write's content becomes an all-`+` patch). */
  patches: string[];
  /** The last full content written, when a Write produced it. */
  content: string | null;
}

/** Files changed by a response's successful edit / write calls, in first-touch
 *  order, with summed line stats. */
export function changedFiles(blocks: Block[]): ChangedFile[] {
  const byPath = new Map<string, ChangedFile>();
  for (const b of blocks) {
    if (b.kind !== 'tool_call' || (b.tool !== 'edit' && b.tool !== 'write')) continue;
    if (b.result != null && !b.result.ok) continue;
    const path = toolFilePath(b);
    if (!path) continue;
    let f = byPath.get(path);
    if (!f) {
      f = { path, add: 0, del: 0, created: false, patches: [], content: null };
      byPath.set(path, f);
    }
    const patch = b.result?.patch ?? editInputPatch(b);
    if (patch) {
      const st = patchStats(patch);
      f.add += st.add;
      f.del += st.del;
      f.patches.push(patch);
    } else {
      const content = writtenContent(b);
      if (content != null) {
        const lines = content.replace(/\n$/, '').split('\n');
        f.add += lines.length;
        f.created = f.patches.length === 0;
        f.content = content;
        f.patches.push([`--- /dev/null`, `+++ b/${path}`, `@@ -0,0 +1,${lines.length} @@`, ...lines.map((l) => `+${l}`)].join('\n'));
      }
    }
  }
  return [...byPath.values()];
}

/** The diff of a set of changed files (one DiffResp file per patch run). */
export function changedDiff(files: ChangedFile[]): DiffResp {
  const out: FileDiff[] = [];
  for (const f of files) {
    const merged = patchToDiff(f.patches.join('\n'), f.path);
    // Every patch of one path folds into ONE file entry (several edits).
    const hunks = merged.files.flatMap((x) => x.hunks);
    out.push({ path: f.path, old_path: null, is_binary: false, hunks, added: f.add, deleted: f.del });
  }
  return { files: out };
}

/** Absolute path for a reference printed relative to the session's cwd. */
export function resolvePath(path: string, cwd: string | null | undefined): string {
  if (path.startsWith('/') || path.startsWith('~/')) return path;
  if (!cwd) return path;
  return `${cwd.replace(/\/$/, '')}/${path.replace(/^\.\//, '')}`;
}

/** A path shown relative to the session's cwd when it lies inside it. */
export function relPath(path: string, cwd: string | null | undefined): string {
  if (!cwd) return path;
  const root = cwd.replace(/\/$/, '') + '/';
  return path.startsWith(root) ? path.slice(root.length) : path;
}

/** Local calendar day key ("2026-09-28"), or '' for no/invalid timestamp. */
export function dayKey(ts: string | null): string {
  if (!ts) return '';
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return '';
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** "Today" / "Yesterday" / "Mon, Sep 28" / "Sep 28, 2025" for a day divider. */
export function fmtDay(ts: string | null, now: Date = new Date()): string {
  if (!ts) return '';
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return '';
  const k = dayKey(ts);
  if (k === dayKey(now.toISOString())) return 'Today';
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (k === dayKey(y.toISOString())) return 'Yesterday';
  const sameYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleDateString(undefined, sameYear ? { weekday: 'short', month: 'short', day: 'numeric' } : { month: 'short', day: 'numeric', year: 'numeric' });
}

/** Token line pieces for a response's usage (only the kinds it has). */
export interface UsagePart {
  key: 'input' | 'thinking' | 'output' | 'cache_read' | 'cache_write';
  label: string;
  value: number;
  title: string;
}
export function usageParts(u: TurnUsage | null | undefined): UsagePart[] {
  if (!u) return [];
  const answer = Math.max(0, u.output_tokens - u.thinking_tokens);
  const parts: UsagePart[] = [
    { key: 'input', label: 'in', value: u.input_tokens, title: 'Input tokens — your message, tool results and context sent to the model (not counting the cache)' },
    { key: 'thinking', label: 'thinking', value: u.thinking_tokens, title: 'Thinking tokens — output the model spent reasoning before answering (the text is not saved)' },
    { key: 'output', label: 'out', value: answer, title: 'Output tokens — the visible answer and tool calls (output minus thinking)' },
    { key: 'cache_read', label: 'cache read', value: u.cache_read_tokens, title: 'Cache read — context re-used from the prompt cache (billed at a discount)' },
    { key: 'cache_write', label: 'cache write', value: u.cache_creation_tokens, title: 'Cache write — context added to the prompt cache this turn' },
  ];
  return parts.filter((p) => p.value > 0 || p.key === 'input' || p.key === 'output');
}
