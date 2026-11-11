// Mock-mode Slack "act as me" (ui/src/lib/api/mock.ts routes it): two
// connected Slack workspaces, a per-person default with a workspace
// override, a small directory, and a send log the e2e reads back
// (`window.__mockSlackSent`). Nothing here talks to Slack.
/* eslint-disable @typescript-eslint/no-explicit-any */

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: any, q: URLSearchParams) => Out };

const teams = [
  { team_id: 'T1', team: 'Acme', url: 'https://acme.slack.com/', user_id: 'U1', user: 'dana', scopes: [], added_at: '', missing_scopes: [] },
  { team_id: 'T2', team: 'Globex', url: 'https://globex.slack.com/', user_id: 'U5', user: 'dana.g', scopes: [], added_at: '', missing_scopes: [] },
];
const users = [
  { id: 'U2', name: 'Maya Levi', handle: 'maya', is_bot: false },
  { id: 'U3', name: 'Dev Otto', handle: 'dev-otto', is_bot: false },
  { id: 'B1', name: 'Review Bot', handle: 'reviewbot', is_bot: true },
];
const conversations = [
  { id: 'C1', name: 'pr-reviews', kind: 'channel' },
  { id: 'C2', name: 'design-review', kind: 'private' },
];
const defaults: { global: string | null; ws: Record<string, string | null> } = { global: null, ws: {} };
const live = (t: string | null | undefined): string | null => (t && teams.some((x) => x.team_id === t) ? t : null);
const view = (ws?: string) => {
  const g = live(defaults.global);
  const w = ws ? live(defaults.ws[ws]) : null;
  return { global: g, ...(ws ? { workspace: w } : {}), effective: w ?? g };
};
function sentLog(): any[] {
  const w = window as any;
  return (w.__mockSlackSent ??= []);
}

export const slackMockRoutes: MockRoute[] = [
  { method: 'GET', re: /^\/integrations\/slack\/me$/, handle: () => ({ json: teams }) },
  { method: 'GET', re: /^\/integrations\/slack\/as-me-default$/, handle: () => ({ json: view() }) },
  {
    method: 'PUT', re: /^\/integrations\/slack\/as-me-default$/,
    handle: (_m, b) => { defaults.global = b?.team_id ?? null; return { json: view() }; },
  },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/integrations\/slack\/as-me-default$/, handle: (m) => ({ json: view(m[1]) }) },
  {
    method: 'PUT', re: /^\/workspaces\/([^/]+)\/integrations\/slack\/as-me-default$/,
    handle: (m, b) => { defaults.ws[m[1]] = b?.team_id ?? null; return { json: view(m[1]) }; },
  },
  {
    method: 'GET', re: /^\/workspaces\/([^/]+)\/integrations\/slack\/teams$/,
    handle: (m) => ({ json: { teams, bot: false, as_me_restricted: false, default_team: view(m[1]).effective } }),
  },
  {
    method: 'GET', re: /^\/workspaces\/[^/]+\/integrations\/slack\/users$/,
    handle: (_m, _b, q) => { const n = (q.get('q') ?? '').toLowerCase(); return { json: { users: users.filter((u) => u.name.toLowerCase().includes(n) || u.handle.includes(n)) } }; },
  },
  {
    method: 'GET', re: /^\/workspaces\/[^/]+\/integrations\/slack\/conversations$/,
    handle: (_m, _b, q) => { const n = (q.get('q') ?? '').toLowerCase(); return { json: { conversations: conversations.filter((c) => c.name.includes(n)) } }; },
  },
  {
    method: 'POST', re: /^\/workspaces\/[^/]+\/integrations\/slack\/messages$/,
    handle: (_m, b) => {
      if (b?.as !== 'user' || !b?.channel || !b?.text) return { status: 400, json: { code: 'invalid', message: 'channel and text are required' } };
      const team = live(b.team) ?? 'T1';
      sentLog().push({ team, channel: b.channel, text: b.text });
      return { json: { channel: b.channel, ts: '1730800000.000200', permalink: 'https://acme.slack.com/archives/C1/p1730800000000200', as: 'user', team_id: team } };
    },
  },
];
