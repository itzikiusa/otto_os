// A node card's icon and second line (pure; ux.md §9.2). The icon carries the
// family on a neutral tile; the line says what the step does in words —
// "Claude · Default · High", "6 lenses × Claude, Codex · pass ≥ 80",
// "Runs pr_gates.sh · 15 min limit" — never the kind's raw name.
import type { WorkflowNode } from '../../../lib/api/types';
import type { IconName } from '../../../lib/components/Icon.svelte';

type Params = Record<string, unknown>;

/** Kinds that run an agent: the only ones with provider / model controls. */
export const AGENT_KINDS: readonly string[] = [
  'agent_prompt', 'prepare_context', 'product_analyze', 'product_rewrite', 'product_plan', 'canvas', 'git_pr', 'self_improve',
];

const ICONS: Record<string, IconName> = {
  manual_trigger: 'play',
  review_run: 'eye',
  git_checkout: 'branch',
  script: 'command',
  post_review_comments: 'comment',
  condition: 'function',
  loop: 'refresh',
  delay: 'clock',
  human_approval: 'shield',
  budget_gate: 'gauge',
  check: 'target',
  http_request: 'globe',
  api_run: 'globe',
  db_query: 'db',
  broker_peek: 'server',
  channel_notify: 'bell',
  log: 'file',
  transform: 'file',
};

export function nodeIcon(kind: string): IconName {
  return ICONS[kind] ?? (AGENT_KINDS.includes(kind) ? 'assistant' : 'box');
}

const PROVIDER_NAMES: Record<string, string> = {
  claude: 'Claude', codex: 'Codex', agy: 'Agy', gemini: 'Gemini', grok: 'Grok', opencode: 'OpenCode', pi: 'Pi', muse: 'Muse',
};
/** "Claude", "Codex", a custom provider's own slug capitalised. */
export function providerName(p: string): string {
  return PROVIDER_NAMES[p] ?? (p ? p[0].toUpperCase() + p.slice(1) : 'Default');
}

const EFFORT_NAMES: Record<string, string> = { low: 'Low', medium: 'Medium', high: 'High', xhigh: 'Extra high', max: 'Max', ultra: 'Ultra' };

/** "Claude · Default · High": provider, model (empty = Default), effort. */
export function modelChip(p: Params, defaultProvider = ''): string {
  const prov = str(p.provider) || defaultProvider;
  return [providerName(prov), str(p.model) || 'Default', EFFORT_NAMES[str(p.effort)] ?? 'Default'].join(' · ');
}

const str = (v: unknown): string => (typeof v === 'string' ? v.trim() : '');
const list = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);
const cut = (s: string, n: number): string => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

function waits(ms: number): string {
  if (ms >= 3_600_000) return `${Math.round(ms / 3_600_000)}h`;
  if (ms >= 60_000) return `${Math.round(ms / 60_000)}m`;
  return `${Math.round(ms / 1000)}s`;
}

/** The script a command runs ("pr_gates.sh"), else its first word. */
function scriptName(command: string): string {
  const words = command.split(/\s+/).filter(Boolean);
  const file = words.find((w) => /\.(sh|py|js|mjs|ts|rb|pl)$/.test(w)) ?? words[0] ?? '';
  return file.split('/').pop() ?? file;
}

export interface SummaryContext {
  /** The workflow's other steps, to name a continued thread's step. */
  nodes?: readonly WorkflowNode[];
  /** The provider an empty pick resolves to. */
  defaultProvider?: string;
  /** Kinds of the workflow's triggers (chat, schedule, webhook, event). */
  triggerKinds?: readonly string[];
}

const TRIGGER_WORDS: Record<string, string> = { chat: 'Chat', schedule: 'Schedule', webhook: 'Webhook', event: 'Event' };

