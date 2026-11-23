// Pure helpers for "Relay with recent output" between linked threads: extracts
// the latest assistant output from a loaded transcript or runtime projection,
// builds a compact 1-line preview for the context badge, and formats a clean
// quoted handoff draft for Otto's Message sheet (editable before sending).

import type { Turn } from '../../lib/api/types';

function stripAnsi(raw: string): string {
  return raw
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '')
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
    .replace(/\r\n?/g, '\n');
}

/** Latest non-empty assistant text block from a loaded transcript's turns. */
export function latestTranscriptAnswer(turns: readonly Turn[]): string {
  for (let i = turns.length - 1; i >= 0; i--) {
    const t = turns[i];
    if (t.role !== 'assistant') continue;
    const parts: string[] = [];
    for (const b of t.blocks) {
      if (b.kind === 'text') {
        const clean = stripAnsi(b.md).trim();
        if (clean) parts.push(clean);
      }
    }
    if (parts.length > 0) return parts.join('\n\n');
  }
  return '';
}

export interface RelayContextSummary {
  /** Cleaned excerpt suitable for quoting in a handoff message (capped). */
  snippet: string;
  /** Compact 1-line preview for the context badge. */
  preview: string;
}

/** Produce a bounded snippet + single-line badge preview from raw output text. */
export function summarizeRelayContext(raw: string, maxChars = 420): RelayContextSummary | null {
  const clean = stripAnsi(raw).trim();
  if (!clean) return null;
  const snippet = clean.length > maxChars ? `${clean.slice(0, Math.max(1, maxChars - 1)).trimEnd()}…` : clean;
  const oneLine = clean.replace(/\s+/g, ' ');
  const preview = oneLine.length > 84 ? `${oneLine.slice(0, 81).trimEnd()}…` : oneLine;
  return { snippet, preview };
}

/** Format a quoted handoff draft referencing the sender thread's recent output. */
export function buildRelayWithOutputDraft(fromTitle: string, snippet: string): string {
  const label = fromTitle.trim() || 'Previous thread';
  const quoted = snippet
    .trim()
    .split('\n')
    .map((line) => (line ? `> ${line}` : '>'))
    .join('\n');
  return `Context from “${label}”:\n${quoted}\n\n`;
}
