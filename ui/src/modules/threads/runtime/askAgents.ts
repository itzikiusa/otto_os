// "Ask another agent" from a runtime thread's composer: typing `@` and part
// of a provider or model name offers them as chips (T3 Code's model mentions,
// apps/web/src/components/chat/composerModelMentions.ts, MIT, Copyright (c)
// 2026 T3 Tools Inc. — `@opus`, `@gpt`, words joined with `:` `/` `-`). In
// Otto a picked chip sends the message to a NEW thread on that agent, linked
// to this one (POST /runtime-sessions/{id}/children). Pure
// (unit/threadsChildren.test.ts).
import type { AskChildTarget, RuntimeProvider } from '../../../lib/api/types';

/** The runtime providers a child thread can run on. */
export const ASK_PROVIDERS: readonly RuntimeProvider[] = ['claude', 'codex', 'gemini', 'grok', 'opencode', 'pi'];
/** Agents one ask reaches (the daemon's ASK_CHILD_MAX_TARGETS). */
export const ASK_MAX_TARGETS = 4;

export interface AskOption {
  provider: RuntimeProvider;
  /** null = the provider's default model. */
  model: string | null;
  label: string;
  /** The provider's name, beside a model's label. */
  detail: string;
}

/** A trailing `@word` being typed: its text and where the `@` sits. */
export function mentionAt(text: string): { query: string; start: number } | null {
  const m = /(^|\s)@([\w.:/-]*)$/.exec(text);
  return m ? { query: m[2].toLowerCase(), start: m.index + m[1].length } : null;
}

/** `text` without the `@word` that starts at `start`. */
export function dropMention(text: string, start: number): string {
  return text.slice(0, start).replace(/\s+$/, start > 0 ? ' ' : '');
}

/** Options for `@query`, best first: every provider (its default model) and
 *  its known models. Each word of the query must match the provider's name,
 *  the model's id or its label. */
export function mentionOptions(
  query: string,
  modelsOf: (p: RuntimeProvider) => readonly { id: string; label: string }[],
  nameOf: (p: RuntimeProvider) => string,
  limit = 8,
): AskOption[] {
  const words = query.toLowerCase().split(/[:/\s-]+/).filter(Boolean);
  const all: AskOption[] = [];
  for (const p of ASK_PROVIDERS) {
    const name = nameOf(p);
    all.push({ provider: p, model: null, label: name, detail: 'default model' });
    for (const m of modelsOf(p)) all.push({ provider: p, model: m.id, label: m.label, detail: name });
  }
  const hay = (o: AskOption): string => `${o.provider} ${o.model ?? ''} ${o.label} ${o.detail}`.toLowerCase();
  const score = (o: AskOption): number => {
    const h = hay(o);
    if (!words.every((w) => h.includes(w))) return -1;
    // A model id or label that starts with the first word ranks first.
    const first = words[0] ?? '';
    return (o.model?.toLowerCase().startsWith(first) || o.label.toLowerCase().startsWith(first) ? 2 : 0) + (o.model ? 0 : 1);
  };
  return all
    .map((o, i) => ({ o, s: score(o), i }))
    .filter((x) => x.s >= 0)
    .sort((a, b) => b.s - a.s || a.i - b.i)
    .slice(0, limit)
    .map((x) => x.o);
}

/** Add `t` to `list` (no duplicate, at most ASK_MAX_TARGETS). */
export function addTarget(list: readonly AskChildTarget[], t: AskChildTarget): AskChildTarget[] {
  const same = (a: AskChildTarget): boolean => a.provider === t.provider && (a.model ?? '') === (t.model ?? '') && (a.effort ?? '') === (t.effort ?? '');
  if (list.some(same) || list.length >= ASK_MAX_TARGETS) return [...list];
  return [...list, t];
}
