// Pure helpers extracted from GroupsTab.svelte (state badge class + offset
// reset payload/label formatting).
export type ResetMode = 'earliest' | 'latest' | 'offset' | 'timestamp';

export function groupStateClass(s: string): string {
  const v = s.toLowerCase();
  if (v.includes('stable')) return 'ok';
  if (v.includes('empty') || v.includes('dead')) return 'dim';
  return 'warn';
}

export function buildResetBody(opts: {
  mode: ResetMode;
  offset: number;
  ts: string;
  topic: string;
  confirm: boolean;
}): Record<string, unknown> {
  let body: Record<string, unknown>;
  if (opts.mode === 'offset') {
    body = { mode: 'offset', offset: Number(opts.offset), confirm: opts.confirm };
  } else if (opts.mode === 'timestamp') {
    body = { mode: 'timestamp', timestamp_ms: new Date(opts.ts).getTime(), confirm: opts.confirm };
  } else {
    body = { mode: opts.mode, confirm: opts.confirm };
  }
  if (opts.topic) body['topic'] = opts.topic;
  return body;
}

export function resetTargetText(mode: ResetMode, offset: number, ts: string): string {
  if (mode === 'offset') return `to offset ${Number(offset)}`;
  if (mode === 'timestamp') return `to ${new Date(ts).toLocaleString()}`;
  return `to ${mode}`;
}
