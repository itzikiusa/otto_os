// Manual-run input helpers (W2). The Run… "Suggest" template is GUIDANCE: it
// is shown as the textarea placeholder and never pre-filled as live values —
// a sent `result_chat: "<channel id — optional>"` streamed a run's progress to
// a nonexistent Slack chat, and `working_directory: "~/path/to/repo"` gave the
// agents a cwd that doesn't exist. Placeholder values are rejected before send.

/** Graph node kinds that need repos declared in the run input. */
const REPO_KINDS = ['review_run', 'git_pr'];
const STORY_KINDS = ['product_analyze', 'product_rewrite', 'product_plan', 'product_publish'];

/** The annotated example for the textarea placeholder: every key this graph
 *  can use, values written as `<…>` so nothing in it is runnable as-is. */
export function runInputExample(kinds: Set<string>): string {
  const obj: Record<string, unknown> = {
    working_directory: '<repo path — optional, defaults to the workspace root>',
  };
  if (REPO_KINDS.some((k) => kinds.has(k))) {
    obj.repos = [
      { repo: '<repo id, name, or path>', type: 'branch', name: '<work branch>', source: '<target branch — optional>' },
    ];
  }
  if (STORY_KINDS.some((k) => kinds.has(k))) obj.story_id = '<product story id>';
  obj.msg = '<what you want done — instructions for the agents>';
  obj.goals = ['<e.g. 100% test coverage>'];
  obj.result_channel = '<slack | telegram — optional>';
  obj.result_chat = '<channel id — optional>';
  return JSON.stringify(obj, null, 2);
}

/** "Suggest": only the keys THIS graph needs, as `<…>` slots to replace.
 *  Never outward-facing keys (`result_*`) or a fake ticket/path. */
export function runInputSkeleton(kinds: Set<string>): string {
  const obj: Record<string, unknown> = {};
  if (REPO_KINDS.some((k) => kinds.has(k))) {
    obj.repos = [{ repo: '<repo id, name, or path>', type: 'branch', name: '<work branch>' }];
  }
  if (STORY_KINDS.some((k) => kinds.has(k))) obj.story_id = '<product story id>';
  obj.msg = '<what you want done>';
  return JSON.stringify(obj, null, 2);
}

/** Values that are template text, not real input. */
const PLACEHOLDER_VALUES = new Set(['PROJ-0000', '~/path/to/repo']);

function isPlaceholder(v: string): boolean {
  const t = v.trim();
  return /^<.*>$/s.test(t) || PLACEHOLDER_VALUES.has(t);
}

/** The dotted path of the first placeholder value in the input, or null. */
export function findPlaceholder(value: unknown, path = ''): string | null {
  if (typeof value === 'string') return isPlaceholder(value) ? path || '(input)' : null;
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      const hit = findPlaceholder(value[i], `${path}[${i}]`);
      if (hit) return hit;
    }
    return null;
  }
  if (value && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) {
      const hit = findPlaceholder(v, path ? `${path}.${k}` : k);
      if (hit) return hit;
    }
  }
  return null;
}

/** Where a manual run's results go (mirrors the engine's
 *  `resolve_chat_target` + webhook delivery), for the outward-facing notice
 *  above Run. Empty when the run posts nowhere. */
export function resultDestinations(input: unknown): string[] {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return [];
  const o = input as Record<string, unknown>;
  const str = (k: string): string => (typeof o[k] === 'string' ? (o[k] as string).trim() : '');
  const out: string[] = [];
  const chat = str('result_chat');
  if (chat) {
    const ch = str('result_channel') || str('channel');
    const name = ch === 'slack' ? 'Slack' : ch === 'telegram' ? 'Telegram' : '';
    if (name) {
      const thread = str('result_thread');
      out.push(`${name} chat ${chat}${thread ? ` (thread ${thread})` : ''}`);
    }
  }
  const hook = str('result_webhook');
  if (hook) out.push(`webhook ${hook}`);
  return out;
}
