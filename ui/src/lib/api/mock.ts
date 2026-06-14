// Mock layer — lets the whole SPA run with no daemon. Activated when
// `import.meta.env.VITE_OTTO_MOCK === '1'` or `localStorage.otto_mock === '1'`.
// It monkey-patches window.fetch (for /api/v1/*) and window.WebSocket (for
// /ws/*) so client.ts stays untouched.

import type {
  BranchInfo,
  CommitInfo,
  Connection,
  DiffResp,
  FileDiff,
  GitAccount,
  Id,
  MemberEntry,
  MetaResp,
  PrComment,
  PrDetail,
  PrSummary,
  Repo,
  RepoStatusResp,
  Session,
  User,
  Workspace,
  WorkspaceWithRole,
} from './types';
import { base64ToText, textToBytes } from '../b64';

export function mockEnabled(): boolean {
  try {
    if (import.meta.env.VITE_OTTO_MOCK === '1') return true;
    return localStorage.getItem('otto_mock') === '1';
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

let seq = 100;
function nid(prefix: string): Id {
  return `${prefix}_${(seq++).toString(36).toUpperCase().padStart(6, '0')}`;
}

const NOW = Date.now();
function ago(mins: number): string {
  return new Date(NOW - mins * 60_000).toISOString();
}

const users: User[] = [
  {
    id: 'usr_root',
    username: 'root',
    display_name: 'Root',
    is_root: true,
    disabled: false,
    created_at: ago(60 * 24 * 30),
  },
  {
    id: 'usr_dana',
    username: 'dana',
    display_name: 'Dana K.',
    is_root: false,
    disabled: false,
    created_at: ago(60 * 24 * 12),
  },
  {
    id: 'usr_omer',
    username: 'omer',
    display_name: 'Omer L.',
    is_root: false,
    disabled: true,
    created_at: ago(60 * 24 * 8),
  },
];

const workspaces: Workspace[] = [
  {
    id: 'wsp_otto',
    name: 'otto',
    root_path: '/Users/dev/otto',
    settings: { notes: '## Otto v1\n\n- [ ] finish diff viewer\n- [ ] palette polish' },
    archived: false,
    created_at: ago(60 * 24 * 20),
  },
  {
    id: 'wsp_casino',
    name: 'casino-platform',
    root_path: '/Users/dev/casino',
    settings: {},
    archived: false,
    created_at: ago(60 * 24 * 5),
  },
];

const members: Record<Id, MemberEntry[]> = {
  wsp_otto: [
    { user_id: 'usr_root', username: 'root', display_name: 'Root', role: 'admin' },
    { user_id: 'usr_dana', username: 'dana', display_name: 'Dana K.', role: 'editor' },
  ],
  wsp_casino: [
    { user_id: 'usr_root', username: 'root', display_name: 'Root', role: 'admin' },
    { user_id: 'usr_dana', username: 'dana', display_name: 'Dana K.', role: 'viewer' },
  ],
};

const sessions: Session[] = [
  {
    id: 'ses_claude1',
    workspace_id: 'wsp_otto',
    kind: 'agent',
    provider: 'claude',
    title: 'claude #1',
    status: 'working',
    cwd: '/Users/dev/otto',
    provider_session_id: 'c0ffee-1',
    connection_id: null,
    created_by: 'usr_root',
    created_at: ago(180),
    last_active_at: ago(0),
    archived: false,
    meta: {},
  },
  {
    id: 'ses_codex1',
    workspace_id: 'wsp_otto',
    kind: 'agent',
    provider: 'codex',
    title: 'codex #1',
    status: 'idle',
    cwd: '/Users/dev/otto/ui',
    provider_session_id: null,
    connection_id: null,
    created_by: 'usr_root',
    created_at: ago(95),
    last_active_at: ago(12),
    archived: false,
    meta: {},
  },
  {
    id: 'ses_shell1',
    workspace_id: 'wsp_otto',
    kind: 'agent',
    provider: 'shell',
    title: 'shell #1',
    status: 'exited',
    cwd: '/Users/dev/otto',
    provider_session_id: null,
    connection_id: null,
    created_by: 'usr_dana',
    created_at: ago(300),
    last_active_at: ago(40),
    archived: false,
    meta: { exit_code: 0 },
  },
  {
    id: 'ses_redis1',
    workspace_id: 'wsp_casino',
    kind: 'connection',
    provider: 'redis',
    title: 'staging redis',
    status: 'reconnectable',
    cwd: '/Users/dev/casino',
    provider_session_id: null,
    connection_id: 'con_redis',
    created_by: 'usr_root',
    created_at: ago(600),
    last_active_at: ago(120),
    archived: false,
    meta: {},
  },
];

const connections: Connection[] = [
  {
    id: 'con_ssh',
    workspace_id: 'wsp_otto',
    name: 'build box',
    kind: 'ssh',
    params: { host: 'build.internal', port: 22, user: 'dev', identity_file: '~/.ssh/id_ed25519' },
    secret_ref: null,
    first_command: null,
    section_id: null,
    created_by: 'usr_root',
    created_at: ago(60 * 24 * 4),
  },
  {
    id: 'con_mysql',
    workspace_id: 'wsp_casino',
    name: 'staging mysql',
    kind: 'mysql',
    params: { host: 'db.example.internal', port: 3306, user: 'reader', db: 'app_db' },
    secret_ref: 'conn-con_mysql',
    first_command: 'SHOW TABLES;',
    section_id: null,
    created_by: 'usr_root',
    created_at: ago(60 * 24 * 3),
  },
  {
    id: 'con_redis',
    workspace_id: 'wsp_casino',
    name: 'staging redis',
    kind: 'redis',
    params: { host: 'redis.example.internal', port: 6379, db: 0 },
    secret_ref: 'conn-con_redis',
    first_command: 'PING',
    section_id: null,
    created_by: 'usr_root',
    created_at: ago(60 * 24 * 3),
  },
  {
    id: 'con_click',
    workspace_id: null,
    name: 'analytics clickhouse',
    kind: 'clickhouse',
    params: { host: 'ch.internal', port: 9000, user: 'analyst', db: 'events' },
    secret_ref: 'conn-con_click',
    first_command: null,
    section_id: null,
    created_by: 'usr_root',
    created_at: ago(60 * 24 * 9),
  },
  {
    id: 'con_mongo',
    workspace_id: 'wsp_otto',
    name: 'docs mongo',
    kind: 'mongodb',
    params: { connection_string: 'mongodb://mongo.internal:27017/docs' },
    secret_ref: 'conn-con_mongo',
    first_command: null,
    section_id: null,
    created_by: 'usr_dana',
    created_at: ago(60 * 24 * 2),
  },
];

const gitAccounts: GitAccount[] = [
  {
    id: 'gac_gh',
    user_id: 'usr_root',
    provider: 'github',
    label: 'github personal',
    username: 'dev-otto',
    token_ref: 'git-gac_gh',
    api_base_url: null,
    namespace: 'dev-otto',
    token_expires_at: null,
    created_at: ago(60 * 24 * 15),
  },
  {
    id: 'gac_bb',
    user_id: 'usr_root',
    provider: 'bitbucket',
    label: 'work bitbucket',
    username: 'dev@work.io',
    token_ref: 'git-gac_bb',
    api_base_url: null,
    namespace: 'your-org',
    token_expires_at: null,
    created_at: ago(60 * 24 * 10),
  },
];

const repos: Repo[] = [
  {
    id: 'rep_otto',
    workspace_id: 'wsp_otto',
    name: 'otto',
    path: '/Users/dev/otto',
    remote_url: 'git@github.com:dev-otto/otto.git',
    provider: 'github',
    git_account_id: 'gac_gh',
    created_at: ago(60 * 24 * 20),
  },
  {
    id: 'rep_wallet',
    workspace_id: 'wsp_casino',
    name: 'go-wallet',
    path: '/Users/dev/casino/go-wallet',
    remote_url: 'https://bitbucket.org/work/go-wallet.git',
    provider: 'bitbucket',
    git_account_id: 'gac_bb',
    created_at: ago(60 * 24 * 5),
  },
];

const repoStatus: Record<Id, RepoStatusResp> = {
  rep_otto: {
    branch: 'feat/diff-viewer',
    upstream: 'origin/feat/diff-viewer',
    ahead: 2,
    behind: 0,
    changes: [
      { path: 'ui/src/modules/git/DiffViewer.svelte', orig_path: null, kind: 'modified', staged: true, unstaged: false },
      { path: 'ui/src/modules/git/ChangesView.svelte', orig_path: null, kind: 'modified', staged: false, unstaged: true },
      { path: 'crates/otto-git/src/parse.rs', orig_path: null, kind: 'modified', staged: false, unstaged: true },
      { path: 'docs/notes.md', orig_path: null, kind: 'untracked', staged: false, unstaged: true },
      { path: 'ui/src/lib/diff.ts', orig_path: 'ui/src/lib/diffs.ts', kind: 'renamed', staged: true, unstaged: false },
    ],
  },
  rep_wallet: {
    branch: 'master',
    upstream: 'origin/master',
    ahead: 0,
    behind: 3,
    changes: [
      { path: 'internal/dao/balance.go', orig_path: null, kind: 'modified', staged: false, unstaged: true },
    ],
  },
};

const branches: Record<Id, BranchInfo[]> = {
  rep_otto: [
    { name: 'feat/diff-viewer', is_current: true, upstream: 'origin/feat/diff-viewer' },
    { name: 'main', is_current: false, upstream: 'origin/main' },
    { name: 'fix/palette-focus', is_current: false, upstream: null },
  ],
  rep_wallet: [
    { name: 'master', is_current: true, upstream: 'origin/master' },
    { name: 'feat/bonus-split', is_current: false, upstream: 'origin/feat/bonus-split' },
  ],
};

function mkLog(repo: string): CommitInfo[] {
  const subjects = [
    'diff viewer: virtualize file sections',
    'palette: plain-english mode toggles',
    'shell: right panel collapse animation',
    'git: porcelain v2 rename parsing',
    'connections: clickhouse argv warning',
    'sessions: resume on daemon restart',
    'rbac: sliding token expiry',
    'pty: ring buffer line accounting',
    'initial commit',
  ];
  return subjects.map((s, i) => {
    const sha = `${repo}${i}a1b2c3d4e5f60718293a4b5c6d7e8f901234567`.slice(0, 40);
    const prevSha = i + 1 < subjects.length
      ? `${repo}${i + 1}a1b2c3d4e5f60718293a4b5c6d7e8f901234567`.slice(0, 40)
      : null;
    return {
      sha,
      short_sha: `${repo.slice(0, 3)}${i}a1b2`.slice(0, 8),
      author: i % 3 === 0 ? 'Dana K.' : 'Root',
      date: ago(60 * (i + 1) * 7),
      subject: s,
      parents: prevSha ? [prevSha] : [],
      refs: i === 0 ? ['main', 'origin/main'] : i === 3 ? ['develop'] : [],
    };
  });
}
const logs: Record<Id, CommitInfo[]> = { rep_otto: mkLog('aotto'), rep_wallet: mkLog('bwall') };

function fdiff(path: string, oldPath: string | null, startOld: number, startNew: number, body: [('context' | 'add' | 'del'), string][]): FileDiff {
  let o = startOld;
  let n = startNew;
  const lines = body.map(([origin, content]) => {
    const line = {
      origin,
      content,
      old_line: origin === 'add' ? null : o,
      new_line: origin === 'del' ? null : n,
    };
    if (origin !== 'add') o++;
    if (origin !== 'del') n++;
    return line;
  });
  const adds = body.filter(([k]) => k === 'add').length;
  const dels = body.filter(([k]) => k === 'del').length;
  return {
    path,
    old_path: oldPath,
    is_binary: false,
    hunks: [
      {
        header: `@@ -${startOld},${body.length - adds} +${startNew},${body.length - dels} @@`,
        lines,
      },
    ],
  };
}

const sampleDiff: DiffResp = {
  files: [
    fdiff('ui/src/modules/git/DiffViewer.svelte', null, 40, 40, [
      ['context', '  let mode = $state<\'unified\' | \'split\'>(\'unified\');'],
      ['context', ''],
      ['del', '  function renderHunk(h: Hunk) {'],
      ['del', '    return h.lines.map((l) => l.content).join(\'\\n\');'],
      ['add', '  function visibleHunks(file: FileDiff): Hunk[] {'],
      ['add', '    if (collapsed.has(file.path)) return [];'],
      ['add', '    return file.hunks;'],
      ['context', '  }'],
      ['context', ''],
      ['add', '  const totalChanged = $derived(countChanges(diff));'],
    ]),
    fdiff('crates/otto-git/src/parse.rs', null, 102, 102, [
      ['context', '    fn parse_rename(line: &str) -> Option<(String, String)> {'],
      ['del', '        let parts: Vec<&str> = line.split(\' \').collect();'],
      ['add', '        let parts: Vec<&str> = line.split(\'\\t\').collect();'],
      ['add', '        // porcelain v2 uses tab separators for rename entries'],
      ['context', '        if parts.len() < 2 {'],
      ['context', '            return None;'],
      ['context', '        }'],
    ]),
    fdiff('ui/src/lib/diff.ts', 'ui/src/lib/diffs.ts', 1, 1, [
      ['context', 'export interface DiffStats {'],
      ['add', '  files: number;'],
      ['context', '  additions: number;'],
      ['context', '  deletions: number;'],
      ['context', '}'],
    ]),
  ],
};

interface MockPr {
  repo_id: Id;
  summary: PrSummary;
  description_md: string;
  comments: PrComment[];
  approved_by: string[];
  mergeable: boolean | null;
}

const prs: MockPr[] = [
  {
    repo_id: 'rep_otto',
    summary: {
      number: 42,
      title: 'Diff viewer: virtualized rendering + side-by-side mode',
      author: 'dana',
      state: 'open',
      source_branch: 'feat/diff-viewer',
      target_branch: 'main',
      updated_at: ago(45),
      url: 'https://github.com/dev-otto/otto/pull/42',
    },
    description_md:
      '## What\n\nRewrites the diff viewer to only render expanded files and adds a **side-by-side** mode.\n\n## Why\n\nLarge PRs (>5k lines) froze the old renderer.\n\n- virtualized file sections\n- `highlight.js` per-line\n- collapse files over 400 changed lines',
    comments: [
      {
        id: 'cmt_1',
        author: 'root',
        body: 'Nice. Can we keep the hunk headers sticky while scrolling?',
        path: null,
        line: null,
        created_at: ago(200),
        replies: [
          {
            id: 'cmt_2',
            author: 'dana',
            body: 'Done in the latest push.',
            path: null,
            line: null,
            created_at: ago(150),
            replies: [],
          },
        ],
      },
      {
        id: 'cmt_3',
        author: 'root',
        body: 'This split should account for collapsed files too.',
        path: 'ui/src/modules/git/DiffViewer.svelte',
        line: 45,
        created_at: ago(120),
        replies: [],
      },
    ],
    approved_by: [],
    mergeable: true,
  },
  {
    repo_id: 'rep_otto',
    summary: {
      number: 38,
      title: 'Palette: plain-English orchestrator mode',
      author: 'root',
      state: 'merged',
      source_branch: 'feat/palette-english',
      target_branch: 'main',
      updated_at: ago(60 * 24 * 2),
      url: 'https://github.com/dev-otto/otto/pull/38',
    },
    description_md: 'Adds ⇥-toggled plain-English mode with optimize/AI-fallback pills.',
    comments: [],
    approved_by: ['dana'],
    mergeable: null,
  },
  {
    repo_id: 'rep_wallet',
    summary: {
      number: 7,
      title: 'Split bonus balance from cash balance',
      author: 'dana',
      state: 'declined',
      source_branch: 'feat/bonus-split',
      target_branch: 'master',
      updated_at: ago(60 * 24 * 6),
      url: 'https://bitbucket.org/work/go-wallet/pull-requests/7',
    },
    description_md: 'Superseded by the unified wallet redesign.',
    comments: [],
    approved_by: [],
    mergeable: null,
  },
];

let settings: Record<string, unknown> = {
  network_listener: { enabled: false, port: 7700 },
  default_provider: 'claude',
};

const meta: MetaResp = {
  version: '1.0.0-mock',
  api_version: 1,
  needs_onboarding: false,
  network_listener: false,
  tools: [
    { name: 'claude', found: true, version: '2.1.4' },
    { name: 'codex', found: true, version: '0.48.0' },
    { name: 'git', found: true, version: '2.49.0' },
    { name: 'ssh', found: true, version: 'OpenSSH 9.8' },
    { name: 'mysql', found: false, version: null },
    { name: 'redis-cli', found: true, version: '7.4.1' },
  ],
  providers: ['claude', 'codex', 'shell'],
  default_provider: 'claude',
};

// ---------------------------------------------------------------------------
// HTTP routing
// ---------------------------------------------------------------------------

type Handler = (m: RegExpMatchArray, body: any, query: URLSearchParams) => { status?: number; json?: unknown } | undefined;

interface Route {
  method: string;
  re: RegExp;
  handle: Handler;
}

function problem(status: number, code: string, message: string) {
  return { status, json: { code, message } };
}

function newSessionFor(workspaceId: Id, req: any): Session {
  const provider = req.kind === 'connection'
    ? connections.find((c) => c.id === req.connection_id)?.kind ?? 'custom'
    : (req.provider ?? 'shell');
  const title =
    req.title ??
    (req.kind === 'connection'
      ? connections.find((c) => c.id === req.connection_id)?.name ?? 'connection'
      : `${provider} #${sessions.filter((s) => s.provider === provider).length + 1}`);
  const s: Session = {
    id: nid('ses'),
    workspace_id: workspaceId,
    kind: req.kind ?? 'agent',
    provider,
    title,
    status: 'running',
    cwd: req.cwd ?? workspaces.find((w) => w.id === workspaceId)?.root_path ?? '~',
    provider_session_id: provider === 'claude' ? nid('csid') : null,
    connection_id: req.connection_id ?? null,
    created_by: 'usr_root',
    created_at: new Date().toISOString(),
    last_active_at: new Date().toISOString(),
    archived: false,
    meta: req.meta ?? {},
  };
  sessions.push(s);
  return s;
}

const routes: Route[] = [
  { method: 'GET', re: /^\/health$/, handle: () => ({ json: { ok: true } }) },
  { method: 'GET', re: /^\/meta$/, handle: () => ({ json: { ...meta, needs_onboarding: localStorage.getItem('otto_mock_onboarding') === '1' } }) },
  {
    method: 'POST',
    re: /^\/onboarding\/root$/,
    handle: (_m, body) => {
      localStorage.removeItem('otto_mock_onboarding');
      return { json: { token: 'mock-token', user: { ...users[0], display_name: body?.display_name || 'Root' } } };
    },
  },
  {
    method: 'POST',
    re: /^\/auth\/login$/,
    handle: (_m, body) => {
      const u = users.find((x) => x.username === body?.username && !x.disabled);
      if (!u || !body?.password) return problem(401, 'unauthorized', 'bad credentials');
      return { json: { token: 'mock-token', user: u } };
    },
  },
  { method: 'POST', re: /^\/auth\/logout$/, handle: () => ({ status: 204 }) },
  { method: 'GET', re: /^\/auth\/me$/, handle: () => ({ json: users[0] }) },

  { method: 'GET', re: /^\/users$/, handle: () => ({ json: users }) },
  {
    method: 'POST',
    re: /^\/users$/,
    handle: (_m, body) => {
      if (users.some((u) => u.username === body.username)) return problem(409, 'conflict', 'username taken');
      const u: User = {
        id: nid('usr'),
        username: body.username,
        display_name: body.display_name || body.username,
        is_root: false,
        disabled: false,
        created_at: new Date().toISOString(),
      };
      users.push(u);
      return { json: u };
    },
  },
  {
    method: 'PATCH',
    re: /^\/users\/([^/]+)$/,
    handle: (m, body) => {
      const u = users.find((x) => x.id === m[1]);
      if (!u) return problem(404, 'not_found', 'user');
      if (body.display_name != null) u.display_name = body.display_name;
      if (body.disabled != null) u.disabled = body.disabled;
      return { json: u };
    },
  },
  {
    method: 'DELETE',
    re: /^\/users\/([^/]+)$/,
    handle: (m) => {
      const u = users.find((x) => x.id === m[1]);
      if (!u) return problem(404, 'not_found', 'user');
      if (u.is_root) return problem(400, 'invalid', 'cannot disable root');
      u.disabled = true;
      return { status: 204 };
    },
  },

  {
    method: 'GET',
    re: /^\/workspaces$/,
    handle: () => ({
      json: workspaces.filter((w) => !w.archived).map((w): WorkspaceWithRole => ({ ...w, my_role: 'admin' })),
    }),
  },
  {
    method: 'POST',
    re: /^\/workspaces$/,
    handle: (_m, body) => {
      const w: Workspace = {
        id: nid('wsp'),
        name: body.name,
        root_path: body.root_path,
        settings: {},
        archived: false,
        created_at: new Date().toISOString(),
      };
      workspaces.push(w);
      members[w.id] = [{ user_id: 'usr_root', username: 'root', display_name: 'Root', role: 'admin' }];
      return { json: w };
    },
  },
  {
    method: 'PATCH',
    re: /^\/workspaces\/([^/]+)$/,
    handle: (m, body) => {
      const w = workspaces.find((x) => x.id === m[1]);
      if (!w) return problem(404, 'not_found', 'workspace');
      if (body.name != null) w.name = body.name;
      if (body.root_path != null) w.root_path = body.root_path;
      if (body.settings != null) w.settings = body.settings;
      if (body.archived != null) w.archived = body.archived;
      return { json: w };
    },
  },
  {
    method: 'DELETE',
    re: /^\/workspaces\/([^/]+)$/,
    handle: (m) => {
      const w = workspaces.find((x) => x.id === m[1]);
      if (w) w.archived = true;
      return { status: 204 };
    },
  },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/members$/, handle: (m) => ({ json: members[m[1]] ?? [] }) },
  {
    method: 'PUT',
    re: /^\/workspaces\/([^/]+)\/members$/,
    handle: (m, body) => {
      const entries: MemberEntry[] = (body.members ?? []).map((e: { user_id: Id; role: MemberEntry['role'] }) => {
        const u = users.find((x) => x.id === e.user_id);
        return {
          user_id: e.user_id,
          username: u?.username ?? '?',
          display_name: u?.display_name ?? '?',
          role: e.role,
        };
      });
      members[m[1]] = entries;
      return { json: entries };
    },
  },

  {
    method: 'GET',
    re: /^\/workspaces\/([^/]+)\/sessions$/,
    handle: (m) => ({ json: sessions.filter((s) => s.workspace_id === m[1]) }),
  },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/sessions$/,
    handle: (m, body) => ({ json: newSessionFor(m[1], body) }),
  },
  {
    method: 'GET',
    re: /^\/sessions\/([^/]+)$/,
    handle: (m) => {
      const s = sessions.find((x) => x.id === m[1]);
      return s ? { json: s } : problem(404, 'not_found', 'session');
    },
  },
  {
    method: 'PATCH',
    re: /^\/sessions\/([^/]+)$/,
    handle: (m, body) => {
      const s = sessions.find((x) => x.id === m[1]);
      if (!s) return problem(404, 'not_found', 'session');
      if (body.title != null) s.title = body.title;
      return { json: s };
    },
  },
  {
    method: 'DELETE',
    re: /^\/sessions\/([^/]+)$/,
    handle: (m) => {
      const i = sessions.findIndex((x) => x.id === m[1]);
      if (i >= 0) sessions.splice(i, 1);
      return { status: 204 };
    },
  },
  {
    method: 'POST',
    re: /^\/sessions\/([^/]+)\/restart$/,
    handle: (m) => {
      const s = sessions.find((x) => x.id === m[1]);
      if (!s) return problem(404, 'not_found', 'session');
      s.status = 'running';
      s.last_active_at = new Date().toISOString();
      return { json: s };
    },
  },
  {
    method: 'POST',
    re: /^\/sessions\/([^/]+)\/archive$/,
    handle: (m) => {
      const s = sessions.find((x) => x.id === m[1]);
      if (!s) return problem(404, 'not_found', 'session');
      s.archived = true;
      s.status = 'exited';
      return { json: s };
    },
  },
  {
    method: 'POST',
    re: /^\/sessions\/([^/]+)\/unarchive$/,
    handle: (m) => {
      const s = sessions.find((x) => x.id === m[1]);
      if (!s) return problem(404, 'not_found', 'session');
      s.archived = false;
      s.status = 'reconnectable';
      return { json: s };
    },
  },

  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/orchestrate$/,
    handle: (_m, body) => {
      const text: string = body?.text ?? '';
      const n = text.match(/(\d+)/);
      const count = Math.min(Number(n?.[1] ?? 2), 4);
      const plan =
        /spawn|start|open .*agent|claude|codex/i.test(text)
          ? [{ action: 'spawn_sessions', provider: /codex/i.test(text) ? 'codex' : 'claude', count }]
          : /connect/i.test(text)
            ? [{ action: 'open_connection', connection_id: connections[0].id }]
            : [{ action: 'broadcast', text }];
      return {
        json: {
          plan,
          optimized_text: body?.optimize ? `Refined: ${text.trim()} (be specific, verify with tests)` : null,
        },
      };
    },
  },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/orchestrate\/execute$/,
    handle: (m, body) => {
      const results = (body?.plan ?? []).map((a: { action: string }, i: number) => {
        const ids: Id[] = [];
        if (a.action === 'spawn_sessions') {
          const sa = a as unknown as { provider: string; count: number };
          for (let k = 0; k < sa.count; k++) ids.push(newSessionFor(m[1], { kind: 'agent', provider: sa.provider }).id);
        }
        return { action_index: i, ok: true, detail: `${a.action} done`, session_ids: ids };
      });
      return { json: { results } };
    },
  },

  {
    method: 'GET',
    re: /^\/workspaces\/([^/]+)\/connections$/,
    handle: (m) => ({ json: connections.filter((c) => c.workspace_id === m[1] || c.workspace_id === null) }),
  },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/connections$/,
    handle: (m, body) => {
      const c: Connection = {
        id: nid('con'),
        workspace_id: m[1],
        name: body.name,
        kind: body.kind,
        params: body.params ?? {},
        secret_ref: body.secret ? `conn-${nid('x')}` : null,
        first_command: body.first_command ?? null,
        section_id: body.section_id ?? null,
        created_by: 'usr_root',
        created_at: new Date().toISOString(),
      };
      connections.push(c);
      return { json: c };
    },
  },
  {
    method: 'PATCH',
    re: /^\/connections\/([^/]+)$/,
    handle: (m, body) => {
      const c = connections.find((x) => x.id === m[1]);
      if (!c) return problem(404, 'not_found', 'connection');
      if (body.name != null) c.name = body.name;
      if (body.kind != null) c.kind = body.kind;
      if (body.params != null) c.params = body.params;
      if (body.first_command !== undefined) c.first_command = body.first_command;
      if (body.secret) c.secret_ref = c.secret_ref ?? `conn-${c.id}`;
      return { json: c };
    },
  },
  {
    method: 'DELETE',
    re: /^\/connections\/([^/]+)$/,
    handle: (m) => {
      const i = connections.findIndex((x) => x.id === m[1]);
      if (i >= 0) connections.splice(i, 1);
      return { status: 204 };
    },
  },
  {
    method: 'POST',
    re: /^\/connections\/([^/]+)\/open$/,
    handle: (m, body) => {
      const c = connections.find((x) => x.id === m[1]);
      if (!c) return problem(404, 'not_found', 'connection');
      const wsId = c.workspace_id ?? workspaces[0].id;
      return { json: newSessionFor(wsId, { kind: 'connection', connection_id: c.id, title: body?.title ?? null }) };
    },
  },
  {
    method: 'POST',
    re: /^\/connections\/([^/]+)\/test$/,
    handle: (m) => {
      const c = connections.find((x) => x.id === m[1]);
      if (!c) return problem(404, 'not_found', 'connection');
      const ok = c.kind !== 'mongodb'; // one fixture failure for UI states
      return {
        json: {
          ok,
          latency_ms: ok ? 12 + Math.floor(Math.random() * 80) : null,
          message: ok ? 'probe ok' : 'connection refused: mongo.internal:27017',
          warn_argv: c.kind === 'clickhouse',
        },
      };
    },
  },

  { method: 'GET', re: /^\/git\/accounts$/, handle: () => ({ json: gitAccounts }) },
  {
    method: 'POST',
    re: /^\/git\/accounts$/,
    handle: (_m, body) => {
      const a: GitAccount = {
        id: nid('gac'),
        user_id: 'usr_root',
        provider: body.provider,
        label: body.label,
        username: body.username,
        token_ref: `git-${nid('t')}`,
        api_base_url: body.api_base_url ?? null,
        namespace: body.namespace ?? null,
        token_expires_at: body.token_expires_at ?? null,
        created_at: new Date().toISOString(),
      };
      gitAccounts.push(a);
      return { json: a };
    },
  },
  {
    method: 'DELETE',
    re: /^\/git\/accounts\/([^/]+)$/,
    handle: (m) => {
      const i = gitAccounts.findIndex((x) => x.id === m[1]);
      if (i >= 0) gitAccounts.splice(i, 1);
      return { status: 204 };
    },
  },

  {
    method: 'GET',
    re: /^\/workspaces\/([^/]+)\/repos$/,
    handle: (m) => ({ json: repos.filter((r) => r.workspace_id === m[1]) }),
  },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/repos$/,
    handle: (m, body) => {
      const name = body.name ?? (body.clone_url ?? body.path ?? 'repo').split('/').pop()?.replace(/\.git$/, '') ?? 'repo';
      const r: Repo = {
        id: nid('rep'),
        workspace_id: m[1],
        name,
        path: body.path ?? `${workspaces.find((w) => w.id === m[1])?.root_path ?? '~'}/${name}`,
        remote_url: body.clone_url ?? null,
        provider: body.clone_url?.includes('github') ? 'github' : body.clone_url?.includes('gitlab') ? 'gitlab' : body.clone_url?.includes('bitbucket') ? 'bitbucket' : null,
        git_account_id: body.git_account_id ?? null,
        created_at: new Date().toISOString(),
      };
      repos.push(r);
      repoStatus[r.id] = { branch: 'main', upstream: null, ahead: 0, behind: 0, changes: [] };
      branches[r.id] = [{ name: 'main', is_current: true, upstream: null }];
      logs[r.id] = mkLog(r.id);
      return { json: r };
    },
  },
  {
    method: 'DELETE',
    re: /^\/repos\/([^/]+)$/,
    handle: (m) => {
      const i = repos.findIndex((x) => x.id === m[1]);
      if (i >= 0) repos.splice(i, 1);
      return { status: 204 };
    },
  },
  { method: 'GET', re: /^\/repos\/([^/]+)\/status$/, handle: (m) => ({ json: repoStatus[m[1]] ?? problemStatus() }) },
  { method: 'GET', re: /^\/repos\/([^/]+)\/branches$/, handle: (m) => ({ json: branches[m[1]] ?? [] }) },
  {
    method: 'GET',
    re: /^\/repos\/([^/]+)\/log$/,
    handle: (m, _b, q) => {
      const limit = Number(q.get('limit') ?? 50);
      const skip = Number(q.get('skip') ?? 0);
      return { json: (logs[m[1]] ?? []).slice(skip, skip + limit) };
    },
  },
  { method: 'GET', re: /^\/repos\/([^/]+)\/diff$/, handle: () => ({ json: sampleDiff }) },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/(stage|unstage)$/,
    handle: (m, body) => {
      const st = repoStatus[m[1]];
      if (!st) return problem(404, 'not_found', 'repo');
      const staging = m[2] === 'stage';
      for (const ch of st.changes) {
        if ((body.paths as string[]).includes(ch.path)) {
          ch.staged = staging;
          ch.unstaged = !staging;
        }
      }
      return { json: st };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/commit$/,
    handle: (m, body) => {
      const st = repoStatus[m[1]];
      if (st) {
        st.changes = st.changes.filter((c) => !c.staged);
        st.ahead += 1;
        logs[m[1]] = [
          {
            sha: nid('sha').padEnd(40, '0'),
            short_sha: nid('s').slice(0, 8),
            author: 'Root',
            date: new Date().toISOString(),
            subject: (body.message as string).split('\n')[0],
            parents: logs[m[1]]?.[0] ? [logs[m[1]][0].sha] : [],
            refs: [],
          },
          ...(logs[m[1]] ?? []),
        ];
      }
      return { json: { sha: 'deadbeef'.padEnd(40, '0') } };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/push$/,
    handle: (m) => {
      const st = repoStatus[m[1]];
      if (st) st.ahead = 0;
      return { json: { output: 'To origin\n   a1b2c3d..e4f5a6b  branch -> branch' } };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/pull$/,
    handle: (m) => {
      const st = repoStatus[m[1]];
      if (st) st.behind = 0;
      return { json: { output: 'Already up to date.' } };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/checkout$/,
    handle: (m, body) => {
      const st = repoStatus[m[1]];
      const br = branches[m[1]] ?? [];
      if (!st) return problem(404, 'not_found', 'repo');
      if (body.create && !br.some((b) => b.name === body.branch)) {
        br.push({ name: body.branch, is_current: false, upstream: null });
      }
      for (const b of br) b.is_current = b.name === body.branch;
      st.branch = body.branch;
      st.upstream = br.find((b) => b.is_current)?.upstream ?? null;
      return { json: st };
    },
  },
  { method: 'POST', re: /^\/repos\/([^/]+)\/stash$/, handle: (m) => ({ json: repoStatus[m[1]] }) },

  {
    method: 'GET',
    re: /^\/repos\/([^/]+)\/prs$/,
    handle: (m, _b, q) => {
      const state = q.get('state') ?? 'open';
      let list = prs.filter((p) => p.repo_id === m[1]);
      if (state !== 'all') list = list.filter((p) => p.summary.state === state);
      return { json: list.map((p) => p.summary) };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/prs$/,
    handle: (m, body) => {
      const num = Math.max(0, ...prs.map((p) => p.summary.number)) + 1;
      const pr: MockPr = {
        repo_id: m[1],
        summary: {
          number: num,
          title: body.title,
          author: 'root',
          state: 'open',
          source_branch: body.source_branch,
          target_branch: body.target_branch,
          updated_at: new Date().toISOString(),
          url: `https://github.com/dev-otto/otto/pull/${num}`,
        },
        description_md: body.description ?? '',
        comments: [],
        approved_by: [],
        mergeable: true,
      };
      prs.push(pr);
      return { json: pr.summary };
    },
  },
  {
    method: 'GET',
    re: /^\/repos\/([^/]+)\/prs\/(\d+)$/,
    handle: (m) => {
      const pr = prs.find((p) => p.repo_id === m[1] && p.summary.number === Number(m[2]));
      if (!pr) return problem(404, 'not_found', 'pr');
      const detail: PrDetail = {
        ...pr.summary,
        description_md: pr.description_md,
        comments: pr.comments,
        approved_by: pr.approved_by,
        reviewers: pr.approved_by.map((name) => ({ name, approved: true, avatar_url: null, reviewed_at: null })),
        mergeable: pr.mergeable,
      };
      return { json: detail };
    },
  },
  { method: 'GET', re: /^\/repos\/([^/]+)\/prs\/(\d+)\/diff$/, handle: () => ({ json: sampleDiff }) },
  {
    method: 'PATCH',
    re: /^\/repos\/([^/]+)\/prs\/(\d+)$/,
    handle: (m, body) => {
      const pr = prs.find((p) => p.repo_id === m[1] && p.summary.number === Number(m[2]));
      if (!pr) return problem(404, 'not_found', 'pr');
      if (body.title != null) pr.summary.title = body.title;
      if (body.description != null) pr.description_md = body.description;
      pr.summary.updated_at = new Date().toISOString();
      return { status: 204 };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/prs\/(\d+)\/comments$/,
    handle: (m, body) => {
      const pr = prs.find((p) => p.repo_id === m[1] && p.summary.number === Number(m[2]));
      if (!pr) return problem(404, 'not_found', 'pr');
      const c: PrComment = {
        id: nid('cmt'),
        author: 'root',
        body: body.body,
        path: body.path ?? null,
        line: body.line ?? null,
        created_at: new Date().toISOString(),
        replies: [],
      };
      if (body.in_reply_to) {
        const find = (list: PrComment[]): PrComment | undefined => {
          for (const x of list) {
            if (x.id === body.in_reply_to) return x;
            const r = find(x.replies);
            if (r) return r;
          }
          return undefined;
        };
        const parent = find(pr.comments);
        if (parent) parent.replies.push(c);
        else pr.comments.push(c);
      } else {
        pr.comments.push(c);
      }
      return { json: c };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/prs\/(\d+)\/approve$/,
    handle: (m) => {
      const pr = prs.find((p) => p.repo_id === m[1] && p.summary.number === Number(m[2]));
      if (pr && !pr.approved_by.includes('root')) pr.approved_by.push('root');
      return { status: 204 };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/prs\/(\d+)\/merge$/,
    handle: (m) => {
      const pr = prs.find((p) => p.repo_id === m[1] && p.summary.number === Number(m[2]));
      if (pr) pr.summary.state = 'merged';
      return { status: 204 };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/prs\/(\d+)\/decline$/,
    handle: (m) => {
      const pr = prs.find((p) => p.repo_id === m[1] && p.summary.number === Number(m[2]));
      if (pr) pr.summary.state = 'declined';
      return { status: 204 };
    },
  },

  { method: 'GET', re: /^\/settings$/, handle: () => ({ json: settings }) },
  {
    method: 'PUT',
    re: /^\/settings$/,
    handle: (_m, body) => {
      settings = { ...settings, ...body };
      return { json: settings };
    },
  },
];

function problemStatus(): RepoStatusResp {
  return { branch: 'main', upstream: null, ahead: 0, behind: 0, changes: [] };
}

// ---------------------------------------------------------------------------
// fetch + WebSocket interception
// ---------------------------------------------------------------------------

const LATENCY = 90;

function handleApi(method: string, pathWithQuery: string, body: unknown): { status: number; body: string | null } {
  const [path, qs] = pathWithQuery.split('?');
  const query = new URLSearchParams(qs ?? '');
  for (const r of routes) {
    if (r.method !== method) continue;
    const m = path.match(r.re);
    if (!m) continue;
    const out = r.handle(m, body ?? {}, query) ?? { status: 204 };
    if (out.status === 204 && out.json === undefined) return { status: 204, body: null };
    return { status: out.status ?? 200, body: JSON.stringify(out.json) };
  }
  return { status: 404, body: JSON.stringify({ code: 'not_found', message: `${method} ${path}` }) };
}

class MockTermSocket {
  // WebSocket-ish surface used by Terminal.svelte
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  binaryType = 'arraybuffer';
  readyState = 0;

  private sessionId: string;
  private closed = false;
  private buf = '';
  private timers: ReturnType<typeof setTimeout>[] = [];

  constructor(url: string) {
    this.sessionId = url.split('/ws/term/')[1]?.split('?')[0] ?? '?';
    this.later(40, () => {
      this.readyState = 1;
      this.onopen?.(new Event('open'));
      const s = sessions.find((x) => x.id === this.sessionId);
      this.json({ type: 'status', status: s?.status ?? 'running' });
      this.bytes(
        `\x1b[1;36mOtto mock terminal\x1b[0m — session \x1b[33m${s?.title ?? this.sessionId}\x1b[0m (${s?.provider ?? '?'})\r\n` +
          `Daemon offline; this PTY echoes your input.\r\n\r\n\x1b[32m➜\x1b[0m `,
      );
    });
  }

  private later(ms: number, fn: () => void): void {
    this.timers.push(setTimeout(() => !this.closed && fn(), ms));
  }

  private json(obj: unknown): void {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(obj) }));
  }

  private bytes(text: string): void {
    const b = textToBytes(text);
    const ab = new ArrayBuffer(b.length);
    new Uint8Array(ab).set(b);
    this.onmessage?.(new MessageEvent('message', { data: ab }));
  }

  send(raw: string): void {
    if (this.closed) return;
    try {
      const msg = JSON.parse(raw);
      if (msg.type === 'input') {
        const text = base64ToText(msg.data);
        for (const ch of text) {
          if (ch === '\r') {
            const line = this.buf;
            this.buf = '';
            this.bytes('\r\n');
            if (line.trim() === 'clear') this.bytes('\x1b[2J\x1b[H');
            else if (line.trim().length > 0) this.bytes(`mock: ${line.trim()}\r\n`);
            this.bytes('\x1b[32m➜\x1b[0m ');
          } else if (ch === '\x7f') {
            if (this.buf.length > 0) {
              this.buf = this.buf.slice(0, -1);
              this.bytes('\b \b');
            }
          } else {
            this.buf += ch;
            this.bytes(ch);
          }
        }
      } else if (msg.type === 'scrollback') {
        this.json({ type: 'scrollback', data: '' });
      }
      // resize: ignored
    } catch {
      /* non-JSON input ignored */
    }
  }

  close(): void {
    this.closed = true;
    this.readyState = 3;
    for (const t of this.timers) clearTimeout(t);
    this.onclose?.(new CloseEvent('close'));
  }
}

class MockEventsSocket {
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  binaryType = 'blob';
  readyState = 0;

  private closed = false;
  private timers: ReturnType<typeof setInterval | typeof setTimeout>[] = [];

  constructor() {
    const t0 = setTimeout(() => {
      if (this.closed) return;
      this.readyState = 1;
      this.onopen?.(new Event('open'));
      const t1 = setTimeout(() => {
        this.emit({
          type: 'notice',
          level: 'info',
          title: 'Mock mode',
          body: 'Otto is running against fixture data — daemon not required.',
        });
      }, 2500);
      let working = true;
      const t2 = setInterval(() => {
        working = !working;
        const s = sessions.find((x) => x.id === 'ses_claude1');
        if (s) {
          s.status = working ? 'working' : 'idle';
          this.emit({
            type: 'session_status',
            session_id: s.id,
            workspace_id: s.workspace_id,
            status: s.status,
          });
        }
      }, 9000);
      this.timers.push(t1, t2);
    }, 60);
    this.timers.push(t0);
  }

  private emit(obj: unknown): void {
    if (this.closed) return;
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(obj) }));
  }

  send(): void {
    /* events socket is server→client only */
  }

  close(): void {
    this.closed = true;
    this.readyState = 3;
    for (const t of this.timers) clearTimeout(t as ReturnType<typeof setTimeout>);
    this.onclose?.(new CloseEvent('close'));
  }
}

let installed = false;

/** Install fetch + WebSocket interception. Idempotent. */
export function setupMock(): void {
  if (installed) return;
  installed = true;

  const realFetch = window.fetch.bind(window);
  window.fetch = (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const apiIdx = url.indexOf('/api/v1');
    if (apiIdx === -1) return realFetch(input, init);

    const path = url.slice(apiIdx + '/api/v1'.length);
    const method = (init?.method ?? 'GET').toUpperCase();
    let body: unknown;
    if (init?.body && typeof init.body === 'string') {
      try {
        body = JSON.parse(init.body);
      } catch {
        body = undefined;
      }
    }
    await new Promise((r) => setTimeout(r, LATENCY * (0.5 + Math.random())));
    const out = handleApi(method, path, body);
    return new Response(out.body, {
      status: out.status,
      headers: out.body ? { 'Content-Type': 'application/json' } : undefined,
    });
  }) as typeof window.fetch;

  const RealWS = window.WebSocket;
  const Patched = function (this: unknown, url: string | URL, protocols?: string | string[]) {
    const u = String(url);
    if (u.includes('/ws/term/')) return new MockTermSocket(u) as unknown as WebSocket;
    if (u.includes('/ws/events')) return new MockEventsSocket() as unknown as WebSocket;
    return new RealWS(url, protocols);
  } as unknown as typeof WebSocket;
  (Patched as { prototype: WebSocket }).prototype = RealWS.prototype;
  Object.defineProperties(Patched, {
    CONNECTING: { value: 0 },
    OPEN: { value: 1 },
    CLOSING: { value: 2 },
    CLOSED: { value: 3 },
  });
  window.WebSocket = Patched;
}
