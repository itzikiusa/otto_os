// The opening prompt of a side chat (`/btw`): the question plus a compact
// snapshot of the main thread — title, harness, folder, branch, changed files
// and the last few turns — so the side agent can answer without asking, and
// without touching the work the main thread is doing. Pure: unit-tested.
import type { Turn } from '../../lib/api/types';

export interface SideChatContext {
  title: string;
  harness: string;
  cwd: string | null;
  branch: string | null;
  changedFiles: readonly string[];
  turns: readonly Turn[];
}

const MAX_TURNS = 8;
const MAX_TURN_CHARS = 600;
const MAX_FILES = 30;
const MAX_CONTEXT_CHARS = 6000;

function clip(text: string, max: number): string {
  const t = text.replace(/\s+/g, ' ').trim();
  return t.length > max ? `${t.slice(0, max - 1)}…` : t;
}

/** One line per turn: its prose, plus the tools it ran (by title). */
export function summarizeTurn(t: Turn): string | null {
  const prose = t.blocks
    .filter((b): b is Extract<typeof b, { kind: 'text' }> => b.kind === 'text')
    .map((b) => b.md)
    .join(' ');
  const tools = t.blocks
    .filter((b): b is Extract<typeof b, { kind: 'tool_call' }> => b.kind === 'tool_call')
    .map((b) => b.title || b.name);
  const parts: string[] = [];
  if (prose.trim()) parts.push(clip(prose, MAX_TURN_CHARS));
  if (tools.length > 0) {
    const shown = tools.slice(0, 5).map((x) => clip(x, 60)).join('; ');
    parts.push(`[tools: ${shown}${tools.length > 5 ? `; +${tools.length - 5} more` : ''}]`);
  }
  if (parts.length === 0) return null;
  return `${t.role === 'user' ? 'User' : 'Assistant'}: ${parts.join(' ')}`;
}

export function buildSideChatPrompt(question: string, ctx: SideChatContext): string {
  const lines: string[] = [
    'This is a side chat. Answer the question below about the main thread described here.',
    'Do not edit files or run commands that change the workspace; the main thread keeps working in it.',
    '',
    `Main thread: "${ctx.title}" (${ctx.harness})${ctx.cwd ? ` in ${ctx.cwd}` : ''}${ctx.branch ? ` on branch ${ctx.branch}` : ''}.`,
  ];
  if (ctx.changedFiles.length > 0) {
    const shown = ctx.changedFiles.slice(0, MAX_FILES).join(', ');
    const more = ctx.changedFiles.length > MAX_FILES ? ` (+${ctx.changedFiles.length - MAX_FILES} more)` : '';
    lines.push(`Uncommitted changes: ${shown}${more}.`);
  }
  const recent = ctx.turns
    .slice(-MAX_TURNS)
    .map(summarizeTurn)
    .filter((x): x is string => x !== null);
  if (recent.length > 0) {
    // Newest last; drop the oldest until the snapshot fits.
    let body = recent;
    while (body.length > 1 && body.join('\n').length > MAX_CONTEXT_CHARS) body = body.slice(1);
    lines.push('', 'Recent main-thread conversation (newest last):', ...body);
  }
  lines.push('', `Question: ${question.trim()}`);
  return lines.join('\n');
}