export function nodeSummary(node: WorkflowNode, ctx: SummaryContext = {}): string {
  const p = (node.params ?? {}) as Params;
  switch (node.kind) {
    case 'manual_trigger': {
      const by = [...new Set((ctx.triggerKinds ?? []).map((k) => TRIGGER_WORDS[k] ?? k))];
      const n = Array.isArray(p.inputs) ? p.inputs.length : 0;
      return [by.length ? `${by.join(', ')} or manual` : 'Manual', n ? `${n} ${n === 1 ? 'input' : 'inputs'}` : ''].filter(Boolean).join(' · ');
    }
    case 'review_run': {
      const rows = Array.isArray(p.reviewers) ? (p.reviewers as Params[]) : [];
      const provs = [...new Set(rows.flatMap((r) => list(r.providers)))].map(providerName);
      const lenses = rows.length ? `${rows.length} ${rows.length === 1 ? 'lens' : 'lenses'}${provs.length ? ` × ${provs.join(', ')}` : ''}` : 'Default lenses';
      return `${lenses} · pass ≥ ${typeof p.threshold === 'number' ? p.threshold : 80}`;
    }
    case 'git_checkout':
      return p.mode === 'branch' ? 'Branch from input, then base' : 'PR from input, then branch';
    case 'script': {
      const cmd = str(p.command);
      const min = Math.round((typeof p.timeout_s === 'number' ? p.timeout_s : 900) / 60);
      return cmd ? `Runs ${scriptName(cmd)} · ${min} min limit` : 'No command yet';
    }
    case 'post_review_comments': {
      const inline = p.inline === undefined ? ['bug', 'warn'] : list(p.inline);
      const parts = [inline.length ? `${inline.join(' and ')} inline` : 'Nothing inline'];
      if (p.info !== 'skip') parts.push('notes combined');
      if (p.summary !== false) parts.push('summary');
      return parts.join(' · ').replace(/^./, (c) => c.toUpperCase());
    }
    case 'check': {
      const crit = Array.isArray(p.criteria) ? (p.criteria as Params[]) : [];
      const you = crit.filter((c) => c.verify === 'human').length;
      return crit.length ? `${crit.length} ${crit.length === 1 ? 'criterion' : 'criteria'}${you ? ` · ${you} by you` : ''}` : 'No criteria yet';
    }
    case 'condition':
      return str(p.expr) || 'No expression yet';
    case 'loop': {
      const max = typeof p.max_iterations === 'number' ? p.max_iterations : null;
      const until = str(p.until);
      return [max ? `Up to ${max} times` : 'Repeats', until ? `until ${until}` : ''].filter(Boolean).join(' · ');
    }
    case 'delay':
      return `Waits ${waits(typeof p.ms === 'number' ? p.ms : 0)}`;
    case 'human_approval':
      return str(p.message) ? `Asks: ${str(p.message)}` : 'Waits for your approval';
    case 'http_request':
    case 'api_run': {
      const url = str(p.url);
      let host = url;
      try {
        host = new URL(url).host || url;
      } catch {
        /* not a full URL yet: show what was typed */
      }
      return `${(str(p.method) || 'GET').toUpperCase()} ${host || '…'}`;
    }
    case 'db_query':
      return str(p.statement) ? cut(str(p.statement).replace(/\s+/g, ' '), 40) : 'No query yet';
    case 'broker_peek':
      return str(p.topic) || 'No topic yet';
    case 'channel_notify':
      return [str(p.channel) || 'Chat', str(p.message) ? cut(str(p.message), 40) : ''].filter(Boolean).join(' · ');
    case 'log':
      return 'Records the data in the run log';
    case 'transform':
      return 'Sets fields on the data';
  }
  if (AGENT_KINDS.includes(node.kind)) {
    const from = str(p.continue_from);
    const src = from ? ctx.nodes?.find((n) => n.id === from) : undefined;
    return modelChip(p, ctx.defaultProvider) + (src ? ` · continues ${src.name || src.id}` : '');
  }
  return '';
}
