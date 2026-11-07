// Who a personal agent's thread belongs to and what started it, read from the
// server-owned thread meta (`personal_agent`, `personal_agent_name`,
// `personal_agent_chat`, `trigger`, `routine`, `delegated_by_agent`, …; see
// `run_meta` / `chat_thread` in crates/otto-assistant/src/personal_agent_threads.rs
// and `delegate` in personal_agent_team.rs). Shared by the Threads sidebar and
// the Personal Agents page so both say the same thing. Pure: no stores.

export type AgentThreadKind = 'chat' | 'scheduled' | 'manual' | 'proactive' | 'delegated';

export interface AgentThreadBadge {
  kind: AgentThreadKind;
  /** Short chip text: "Chat", "Scheduled · Morning brief", "Delegated by Otto". */
  label: string;
  /** Longer tooltip. */
  title: string;
}

type Meta = Record<string, unknown> | null | undefined;

const str = (m: Meta, k: string): string | null => {
  const v = m?.[k];
  return typeof v === 'string' && v.trim() !== '' ? v.trim() : null;
};

/** Thread titles the daemon gives an agent's threads ("Chat: X", "Agent: X"). */
const TITLE_PREFIX = /^(?:Chat|Agent):\s*/;

/** The personal agent a thread runs as, or null for a plain thread. */
export function personalAgentOf(meta: Meta): string | null {
  return str(meta, 'source') === 'personal_agent' ? str(meta, 'personal_agent') : null;
}

/** The agent's display name: the meta's, else the title's ("Chat: Scout" → "Scout"). */
export function personalAgentName(meta: Meta, title: string): string | null {
  if (!personalAgentOf(meta)) return null;
  const fromMeta = str(meta, 'personal_agent_name');
  if (fromMeta) return fromMeta;
  const t = title.trim();
  return TITLE_PREFIX.test(t) ? t.replace(TITLE_PREFIX, '').trim() || null : null;
}

/** What started an agent's thread, as a chip. `nameOf` resolves an agent id
 *  (the delegating main agent) when the meta carries no name. */
export function agentThreadBadge(meta: Meta, nameOf?: (agentId: string) => string | undefined): AgentThreadBadge | null {
  if (!personalAgentOf(meta)) return null;
  if (meta?.personal_agent_chat === true) {
    return { kind: 'chat', label: 'Chat', title: 'Your conversation with this agent' };
  }
  const lead = str(meta, 'delegated_by_agent');
  if (lead) {
    const who = str(meta, 'delegated_by_name') ?? nameOf?.(lead) ?? 'the main agent';
    return { kind: 'delegated', label: `Delegated by ${who}`, title: `Work ${who} handed to this agent` };
  }
  const trigger = str(meta, 'trigger');
  const mode = str(meta, 'agent_mode');
  const routine = str(meta, 'routine');
  if (trigger === 'schedule' || mode === 'scheduled') {
    return {
      kind: 'scheduled',
      label: routine ? `Scheduled · ${routine}` : 'Scheduled',
      title: routine ? `A routine run: ${routine}` : 'A routine run on its schedule',
    };
  }
  if (trigger === 'proactive' || mode === 'proactive') {
    return {
      kind: 'proactive',
      label: 'Proactive',
      title: routine ? `Worked a standing goal in the background: ${routine}` : 'Worked a standing goal in the background (read-only)',
    };
  }
  if (str(meta, 'run_id')) return { kind: 'manual', label: 'Manual run', title: 'A run someone started with Run now' };
  return null;
}

/** A run's chip on the agent page, from its row (`trigger` / `mode`). */
export function runBadge(run: { trigger: string; mode?: string; schedule_id?: string | null }, routine?: string | null): AgentThreadBadge {
  if (run.mode === 'proactive' || run.trigger === 'proactive') {
    return { kind: 'proactive', label: 'Proactive', title: 'Worked a standing goal in the background (read-only)' };
  }
  if (run.trigger === 'schedule') {
    return {
      kind: 'scheduled',
      label: routine ? `Scheduled · ${routine}` : 'Scheduled',
      title: routine ? `A routine run: ${routine}` : 'A routine run on its schedule',
    };
  }
  return { kind: 'manual', label: 'Manual run', title: 'A run someone started with Run now' };
}

/** A routine's short name: its directive's first line, cut at the first
 *  sentence and at a word boundary (≤ 40 chars), markdown marks dropped. */
export function routineLabel(directive: string): string {
  const first = directive
    .split('\n')
    .map((l) => l.trim().replace(/^#+/, '').replace(/[*_`]+/g, '').trim())
    .find((l) => l !== '') ?? '';
  if (!first) return 'Routine';
  const sentence = first.replace(/^(.{12,}?[.!?])\s.*$/, '$1').replace(/[.:]$/, '');
  const chars = [...sentence];
  if (chars.length <= 40) return sentence;
  const cut = chars.slice(0, 40).join('');
  const space = cut.lastIndexOf(' ');
  return `${(space > 20 ? cut.slice(0, space) : cut).replace(/[,;:\s]+$/, '')}…`;
}
