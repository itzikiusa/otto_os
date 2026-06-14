// Deterministic natural-language parser for the ⌘K orchestrator box.
//
// Common intents — "open 2 claude sessions", "spawn a codex", "broadcast run
// the tests", "tell everyone to stop" — are recognized INSTANTLY with no LLM.
// Ported from loom's commandInterpreter: tolerant of filler words, number
// words, provider aliases, typos, and trailing nouns. Returns an ActionPlan
// directly (the same shape the AI planner produces) or null when it can't be
// sure — the caller then falls back to the AI planner if enabled.

import type { Action, ActionPlan } from './api/types';

const NUMBER_WORDS: Record<string, number> = {
  a: 1, an: 1, one: 1, two: 2, three: 3, four: 4, five: 5,
  six: 6, seven: 7, eight: 8, nine: 9, ten: 10,
  couple: 2, pair: 2, few: 3, several: 3,
};

// token → canonical provider id (must match the daemon's provider names)
const PROVIDER_ALIASES: Record<string, string> = {
  claude: 'claude', anthropic: 'claude',
  codex: 'codex', gpt: 'codex', gpt5: 'codex', 'gpt-5': 'codex', openai: 'codex', chatgpt: 'codex',
  agy: 'agy', antigravity: 'agy', gemini: 'agy', google: 'agy',
  shell: 'shell', terminal: 'shell', bash: 'shell', zsh: 'shell',
};
const PROVIDER_TOKENS = Object.keys(PROVIDER_ALIASES).sort((a, b) => b.length - a.length);

function editDistance(a: string, b: string): number {
  const m = a.length, n = b.length;
  if (Math.abs(m - n) > 2) return 99;
  const dp: number[] = Array.from({ length: n + 1 }, (_, i) => i);
  for (let i = 1; i <= m; i++) {
    let prev = dp[0];
    dp[0] = i;
    for (let j = 1; j <= n; j++) {
      const tmp = dp[j];
      dp[j] = a[i - 1] === b[j - 1] ? prev : 1 + Math.min(prev, dp[j], dp[j - 1]);
      prev = tmp;
    }
  }
  return dp[n];
}

// Common words that must never fuzzy-match a provider (they appear in spawn
// phrases and previously mis-resolved, e.g. "sessions" → codex).
const STOPWORDS = new Set([
  'session', 'sessions', 'terminal', 'terminals', 'pane', 'panes', 'agent',
  'agents', 'window', 'windows', 'tab', 'tabs', 'new', 'open', 'please',
]);

/** Map a token to a canonical provider, tolerating typos (clude, codx, …). */
export function resolveProvider(tok: string): string | null {
  if (PROVIDER_ALIASES[tok]) return PROVIDER_ALIASES[tok];
  if (tok.length < 4 || STOPWORDS.has(tok)) return null;
  // Tight fuzzy threshold (edit distance ≤ 1) so only real typos match —
  // unrelated nouns of similar length no longer resolve to a provider.
  let best: string | null = null;
  let bestDist = 2;
  for (const alias of PROVIDER_TOKENS) {
    if (alias.length < 4) continue;
    const d = editDistance(tok, alias);
    if (d < bestDist) {
      bestDist = d;
      best = PROVIDER_ALIASES[alias];
    }
  }
  return best;
}

const SPAWN_VERBS =
  /\b(open|spawn|launch|start|create|boot|add|fire up|spin up|give me|i want|i need|make|run)\b/;
const BROADCAST_VERBS = /\b(broadcast|tell|send|ask|say|message|prompt)\b/;
const ALL_TARGETS = /\b(all|everyone|every one|everybody|them|all of them|each)\b/;

function wordToCount(tok: string): number | null {
  if (/^\d+$/.test(tok)) return parseInt(tok, 10);
  if (tok in NUMBER_WORDS) return NUMBER_WORDS[tok];
  return null;
}

function normalize(s: string): string {
  return s.toLowerCase().replace(/\s+/g, ' ').trim();
}

/** Scan for "<count> <provider>" pairs anywhere in the text. */
function extractSpawnItems(text: string): { provider: string; count: number }[] {
  const items: { provider: string; count: number }[] = [];
  const tokens = text.split(/[\s,]+/).filter(Boolean);
  for (let i = 0; i < tokens.length; i++) {
    const provider = resolveProvider(tokens[i].replace(/[^a-z0-9-]/g, ''));
    if (!provider) continue;
    // Look back a few tokens for a count (skipping "new"/articles).
    let count = 1;
    for (let k = i - 1; k >= Math.max(0, i - 3); k--) {
      const t = tokens[k].replace(/[^a-z0-9]/g, '');
      if (t === 'new' || t === '') continue;
      const c = wordToCount(t);
      if (c !== null) {
        count = c;
        break;
      }
      if (resolveProvider(t)) break; // hit another provider — stop
    }
    items.push({ provider, count });
  }
  // Merge duplicate providers.
  const merged = new Map<string, number>();
  for (const it of items) merged.set(it.provider, (merged.get(it.provider) ?? 0) + it.count);
  return [...merged.entries()].map(([provider, count]) => ({ provider, count }));
}

/**
 * Parse `input` into an ActionPlan deterministically, or return null when the
 * intent is unclear (caller may fall back to the AI planner).
 */
export function parseCommand(input: string): ActionPlan | null {
  const text = normalize(input);
  if (text === '') return null;

  const hasSpawn = SPAWN_VERBS.test(text);
  const items = extractSpawnItems(text);

  // Spawn: a verb + at least one provider, OR a provider with an explicit count.
  if (items.length > 0 && (hasSpawn || items.some((i) => i.count > 1))) {
    const actions: Action[] = items
      .filter((i) => i.count > 0 && i.provider)
      .map((i) => ({ action: 'spawn_sessions', provider: i.provider, count: Math.min(i.count, 12) }));
    if (actions.length > 0) return actions;
  }

  // Broadcast: "tell/broadcast/send … <message>" to all sessions.
  const bcast = text.match(BROADCAST_VERBS);
  if (bcast && ALL_TARGETS.test(text)) {
    // Take everything after the broadcast verb as the message, stripping a
    // leading "all/everyone to" politely.
    const idx = input.toLowerCase().indexOf(bcast[0]);
    let msg = input.slice(idx + bcast[0].length).trim();
    msg = msg.replace(/^(all|everyone|everybody|them|each)\s+(of them\s+)?(to\s+)?/i, '').trim();
    msg = msg.replace(/^(to\s+)/i, '').trim();
    if (msg !== '') return [{ action: 'broadcast', text: msg }];
  }

  return null;
}
