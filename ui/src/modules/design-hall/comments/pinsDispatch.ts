import type { Session } from '../../../lib/api/types';
import { pin3dWords } from '../../../lib/design/scene3d/pin3d.ts';
import { plural } from '../../../lib/plural.ts';
import { pinWords, type Thread } from './pins.ts';

export const MAX_PINS_MARKDOWN_CHARS = 8_000;

export interface PinsChecklistInput {
  artifactId: string;
  artifactTitle: string;
  versionSeq?: number | null;
  threads: readonly Thread[];
}

/** Unresolved review threads, pinned comments (2D or 3D) first (preserving creation order within each bucket). */
export function collectOpenThreads(threads: readonly Thread[]): Thread[] {
  const open = threads.filter((t) => !t.root.resolved);
  const pinned = open.filter((t) => Boolean(t.root.pin || t.root.pin_3d));
  const general = open.filter((t) => !t.root.pin && !t.root.pin_3d);
  return [...pinned, ...general];
}

function cleanLine(raw: string, maxLen: number): string {
  const oneLine = raw.replace(/\s+/g, ' ').trim();
  if (oneLine.length <= maxLen) return oneLine;
  return `${oneLine.slice(0, maxLen - 1)}…`;
}

/** Build a structured, size-capped Markdown checklist of unresolved design review pins & notes. */
export function formatPinsMarkdown(input: PinsChecklistInput): string {
  const open = collectOpenThreads(input.threads);
  const seqSuffix = input.versionSeq != null ? `@v${input.versionSeq}` : '';
  const ref = `otto://design/${input.artifactId}${seqSuffix}`;
  const header = [
    `## Design Review Pins — ${input.artifactTitle || input.artifactId}`,
    `Design ref: \`${ref}\``,
    '',
    'Please address these open design review items:',
  ];

  if (open.length === 0) {
    return `${header.join('\n')}\n- (No open review comments.)\n`;
  }

  const lines: string[] = [...header];
  let total = lines.join('\n').length;
  let omitted = 0;

  for (const t of open) {
    const meta: string[] = [];
    if (t.root.pin_3d) {
      meta.push(pin3dWords(t.root.pin_3d));
    } else if (t.root.pin) {
      if (t.root.pin.frame) meta.push(`frame: ${t.root.pin.frame}`);
      meta.push(pinWords(t.root.pin));
    }
    if (t.root.version_seq != null) meta.push(`v${t.root.version_seq}`);
    const label = t.root.pin_3d ? `3D Pin #${t.n}` : t.root.pin ? `Pin #${t.n}` : `Comment #${t.n}`;
    const metaStr = meta.length ? ` (${meta.join(' · ')})` : '';
    const author = t.root.author_name?.trim() || 'Reviewer';
    const itemLines = [`- [ ] **${label}**${metaStr} — ${author}: ${cleanLine(t.root.body, 360)}`];
    for (const r of t.replies) {
      const rAuthor = r.author_name?.trim() || 'Reviewer';
      itemLines.push(`  - Reply (${rAuthor}): ${cleanLine(r.body, 220)}`);
    }
    const block = itemLines.join('\n');
    if (total + block.length + 64 > MAX_PINS_MARKDOWN_CHARS) {
      omitted++;
      continue;
    }
    lines.push(block);
    total += block.length + 1;
  }

  if (omitted > 0) {
    lines.push(`- …and ${plural(omitted, 'more open item')} (truncated to fit).`);
  }
  return `${lines.join('\n')}\n`;
}

/** Compile unresolved 3D polygon feedback into a concise regeneration prompt. */
export function formatRegenerateFeedbackPrompt(artifactTitle: string, threads: readonly Thread[]): string {
  const open = collectOpenThreads(threads);
  if (open.length === 0) return `Refine 3D model "${artifactTitle}".`;
  const bullets = open.slice(0, 12).map((t) => {
    const where = t.root.pin_3d ? `[${pin3dWords(t.root.pin_3d)}] ` : '';
    return `- ${where}${cleanLine(t.root.body, 220)}`;
  });
  return `Refine 3D model "${artifactTitle}" addressing this polygon review feedback:\n${bullets.join('\n')}`;
}

export type TargetSessionCandidate = Pick<Session, 'id' | 'title' | 'kind' | 'archived' | 'status'>;

/** Pick the best open foreground agent session to receive the checklist (active first). */
export function pickTargetAgentSession(
  sessions: readonly TargetSessionCandidate[],
  activeSessionId: string | null,
): TargetSessionCandidate | null {
  const liveAgent = (s: Pick<Session, 'kind' | 'archived' | 'status'>) =>
    s.kind === 'agent' && !s.archived && s.status !== 'exited';
  if (activeSessionId) {
    const active = sessions.find((s) => s.id === activeSessionId && liveAgent(s));
    if (active) return active;
  }
  return sessions.find(liveAgent) ?? null;
}
