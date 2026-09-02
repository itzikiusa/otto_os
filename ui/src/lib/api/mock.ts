// Mock layer — lets the whole SPA run with no daemon. Activated when
// `import.meta.env.VITE_OTTO_MOCK === '1'` or `localStorage.otto_mock === '1'`.
// It monkey-patches window.fetch (for /api/v1/*) and window.WebSocket (for
// /ws/*) so client.ts stays untouched.

import type {
  AwsAccount,
  AwsPermissions,
  AwsStatus,
  AthenaExecution,
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

// In-memory API-client collections/requests (for import + create in mock mode).
let gitCollectionFiles: { name: string; content: string }[] = [];
let apiSeq = 0;
const apiCollections: Record<string, unknown>[] = [];
const apiRequests: Record<string, unknown>[] = [];

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
    environment: 'dev',
    read_only: false,
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
    environment: 'staging',
    read_only: false,
    created_by: 'usr_root',
    created_at: ago(60 * 24 * 3),
  },
  {
    id: 'con_pg',
    workspace_id: 'wsp_casino',
    name: 'staging postgres',
    kind: 'postgres',
    params: { host: 'pg.example.internal', port: 5432, user: 'reader', db: 'shopdb' },
    secret_ref: 'conn-con_pg',
    first_command: null,
    section_id: null,
    environment: 'staging',
    read_only: false,
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
    environment: 'staging',
    read_only: false,
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
    environment: 'prod',
    read_only: false,
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
    environment: 'dev',
    read_only: true,
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
            resolved: false,
          },
        ],
        // Resolved thread — exercises the status chip + reopen affordance.
        resolved: true,
        thread_id: 'cmt_1',
      },
      {
        id: 'cmt_3',
        author: 'root',
        body: 'This split should account for collapsed files too.',
        path: 'ui/src/modules/git/DiffViewer.svelte',
        line: 45,
        created_at: ago(120),
        replies: [],
        resolved: false,
        thread_id: 'cmt_3',
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
  model_flags: { claude: true, codex: true, agy: true, shell: false },
};

// ---------------------------------------------------------------------------
// HTTP routing
// ---------------------------------------------------------------------------

type Handler = (
  m: RegExpMatchArray,
  body: any,
  query: URLSearchParams,
) => { status?: number; json?: unknown; /** Verbatim non-JSON body (text/plain logs). */ raw?: string } | undefined;

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


// ---------------------------------------------------------------------------
// AWS console fixtures (mirrors docs/design/aws-k8s-consoles.md §2)
// ---------------------------------------------------------------------------

const awsPermsOk: AwsPermissions = {
  checked_at: ago(4),
  identity: { account: '123456789012', arn: 'arn:aws:sts::123456789012:assumed-role/Admin/dana', user_id: 'AROA…' },
  services: { s3: 'allowed', sqs: 'allowed', ec2: 'allowed', athena: 'allowed', eks: 'allowed' },
  login_required: false,
};
const awsAccounts: AwsAccount[] = [
  {
    id: 'aws_sandbox',
    name: 'sandbox',
    auth_mode: 'profile',
    profile: 'sandbox',
    region: 'eu-west-1',
    role_arn: null,
    environment: 'dev',
    color: '#3b82f6',
    identity: awsPermsOk.identity,
    permissions: awsPermsOk,
    created_by: 'usr_root',
    created_at: ago(60 * 24 * 3),
    updated_at: ago(60),
    last_used_at: ago(5),
  },
  {
    id: 'aws_prod',
    name: 'prod-eu',
    auth_mode: 'access_keys',
    access_key_id: 'AKIAIOSFODNN7EXAMPLE',
    region: 'eu-central-1',
    role_arn: 'arn:aws:iam::999999999999:role/ReadOnly',
    environment: 'prod',
    color: '#ef4444',
    identity: { account: '999999999999', arn: 'arn:aws:sts::999999999999:assumed-role/ReadOnly/otto', user_id: 'AROA…' },
    permissions: {
      checked_at: ago(12),
      services: { s3: 'allowed', sqs: 'denied', ec2: 'allowed', athena: 'unknown', eks: 'denied' },
      login_required: false,
    },
    created_by: 'usr_root',
    created_at: ago(60 * 24 * 9),
    updated_at: ago(60 * 24),
    last_used_at: ago(30),
  },
];
const awsStatus: AwsStatus = {
  installed: localStorage.getItem('otto_mock_aws_missing') !== '1',
  version: '2.17.3',
  path: '/opt/homebrew/bin/aws',
  install: { tool: 'aws', state: 'idle', log_tail: '' },
};
let athenaPolls = 0;
const athenaHistory: AthenaExecution[] = [
  { id: 'q-1111', query: 'SELECT count(*) FROM logs.requests WHERE dt = date \'2026-09-01\'', state: 'SUCCEEDED', submitted_at: ago(40), completed_at: ago(39), data_scanned_bytes: 512 * 1024 * 1024, execution_ms: 4210 },
  { id: 'q-2222', query: 'SELECT * FROM logs.requests LIMIT 10', state: 'FAILED', submitted_at: ago(90), completed_at: ago(89), data_scanned_bytes: 0, execution_ms: 300 },
];
function awsAccount(id: string): AwsAccount | undefined {
  return awsAccounts.find((a) => a.id === id);
}


// ---------------------------------------------------------------------------
// Kubernetes console (/k8s/*) — two clusters, a handful of pods/deployments
// per namespace, canned describe/events/logs. Enough to drive the workspace
// without a daemon; mutations only toast.
// ---------------------------------------------------------------------------

interface MockK8sCluster {
  id: Id;
  name: string;
  source: 'kubeconfig' | 'imported' | 'eks';
  kubeconfig_path: string | null;
  context_name: string;
  default_namespace: string | null;
  aws_account_id: Id | null;
  environment: 'dev' | 'staging' | 'prod';
  color: string | null;
  capabilities: { server_version: string; metrics_server: boolean; argo_rollouts: boolean; argocd: boolean; checked_at: string } | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
  last_used_at: string | null;
}

const k8sClusters: MockK8sCluster[] = [
  {
    id: 'k8s_dev', name: 'dev-eu-1', source: 'kubeconfig', kubeconfig_path: '~/.kube/config', context_name: 'dev-eu-1',
    default_namespace: 'payments', aws_account_id: null, environment: 'dev', color: '#4f8cff',
    capabilities: { server_version: 'v1.30.2', metrics_server: true, argo_rollouts: true, argocd: true, checked_at: ago(4) },
    created_by: 'usr_root', created_at: ago(60 * 24 * 9), updated_at: ago(60 * 3), last_used_at: ago(2),
  },
  {
    id: 'k8s_prod', name: 'prod-eu-1', source: 'eks', kubeconfig_path: '<data>/kube/k8s_prod.yaml', context_name: 'prod-eu-1',
    default_namespace: null, aws_account_id: null, environment: 'prod', color: '#e5484d',
    capabilities: { server_version: 'v1.29.7-eks', metrics_server: false, argo_rollouts: false, argocd: false, checked_at: ago(30) },
    created_by: 'usr_root', created_at: ago(60 * 24 * 30), updated_at: ago(60 * 24), last_used_at: ago(60 * 5),
  },
];
interface MockInstallJob {
  tool: string;
  state: string;
  log_tail: string;
  started_at: string | null;
  finished_at: string | null;
  error: string | null;
}
const k8sInstall: Record<'kubectl' | 'k9s', MockInstallJob> = {
  kubectl: { tool: 'kubectl', state: 'idle', log_tail: '', started_at: null, finished_at: null, error: null },
  k9s: { tool: 'k9s', state: 'idle', log_tail: '', started_at: null, finished_at: null, error: null },
};
const K8S_NS = ['payments', 'kube-system', 'argocd', 'default'];
function k8sPods(ns: string) {
  const mk = (name: string, status: string, ready: string, restarts: number, health: string, age: number) => ({
    name, namespace: ns, kind: 'Pod', status, ready, restarts, age_seconds: age, node: 'ip-10-0-1-23', ip: `10.0.${ns.length}.${(name.length * 7) % 250}`,
    cpu: (name.length * 13) % 400, mem: (64 + ((name.length * 31) % 900)) * 1024 * 1024, images: [`registry.local/${name.split('-')[0]}:1.${name.length}.0`],
    labels: { app: name.split('-')[0], 'app.kubernetes.io/part-of': ns }, extra: {}, health,
  });
  return [
    mk(`${ns}-api-7d9f8b6c4-x2kqp`, 'Running', '2/2', 0, 'ok', 3600 * 26),
    mk(`${ns}-api-7d9f8b6c4-m8vzt`, 'Running', '2/2', 1, 'ok', 3600 * 26),
    mk(`${ns}-worker-5c6d7e8f9-q1w2e`, 'CrashLoopBackOff', '0/1', 14, 'bad', 3600 * 2),
    mk(`${ns}-migrate-28841-abcde`, 'Succeeded', '0/1', 0, 'ok', 3600 * 50),
    mk(`${ns}-cache-0`, 'Pending', '0/1', 0, 'progressing', 45),
  ];
}
function k8sWorkloads(ns: string, kind: string) {
  const K = kind === 'deployments' ? 'Deployment' : kind === 'statefulsets' ? 'StatefulSet' : 'DaemonSet';
  return [
    { name: `${ns}-api`, namespace: ns, kind: K, status: 'Available', ready: '2/2', age_seconds: 86400 * 12, labels: { app: 'api' }, extra: { desired: '2', updated: '2', available: '2' }, health: 'ok' },
    { name: `${ns}-worker`, namespace: ns, kind: K, status: 'Progressing', ready: '0/1', age_seconds: 86400 * 3, labels: { app: 'worker' }, extra: { desired: '1', updated: '1', available: '0' }, health: 'bad' },
  ];
}
function k8sRows(kind: string, ns: string) {
  const nss = ns ? [ns] : K8S_NS;
  const out: unknown[] = [];
  for (const n of nss) {
    if (kind === 'pods') out.push(...k8sPods(n));
    else if (kind === 'deployments' || kind === 'statefulsets' || kind === 'daemonsets') out.push(...k8sWorkloads(n, kind));
    else if (kind === 'services') out.push({ name: `${n}-api`, namespace: n, kind: 'Service', status: 'Active', age_seconds: 86400 * 12, labels: {}, extra: { type: 'ClusterIP', cluster_ip: '10.96.12.4', external_ip: '', ports: '80/TCP,443/TCP' }, health: 'ok' });
    else if (kind === 'cronjobs') out.push({ name: `${n}-nightly`, namespace: n, kind: 'CronJob', status: 'Scheduled', age_seconds: 86400 * 40, labels: {}, extra: { schedule: '0 2 * * *', suspend: 'false', active: '0', last_schedule: '7h' }, health: 'ok' });
    else if (kind === 'secrets') out.push({ name: `${n}-db-credentials`, namespace: n, kind: 'Secret', status: 'Opaque', age_seconds: 86400 * 90, labels: {}, extra: { type: 'Opaque', keys: 'username,password' }, health: 'ok' });
    else if (kind === 'rollouts') out.push({ name: `${n}-checkout`, namespace: n, kind: 'Rollout', status: 'Paused', ready: '3/5', age_seconds: 86400 * 5, labels: {}, extra: { strategy: 'canary', phase: 'Paused', step: '2/6', weight: '20', paused: 'true', desired: '5', available: '3' }, health: 'progressing' });
    else if (kind === 'applications' && n === 'argocd') out.push(
      { name: 'payments', namespace: n, kind: 'Application', status: 'Synced', age_seconds: 86400 * 120, labels: {}, extra: { sync: 'Synced', health: 'Healthy', revision: 'a1b2c3d4', repo: 'git@github.com:acme/deploy.git', path: 'apps/payments', dest_ns: 'payments' }, health: 'ok' },
      { name: 'ledger', namespace: n, kind: 'Application', status: 'OutOfSync', age_seconds: 86400 * 80, labels: {}, extra: { sync: 'OutOfSync', health: 'Degraded', revision: 'f00dbabe', repo: 'git@github.com:acme/deploy.git', path: 'apps/ledger', dest_ns: 'ledger' }, health: 'bad' },
    );
    else if (kind === 'events') out.push({ name: `${n}-worker.17f1`, namespace: n, kind: 'Event', status: '2m', age_seconds: 120, labels: {}, extra: { type: 'Warning', reason: 'BackOff', object: `pod/${n}-worker-5c6d7e8f9-q1w2e`, message: 'Back-off restarting failed container worker', count: '14' }, health: 'warn' });
  }
  return out;
}
const K8S_LOG_LINES = Array.from({ length: 800 }, (_, i) => `${new Date(NOW - (800 - i) * 900).toISOString()} level=${i % 37 === 0 ? 'error' : 'info'} msg="handled request" path=/v1/charges/${1000 + i} status=${i % 37 === 0 ? 502 : 200} dur=${(i * 7) % 240}ms`);

const k8sRoutes: Route[] = [
  { method: 'GET', re: /^\/k8s\/status$/, handle: () => ({ json: { kubectl: { installed: true, version: 'v1.31.0', path: '/opt/homebrew/bin/kubectl' }, k9s: { installed: false, version: null, path: null }, install: k8sInstall } }) },
  {
    method: 'POST', re: /^\/k8s\/install$/,
    handle: (_m, body) => {
      const tool = body?.tool === 'k9s' ? 'k9s' : 'kubectl';
      k8sInstall[tool] = { ...k8sInstall[tool], state: 'running', log_tail: `==> Downloading ${tool}…\n`, started_at: new Date().toISOString() };
      setTimeout(() => { k8sInstall[tool] = { ...k8sInstall[tool], state: 'done', log_tail: k8sInstall[tool].log_tail + '==> Verified.\n', finished_at: new Date().toISOString() }; }, 4000);
      return { json: k8sInstall[tool] };
    },
  },
  { method: 'GET', re: /^\/k8s\/discover$/, handle: () => ({ json: { contexts: [
    { name: 'dev-eu-1', cluster: 'dev-eu-1.k8s.local', user: 'dev-admin', namespace: 'payments', kubeconfig_path: '~/.kube/config', server: 'https://10.0.0.10:6443' },
    { name: 'staging-eu-1', cluster: 'staging', user: 'staging-admin', namespace: null, kubeconfig_path: '~/.kube/config', server: 'https://staging.k8s.acme.io' },
    { name: 'kind-local', cluster: 'kind-local', user: 'kind-local', namespace: null, kubeconfig_path: '~/.kube/kind.yaml', server: 'https://127.0.0.1:52341' },
  ] } }) },
  { method: 'GET', re: /^\/k8s\/clusters$/, handle: () => ({ json: k8sClusters }) },
  {
    method: 'POST', re: /^\/k8s\/clusters$/,
    handle: (_m, body) => {
      const c: MockK8sCluster = { id: nid('k8s'), name: body.name, source: 'kubeconfig', kubeconfig_path: body.kubeconfig_path ?? null, context_name: body.context_name, default_namespace: body.default_namespace ?? null, aws_account_id: null, environment: body.environment ?? 'dev', color: body.color ?? null, capabilities: null, created_by: 'usr_root', created_at: new Date().toISOString(), updated_at: new Date().toISOString(), last_used_at: null };
      k8sClusters.push(c);
      return { status: 201, json: c };
    },
  },
  {
    method: 'POST', re: /^\/k8s\/clusters\/import$/,
    handle: (_m, body) => {
      if (!String(body.kubeconfig_yaml ?? '').includes('kind: Config')) return problem(400, 'invalid', 'not a kubeconfig (missing `kind: Config`)');
      const c: MockK8sCluster = { id: nid('k8s'), name: body.name, source: 'imported', kubeconfig_path: '<data>/kube/new.yaml', context_name: body.context_name ?? 'current', default_namespace: body.default_namespace ?? null, aws_account_id: null, environment: body.environment ?? 'dev', color: null, capabilities: null, created_by: 'usr_root', created_at: new Date().toISOString(), updated_at: new Date().toISOString(), last_used_at: null };
      k8sClusters.push(c);
      return { status: 201, json: c };
    },
  },
  { method: 'GET', re: /^\/k8s\/clusters\/([^/]+)$/, handle: (m) => { const c = k8sClusters.find((x) => x.id === m[1]); return c ? { json: c } : problem(404, 'not_found', 'cluster'); } },
  { method: 'PATCH', re: /^\/k8s\/clusters\/([^/]+)$/, handle: (m, body) => { const c = k8sClusters.find((x) => x.id === m[1]); if (!c) return problem(404, 'not_found', 'cluster'); Object.assign(c, body, { updated_at: new Date().toISOString() }); return { json: c }; } },
  { method: 'DELETE', re: /^\/k8s\/clusters\/([^/]+)$/, handle: (m) => { const i = k8sClusters.findIndex((x) => x.id === m[1]); if (i >= 0) k8sClusters.splice(i, 1); return { status: 204 }; } },
  { method: 'POST', re: /^\/k8s\/clusters\/([^/]+)\/test$/, handle: (m) => ({ json: { ok: m[1] !== 'k8s_prod', latency_ms: 142, message: m[1] === 'k8s_prod' ? 'login required: the SSO session for profile prod has expired' : 'ok', server_version: 'v1.30.2' } }) },
  { method: 'GET', re: /^\/k8s\/clusters\/([^/]+)\/capabilities$/, handle: (m) => { const c = k8sClusters.find((x) => x.id === m[1]); return c?.capabilities ? { json: c.capabilities } : { json: { server_version: 'v1.30.2', metrics_server: false, argo_rollouts: false, argocd: false, checked_at: new Date().toISOString() } }; } },
  { method: 'GET', re: /^\/k8s\/clusters\/([^/]+)\/namespaces$/, handle: () => ({ json: { namespaces: K8S_NS.map((n) => ({ name: n, status: 'Active', age_seconds: 86400 * 100 })) } }) },
  { method: 'GET', re: /^\/k8s\/clusters\/([^/]+)\/nodes$/, handle: () => ({ json: { nodes: [
    { name: 'ip-10-0-1-23', status: 'Ready', roles: 'control-plane', version: 'v1.30.2', cpu_capacity: 4000, mem_capacity: 16 * 1024 ** 3, cpu_usage: 812, mem_usage: Math.round(6.1 * 1024 ** 3), age_seconds: 86400 * 200 },
    { name: 'ip-10-0-2-41', status: 'Ready', roles: 'worker', version: 'v1.30.2', cpu_capacity: 8000, mem_capacity: 32 * 1024 ** 3, cpu_usage: 3200, mem_usage: 19 * 1024 ** 3, age_seconds: 86400 * 120 },
    { name: 'ip-10-0-3-77', status: 'NotReady', roles: 'worker', version: 'v1.30.1', cpu_capacity: 8000, mem_capacity: 32 * 1024 ** 3, cpu_usage: null, mem_usage: null, age_seconds: 3600 * 5 },
  ] } }) },
  {
    method: 'GET', re: /^\/k8s\/clusters\/([^/]+)\/resources$/,
    handle: (m, _b, q) => {
      if (m[1] === 'k8s_prod') return problem(502, 'invalid', 'login required: the SSO session for profile prod has expired (run `aws sso login`)');
      const kind = q.get('kind') ?? 'pods';
      const ns = q.get('ns') ?? '';
      return { json: { kind, items: k8sRows(kind, ns), has_metrics: kind === 'pods' } };
    },
  },
  {
    method: 'GET', re: /^\/k8s\/clusters\/([^/]+)\/resource$/,
    handle: (_m, _b, q) => {
      const name = q.get('name') ?? '';
      const ns = q.get('ns') ?? '';
      const kind = q.get('kind') ?? 'pods';
      const isSecret = kind === 'secrets';
      return { json: {
        manifest: { apiVersion: 'v1', kind: kind === 'pods' ? 'Pod' : kind, metadata: { name, namespace: ns, labels: { app: name.split('-')[0] }, creationTimestamp: ago(60 * 26) }, ...(isSecret ? { type: 'Opaque', data: { username: '<redacted>', password: '<redacted>' } } : { spec: { containers: [{ name: 'app', image: `registry.local/${name.split('-')[0]}:1.4.0`, ports: [{ containerPort: 8080 }] }, { name: 'sidecar', image: 'envoy:1.30' }] }, status: { phase: 'Running', podIP: '10.0.8.21' } }) },
        describe: `Name:         ${name}\nNamespace:    ${ns}\nNode:         ip-10-0-1-23/10.0.1.23\nStatus:       Running\nIP:           10.0.8.21\nContainers:\n  app:\n    Image:      registry.local/app:1.4.0\n    State:      Running\n    Ready:      True\n    Restart Count: 0\nEvents:\n  Type    Reason   Age   From     Message\n  ----    ------   ----  ----     -------\n  Normal  Pulled   26h   kubelet  Container image already present on machine\n`,
        events: [
          { type: 'Normal', reason: 'Scheduled', message: `Successfully assigned ${ns}/${name} to ip-10-0-1-23`, count: 1, last_seen: '26h' },
          { type: 'Normal', reason: 'Pulled', message: 'Container image already present on machine', count: 1, last_seen: '26h' },
          ...(name.includes('worker') ? [{ type: 'Warning', reason: 'BackOff', message: 'Back-off restarting failed container worker', count: 14, last_seen: '2m' }] : []),
        ],
      } };
    },
  },
  { method: 'GET', re: /^\/k8s\/clusters\/([^/]+)\/pods\/([^/]+)\/([^/]+)\/containers$/, handle: () => ({ json: { containers: [
    { name: 'app', image: 'registry.local/app:1.4.0', ready: true, state: 'running', restarts: 0, init: false },
    { name: 'sidecar', image: 'envoy:1.30', ready: true, state: 'running', restarts: 0, init: false },
    { name: 'init-migrate', image: 'registry.local/app:1.4.0', ready: true, state: 'terminated', restarts: 0, init: true },
  ] } }) },
  {
    method: 'GET', re: /^\/k8s\/clusters\/([^/]+)\/pods\/([^/]+)\/([^/]+)\/logs$/,
    handle: (_m, _b, q) => {
      const tail = Number(q.get('tail') ?? 500);
      const ts = q.get('timestamps') === 'true';
      const lines = K8S_LOG_LINES.slice(-tail).map((l) => (ts ? l : l.replace(/^\S+ /, '')));
      return { raw: lines.join('\n') + '\n' };
    },
  },
  { method: 'GET', re: /^\/k8s\/clusters\/([^/]+)\/metrics$/, handle: (_m, _b, q) => { const ns = q.get('ns') ?? ''; const nss = ns ? [ns] : K8S_NS; return { json: { available: true, pods: nss.flatMap((n) => k8sPods(n).map((p) => ({ name: p.name, namespace: n, cpu_millicores: (p.name.length * 13) % 400, mem_bytes: (64 + ((p.name.length * 31) % 900)) * 1024 * 1024, containers: [{ name: 'app', cpu_millicores: ((p.name.length * 13) % 400) * 0.8, mem_bytes: (64 + ((p.name.length * 31) % 900)) * 1024 * 1024 * 0.7 }, { name: 'sidecar', cpu_millicores: ((p.name.length * 13) % 400) * 0.2, mem_bytes: (64 + ((p.name.length * 31) % 900)) * 1024 * 1024 * 0.3 }] }))) } }; } },
  { method: 'POST', re: /^\/k8s\/clusters\/([^/]+)\/exec$/, handle: (_m, body) => ({ json: newSessionFor(body.workspace_id, { kind: 'shell', provider: 'k8s', title: `${body.pod} · ${body.ns}` }) }) },
  { method: 'POST', re: /^\/k8s\/clusters\/([^/]+)\/k9s$/, handle: () => problem(400, 'invalid', 'k9s not installed — install it from the Kubernetes overview') },
  { method: 'POST', re: /^\/k8s\/clusters\/([^/]+)\/actions$/, handle: (_m, body) => ({ json: { ok: true, message: `${body.action} ${body.kind}/${body.name} (mock)`, output: body.action === 'rollout_status' ? 'deployment "api" successfully rolled out' : null } }) },
];

const routes: Route[] = [
  ...k8sRoutes,
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
  // MeResp shape (auth.svelte.ts reads `.user` / `.real_user`) — a bare user
  // left `auth.me` undefined, so every `auth.can(...)` gate was false in mock mode.
  { method: 'GET', re: /^\/auth\/me$/, handle: () => ({ json: { user: users[0], real_user: users[0], impersonating: false } }) },

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

  // ── API client (mock) ──────────────────────────────────────────────────────
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/api-client\/collections$/, handle: (m) => ({ json: apiCollections.filter((c) => c.workspace_id === m[1]) }) },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/api-client\/collections$/,
    handle: (m, body) => {
      const b = body as { name?: string; parent_id?: string | null };
      const c = { id: `col-${++apiSeq}`, workspace_id: m[1], parent_id: b.parent_id ?? null, name: b.name ?? 'Collection', position: apiSeq, created_by: 'u1', created_at: '2026-01-01T00:00:00Z' };
      apiCollections.push(c);
      return { json: c };
    },
  },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/api-client\/requests$/, handle: (m) => ({ json: apiRequests.filter((r) => r.workspace_id === m[1]) }) },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/api-client\/requests$/,
    handle: (m, body) => {
      const b = body as Record<string, unknown>;
      const r = { id: `req-${++apiSeq}`, workspace_id: m[1], collection_id: b.collection_id ?? null, name: b.name ?? 'Request', method: b.method ?? 'GET', url: b.url ?? '', headers: b.headers ?? [], query: b.query ?? [], body_mode: b.body_mode ?? 'none', body: b.body ?? '', auth: b.auth ?? { type: 'none' }, position: apiSeq, created_at: '2026-01-01T00:00:00Z' };
      apiRequests.push(r);
      return { json: r };
    },
  },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/api-client\/environments$/, handle: () => ({ json: [] }) },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/api-client\/history$/, handle: () => ({ json: [] }) },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/api-client\/automations$/, handle: () => ({ json: [] }) },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/api-client\/execute$/,
    handle: (_m, body) => {
      const url = String((body as { url?: string })?.url ?? '');
      const method = String((body as { method?: string })?.method ?? 'GET');
      if (/\.png(\?|$)|image/i.test(url)) {
        // 1×1 transparent PNG, so the preview pane has something to render.
        const b64 =
          'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/IzeAAAAAElFTkSuQmCC';
        const size = atob(b64).length;
        return {
          json: {
            status: 200, status_text: 'OK',
            headers: [{ key: 'Content-Type', value: 'image/png' }, { key: 'Content-Length', value: String(size) }],
            body: '', body_base64: b64, truncated: false, too_large: false,
            duration_ms: 12, size_bytes: size, content_type: 'image/png',
            trace: [
              { label: 'Request', detail: `GET ${url}`, ms: null, level: 'info' },
              { label: 'Sent request', detail: '0 header(s)', ms: null, level: 'info' },
              { label: 'Waiting (TTFB)', detail: 'time to first response byte', ms: 9, level: 'timing' },
              { label: 'Downloaded', detail: `${size} B`, ms: 1, level: 'timing' },
              { label: 'Completed', detail: '200 OK', ms: 12, level: 'success' },
            ],
          },
        };
      }
      const reqBody = String((body as { body?: string })?.body ?? '');
      if (reqBody.includes('__schema')) {
        const schema = JSON.stringify({ data: { __schema: { queryType: { name: 'Query' }, types: [
          { name: 'Query', kind: 'OBJECT', fields: [{ name: 'players' }, { name: 'tournament' }] },
          { name: 'Player', kind: 'OBJECT', fields: [{ name: 'id' }, { name: 'name' }, { name: 'balance' }] },
        ] } } });
        return { json: {
          status: 200, status_text: 'OK',
          headers: [{ key: 'Content-Type', value: 'application/json' }],
          body: schema, body_base64: btoa(unescape(encodeURIComponent(schema))), truncated: false, too_large: false,
          duration_ms: 15, size_bytes: schema.length, content_type: 'application/json',
          trace: [{ label: 'Completed', detail: '200 OK', ms: 15, level: 'success' }],
        } };
      }
      const payload = JSON.stringify({ ok: true, echo: { method, url } });
      const b64 = btoa(unescape(encodeURIComponent(payload)));
      return {
        json: {
          status: 200, status_text: 'OK',
          headers: [{ key: 'Content-Type', value: 'application/json' }, { key: 'Content-Length', value: String(payload.length) }],
          body: payload, body_base64: b64, truncated: false, too_large: false,
          duration_ms: 18, size_bytes: payload.length, content_type: 'application/json',
          trace: [
            { label: 'Request', detail: `${method} ${url}`, ms: null, level: 'info' },
            { label: 'Sent request', detail: '0 header(s)', ms: null, level: 'info' },
            { label: 'Waiting (TTFB)', detail: 'time to first response byte', ms: 14, level: 'timing' },
            { label: 'Downloaded', detail: `${payload.length} B`, ms: 2, level: 'timing' },
            { label: 'Completed', detail: '200 OK', ms: 18, level: 'success' },
          ],
        },
      };
    },
  },

  { method: 'GET', re: /^\/workspaces\/([^/]+)\/api-client\/cookies$/, handle: () => ({ json: [{ name: 'session', value: 'mock-abc123', domain: 'example.com', path: '/' }] }) },
  { method: 'DELETE', re: /^\/workspaces\/([^/]+)\/api-client\/cookies$/, handle: () => ({ status: 204 }) },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/api-client\/oauth2\/token$/,
    handle: (_m, body) => {
      const grant = String((body as { grant?: string })?.grant ?? 'client_credentials');
      return { json: { access_token: `mock-token-${grant}-abc123`, token_type: 'Bearer', expires_in: 3600, refresh_token: 'mock-refresh-xyz', scope: 'read write' } };
    },
  },
  {
    method: 'POST',
    re: /^\/api-client\/import-curl$/,
    handle: (_m, body) => {
      const curl = String((body as { curl?: string })?.curl ?? '');
      const urlMatch = curl.match(/https?:\/\/[^\s'"]+/);
      const methodMatch = curl.match(/-X\s+(\w+)/i);
      return {
        json: {
          method: (methodMatch?.[1] ?? 'GET').toUpperCase(),
          url: urlMatch?.[0] ?? 'https://example.com',
          headers: [], query: [], body_mode: 'none', body: '', auth: { type: 'none' },
        },
      };
    },
  },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/api-client\/grpc\/describe$/,
    handle: () => ({
      json: {
        services: [
          {
            name: 'demo.Greeter',
            methods: [
              {
                name: 'SayHello',
                full: '/demo.Greeter/SayHello',
                input_type: 'demo.HelloRequest',
                output_type: 'demo.HelloReply',
                input_schema: '{\n  "name": "",\n  "count": 0\n}',
                client_streaming: false,
                server_streaming: false,
              },
            ],
          },
        ],
      },
    }),
  },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/api-client\/grpc\/reflect$/,
    handle: () => ({
      json: {
        services: [
          {
            name: 'demo.Greeter',
            methods: [
              { name: 'SayHello', full: '/demo.Greeter/SayHello', input_type: 'demo.HelloRequest', output_type: 'demo.HelloReply', input_schema: '{\n  "name": ""\n}', client_streaming: false, server_streaming: false },
              { name: 'StreamHellos', full: '/demo.Greeter/StreamHellos', input_type: 'demo.HelloRequest', output_type: 'demo.HelloReply', input_schema: '{\n  "name": ""\n}', client_streaming: false, server_streaming: true },
            ],
          },
        ],
      },
    }),
  },
  {
    method: 'POST',
    re: /^\/workspaces\/([^/]+)\/api-client\/grpc\/invoke$/,
    handle: (_m, body) => {
      const reqBody = (body as { body?: string })?.body ?? '{}';
      let name = 'world';
      try { name = JSON.parse(reqBody).name || 'world'; } catch { /* ignore */ }
      const payload = JSON.stringify({ message: `Hello, ${name}!`, tags: ['mock'] }, null, 2);
      return {
        json: {
          status: 200, status_text: 'OK',
          headers: [{ key: 'grpc-status', value: '0' }, { key: 'content-type', value: 'application/grpc+json' }],
          body: payload, body_base64: btoa(unescape(encodeURIComponent(payload))),
          truncated: false, too_large: false, duration_ms: 22, size_bytes: payload.length,
          content_type: 'application/grpc+json',
          trace: [
            { label: 'Request', detail: 'gRPC /demo.Greeter/SayHello', ms: null, level: 'info' },
            { label: 'Connected', detail: 'mock', ms: 8, level: 'timing' },
            { label: 'Response', detail: 'message received', ms: 12, level: 'timing' },
            { label: 'Completed', detail: 'OK (grpc-status 0)', ms: 22, level: 'success' },
          ],
        },
      };
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
        environment: body.environment ?? 'dev',
        read_only: body.read_only ?? false,
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
      if (body.section_id !== undefined) c.section_id = body.section_id;
      if (body.environment != null) c.environment = body.environment;
      if (body.read_only != null) c.read_only = body.read_only;
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
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/api-collections\/push$/,
    handle: (_m, body) => {
      const files = ((body as { files?: { name: string; content: string }[] })?.files ?? []);
      gitCollectionFiles = files;
      return { json: { commit: 'abc1234def', push: 'pushed', files: files.length } };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/api-collections\/pull$/,
    handle: () => ({ json: { files: gitCollectionFiles } }),
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
        resolved: false,
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
        c.thread_id = c.id; // new top-level comment starts its own thread
        pr.comments.push(c);
      }
      return { json: c };
    },
  },
  {
    method: 'POST',
    re: /^\/repos\/([^/]+)\/prs\/(\d+)\/comments\/([^/]+)\/resolve$/,
    handle: (m, body) => {
      const pr = prs.find((p) => p.repo_id === m[1] && p.summary.number === Number(m[2]));
      if (!pr) return problem(404, 'not_found', 'pr');
      const head = pr.comments.find((c) => c.thread_id === m[3]);
      if (!head) return problem(404, 'not_found', 'thread');
      head.resolved = body.resolved === true;
      return { status: 204 };
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


  // --- AWS console ---
  { method: 'GET', re: /^\/aws\/status$/, handle: () => ({ json: awsStatus }) },
  {
    method: 'POST',
    re: /^\/aws\/install$/,
    handle: () => {
      awsStatus.install = { tool: 'aws', state: 'running', log_tail: '==> Downloading awscli…\n', started_at: new Date().toISOString() };
      setTimeout(() => {
        awsStatus.install = { ...awsStatus.install, state: 'done', log_tail: awsStatus.install.log_tail + '🍺  awscli installed\n', finished_at: new Date().toISOString() };
        awsStatus.installed = true;
        localStorage.removeItem('otto_mock_aws_missing');
      }, 4000);
      return { json: awsStatus.install };
    },
  },
  {
    method: 'GET',
    re: /^\/aws\/discover$/,
    handle: () => ({
      json: {
        profiles: [
          { name: 'default', region: 'us-east-1', source: 'credentials' },
          { name: 'sandbox', region: 'eu-west-1', sso_start_url: 'https://acme.awsapps.com/start', sso_session: 'acme', source: 'config' },
          { name: 'prod-admin', region: 'eu-central-1', role_arn: 'arn:aws:iam::999999999999:role/Admin', source: 'config' },
        ],
      },
    }),
  },
  {
    method: 'GET',
    re: /^\/aws\/regions$/,
    handle: () => ({
      json: {
        regions: [
          ['us-east-1', 'US East (N. Virginia)'], ['us-east-2', 'US East (Ohio)'], ['us-west-1', 'US West (N. California)'],
          ['us-west-2', 'US West (Oregon)'], ['eu-west-1', 'Europe (Ireland)'], ['eu-west-2', 'Europe (London)'],
          ['eu-central-1', 'Europe (Frankfurt)'], ['eu-north-1', 'Europe (Stockholm)'], ['ap-southeast-1', 'Asia Pacific (Singapore)'],
          ['ap-northeast-1', 'Asia Pacific (Tokyo)'], ['sa-east-1', 'South America (São Paulo)'], ['il-central-1', 'Israel (Tel Aviv)'],
        ].map(([code, name]) => ({ code, name })),
      },
    }),
  },
  { method: 'GET', re: /^\/aws\/accounts$/, handle: () => ({ json: awsAccounts }) },
  {
    method: 'POST',
    re: /^\/aws\/accounts$/,
    handle: (_m, body) => {
      const a: AwsAccount = {
        id: nid('aws'),
        name: body.name,
        auth_mode: body.auth_mode,
        profile: body.profile ?? null,
        access_key_id: body.access_key_id ?? null,
        region: body.region,
        role_arn: body.role_arn ?? null,
        environment: body.environment ?? 'dev',
        color: body.color ?? null,
        identity: null,
        permissions: null,
        created_by: 'usr_root',
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      };
      awsAccounts.push(a);
      return { status: 201, json: a };
    },
  },
  { method: 'GET', re: /^\/aws\/accounts\/([^/]+)$/, handle: (m) => (awsAccount(m[1]) ? { json: awsAccount(m[1]) } : problem(404, 'not_found', 'account')) },
  {
    method: 'PATCH',
    re: /^\/aws\/accounts\/([^/]+)$/,
    handle: (m, body) => {
      const a = awsAccount(m[1]);
      if (!a) return problem(404, 'not_found', 'account');
      Object.assign(a, { ...body, secret_access_key: undefined, session_token: undefined, updated_at: new Date().toISOString() });
      return { json: a };
    },
  },
  {
    method: 'DELETE',
    re: /^\/aws\/accounts\/([^/]+)$/,
    handle: (m) => {
      const i = awsAccounts.findIndex((a) => a.id === m[1]);
      if (i >= 0) awsAccounts.splice(i, 1);
      return { status: 204 };
    },
  },
  {
    method: 'POST',
    re: /^\/aws\/accounts\/([^/]+)\/test$/,
    handle: (m) => {
      const a = awsAccount(m[1]);
      if (!a) return problem(404, 'not_found', 'account');
      const loginRequired = a.auth_mode === 'profile' && a.profile === 'prod-admin';
      return {
        json: loginRequired
          ? { ok: false, latency_ms: 120, message: 'login required: The SSO session associated with this profile has expired', login_required: true }
          : { ok: true, latency_ms: 340, message: 'ok', identity: a.identity ?? awsPermsOk.identity, login_required: false },
      };
    },
  },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/permissions$/,
    handle: (m) => {
      const a = awsAccount(m[1]);
      if (!a) return problem(404, 'not_found', 'account');
      a.permissions = a.permissions ?? { ...awsPermsOk, checked_at: new Date().toISOString() };
      return { json: a.permissions };
    },
  },
  {
    method: 'POST',
    re: /^\/aws\/accounts\/([^/]+)\/login$/,
    handle: (m, body) => {
      const a = awsAccount(m[1]);
      if (!a) return problem(404, 'not_found', 'account');
      if (a.auth_mode !== 'profile') return problem(400, 'invalid', 'access_keys accounts cannot sign in interactively');
      return { json: newSessionFor(body.workspace_id ?? 'wsp_otto', { provider: 'aws', title: `aws sso login · ${a.name}` }) };
    },
  },
  // S3
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/s3\/buckets$/,
    handle: () => ({
      json: {
        buckets: [
          { name: 'acme-data-lake', creation_date: ago(60 * 24 * 400), region: 'eu-west-1' },
          { name: 'acme-app-logs', creation_date: ago(60 * 24 * 120), region: 'eu-west-1' },
          { name: 'acme-backups', creation_date: ago(60 * 24 * 800), region: 'eu-central-1' },
        ],
      },
    }),
  },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/s3\/buckets\/([^/]+)\/objects$/,
    handle: (_m, _b, query) => {
      const prefix = query.get('prefix') ?? '';
      const depth = prefix.split('/').filter(Boolean).length;
      const prefixes = depth < 2 ? [`${prefix}2026-09-01/`, `${prefix}2026-09-02/`] : [];
      const objects = Array.from({ length: 6 }, (_, i) => ({
        key: `${prefix}${['events', 'report', 'config', 'trace', 'summary', 'data'][i]}-${i}.${['json', 'csv', 'json', 'log', 'txt', 'parquet'][i]}`,
        size: (i + 1) * 4321,
        last_modified: ago(i * 37 + 10),
        storage_class: i === 5 ? 'GLACIER' : 'STANDARD',
        etag: `"etag${i}"`,
      }));
      return { json: { prefixes, objects, is_truncated: false } };
    },
  },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/s3\/buckets\/([^/]+)\/object$/,
    handle: (_m, _b, query) => ({ json: { key: query.get('key'), size: 4321, content_type: 'application/json', last_modified: ago(10), etag: '"e"', metadata: {}, storage_class: 'STANDARD' } }),
  },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/s3\/buckets\/([^/]+)\/preview$/,
    handle: (_m, _b, query) => {
      const key = query.get('key') ?? '';
      if (key.endsWith('.parquet')) return { json: { binary: true, content_type: 'application/octet-stream' } };
      if (key.endsWith('.csv')) return { json: { text: 'id,name,amount\n1,alpha,10.5\n2,beta,20\n3,gamma,7.25\n', truncated: false, content_type: 'text/csv' } };
      if (key.endsWith('.json')) return { json: { text: JSON.stringify({ event: 'deposit', amount: 10.5, tags: ['a', 'b'], meta: { source: 'api' } }), truncated: false, content_type: 'application/json' } };
      return { json: { text: `line 1 of ${key}\nline 2\nline 3\n`, truncated: false, content_type: 'text/plain' } };
    },
  },
  // SQS
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/sqs\/queues$/,
    handle: () => ({
      json: {
        queues: [
          { url: 'https://sqs.eu-west-1.amazonaws.com/123456789012/orders', name: 'orders', fifo: false },
          { url: 'https://sqs.eu-west-1.amazonaws.com/123456789012/orders-dlq', name: 'orders-dlq', fifo: false },
          { url: 'https://sqs.eu-west-1.amazonaws.com/123456789012/payments.fifo', name: 'payments.fifo', fifo: true },
        ],
      },
    }),
  },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/sqs\/queues\/attributes$/,
    handle: (_m, _b, query) => {
      const url = query.get('url') ?? '';
      const name = url.split('/').pop() ?? '';
      const isDlq = name.endsWith('-dlq');
      return {
        json: {
          attributes: {
            QueueArn: `arn:aws:sqs:eu-west-1:123456789012:${name}`,
            ApproximateNumberOfMessages: isDlq ? '17' : '3',
            VisibilityTimeout: '30',
            MessageRetentionPeriod: '345600',
            ...(isDlq ? {} : { RedrivePolicy: JSON.stringify({ deadLetterTargetArn: 'arn:aws:sqs:eu-west-1:123456789012:orders-dlq', maxReceiveCount: 5 }) }),
          },
          approx_messages: isDlq ? 17 : 3,
          approx_not_visible: 1,
          approx_delayed: 0,
          dlq_target_arn: isDlq ? null : 'arn:aws:sqs:eu-west-1:123456789012:orders-dlq',
        },
      };
    },
  },
  {
    method: 'POST',
    re: /^\/aws\/accounts\/([^/]+)\/sqs\/queues\/peek$/,
    handle: (_m, body) => ({
      json: {
        messages: Array.from({ length: Math.min(Number(body.max ?? 10), 3) }, (_, i) => ({
          message_id: nid('msg'),
          receipt_handle: nid('rh'),
          body: JSON.stringify({ order_id: 1000 + i, status: 'created', total: 19.99 * (i + 1) }),
          attributes: { SentTimestamp: String(Date.now() - i * 60000), ApproximateReceiveCount: String(i + 1) },
          message_attributes: i === 0 ? { source: { DataType: 'String', StringValue: 'checkout' } } : {},
          md5: 'd41d8cd98f00b204e9800998ecf8427e',
        })),
      },
    }),
  },
  { method: 'POST', re: /^\/aws\/accounts\/([^/]+)\/sqs\/queues\/send$/, handle: () => ({ json: { message_id: nid('msg') } }) },
  { method: 'POST', re: /^\/aws\/accounts\/([^/]+)\/sqs\/queues\/delete-message$/, handle: () => ({ status: 204 }) },
  { method: 'POST', re: /^\/aws\/accounts\/([^/]+)\/sqs\/queues\/purge$/, handle: () => ({ status: 204 }) },
  { method: 'POST', re: /^\/aws\/accounts\/([^/]+)\/sqs\/queues\/redrive$/, handle: () => ({ json: { task_handle: nid('mv') } }) },
  // EC2
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/ec2\/instances$/,
    handle: (_m, _b, query) => {
      const region = query.get('region') ?? 'eu-west-1';
      const states = ['running', 'running', 'stopped', 'running', 'pending', 'terminated'];
      return {
        json: {
          instances: states.map((state, i) => ({
            instance_id: `i-0${(i + 1).toString(16).padStart(16, '0')}`,
            name: ['web-1', 'web-2', 'batch', 'db-replica', 'ci-runner', 'old-web'][i],
            state,
            type: ['t3.medium', 't3.medium', 'c6i.xlarge', 'r6g.large', 'm6i.large', 't2.micro'][i],
            az: `${region}${['a', 'b', 'a', 'c', 'b', 'a'][i]}`,
            private_ip: `10.0.${i}.${10 + i}`,
            public_ip: i < 2 ? `54.12.${i}.7` : null,
            launch_time: ago(60 * 24 * (i + 1) * 3),
            platform: null,
            vpc_id: 'vpc-0abc',
            subnet_id: `subnet-0${i}`,
            tags: { Name: ['web-1', 'web-2', 'batch', 'db-replica', 'ci-runner', 'old-web'][i], env: 'dev', team: 'platform' },
          })),
        },
      };
    },
  },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/ec2\/instances\/([^/]+)$/,
    handle: (m) => ({
      json: {
        instance_id: m[2], name: 'web-1', state: 'running', type: 't3.medium', az: 'eu-west-1a', private_ip: '10.0.0.10', public_ip: '54.12.0.7',
        launch_time: ago(60 * 24 * 3), platform: null, vpc_id: 'vpc-0abc', subnet_id: 'subnet-00', tags: { Name: 'web-1', env: 'dev' },
        raw: { InstanceId: m[2], ImageId: 'ami-0123', State: { Code: 16, Name: 'running' }, BlockDeviceMappings: [{ DeviceName: '/dev/xvda', Ebs: { VolumeId: 'vol-1', Status: 'attached' } }], SecurityGroups: [{ GroupId: 'sg-1', GroupName: 'web' }] },
      },
    }),
  },
  {
    method: 'POST',
    re: /^\/aws\/accounts\/([^/]+)\/ec2\/instances\/([^/]+)\/(start|stop|reboot)$/,
    handle: (m) => ({ json: { previous_state: m[3] === 'start' ? 'stopped' : 'running', current_state: m[3] === 'start' ? 'pending' : m[3] === 'stop' ? 'stopping' : 'running' } }),
  },
  // Athena
  { method: 'GET', re: /^\/aws\/accounts\/([^/]+)\/athena\/workgroups$/, handle: () => ({ json: { workgroups: [{ name: 'primary', state: 'ENABLED', output_location: 's3://acme-athena-results/' }, { name: 'analytics', state: 'ENABLED', output_location: null }] } }) },
  { method: 'GET', re: /^\/aws\/accounts\/([^/]+)\/athena\/databases$/, handle: () => ({ json: { databases: ['default', 'logs', 'billing'] } }) },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/athena\/tables$/,
    handle: (_m, _b, query) => {
      const db = query.get('database') ?? 'default';
      const tables = db === 'logs'
        ? [
            { name: 'requests', type: 'EXTERNAL_TABLE', columns: [{ name: 'ts', type: 'timestamp' }, { name: 'path', type: 'string' }, { name: 'status', type: 'int' }, { name: 'latency_ms', type: 'bigint' }, { name: 'dt', type: 'date' }] },
            { name: 'errors', type: 'EXTERNAL_TABLE', columns: [{ name: 'ts', type: 'timestamp' }, { name: 'message', type: 'string' }, { name: 'service', type: 'string' }] },
          ]
        : db === 'billing'
          ? [{ name: 'cur', type: 'EXTERNAL_TABLE', columns: [{ name: 'line_item_usage_account_id', type: 'string' }, { name: 'line_item_unblended_cost', type: 'double' }, { name: 'bill_billing_period_start_date', type: 'timestamp' }] }]
          : [{ name: 'sample', type: 'EXTERNAL_TABLE', columns: [{ name: 'id', type: 'bigint' }, { name: 'name', type: 'string' }] }];
      return { json: { tables } };
    },
  },
  { method: 'GET', re: /^\/aws\/accounts\/([^/]+)\/athena\/history$/, handle: () => ({ json: { executions: athenaHistory } }) },
  {
    method: 'POST',
    re: /^\/aws\/accounts\/([^/]+)\/athena\/query$/,
    handle: (_m, body) => {
      if (body.workgroup === 'analytics' && !body.output_location) return problem(400, 'invalid', 'workgroup "analytics" has no output location — pass output_location or pick another workgroup');
      athenaPolls = 0;
      const id = nid('qx');
      athenaHistory.unshift({ id, query: body.sql, state: 'RUNNING', submitted_at: new Date().toISOString() });
      return { json: { query_execution_id: id } };
    },
  },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/athena\/query\/([^/]+)$/,
    handle: (m) => {
      const h = athenaHistory.find((x) => x.id === m[2]);
      if (h && h.state === 'FAILED') return { json: { state: 'FAILED', reason: 'SYNTAX_ERROR: line 1:8: Column \'*\' cannot be resolved', stats: { data_scanned_bytes: 0, execution_ms: 300 } } };
      athenaPolls += 1;
      if (h && h.state === 'RUNNING' && athenaPolls < 3) return { json: { state: athenaPolls === 1 ? 'QUEUED' : 'RUNNING', stats: { data_scanned_bytes: athenaPolls * 40_000_000, execution_ms: athenaPolls * 900 } } };
      if (h) { h.state = 'SUCCEEDED'; h.completed_at = new Date().toISOString(); h.data_scanned_bytes = 128_000_000; h.execution_ms = 2700; }
      return {
        json: {
          state: 'SUCCEEDED',
          stats: { data_scanned_bytes: 128_000_000, execution_ms: 2700 },
          result: {
            columns: [{ name: 'dt', type_hint: 'date' }, { name: 'requests', type_hint: 'bigint' }, { name: 'p95_ms', type_hint: 'double' }],
            rows: Array.from({ length: 14 }, (_, i) => [`2026-08-${String(i + 1).padStart(2, '0')}`, 120000 + i * 913, 180.5 + i]),
            stats: { duration_ms: 2700, row_count: 14, bytes_read: 128_000_000 },
            truncated: false,
          },
        },
      };
    },
  },
  { method: 'POST', re: /^\/aws\/accounts\/([^/]+)\/athena\/query\/([^/]+)\/cancel$/, handle: () => ({ status: 204 }) },
  // EKS
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/eks\/clusters$/,
    handle: () => ({
      json: {
        clusters: [
          { name: 'platform-dev', status: 'ACTIVE', version: '1.30', endpoint: 'https://ABC.gr7.eu-west-1.eks.amazonaws.com', arn: 'arn:aws:eks:eu-west-1:123456789012:cluster/platform-dev', created_at: ago(60 * 24 * 200) },
          { name: 'data-staging', status: 'UPDATING', version: '1.29', endpoint: 'https://DEF.gr7.eu-west-1.eks.amazonaws.com', arn: 'arn:aws:eks:eu-west-1:123456789012:cluster/data-staging', created_at: ago(60 * 24 * 90) },
        ],
      },
    }),
  },
  {
    method: 'GET',
    re: /^\/aws\/accounts\/([^/]+)\/eks\/clusters\/([^/]+)$/,
    handle: (m) => ({
      json: {
        cluster: { name: m[2], status: 'ACTIVE', version: '1.30', platformVersion: 'eks.5', kubernetesNetworkConfig: { serviceIpv4Cidr: '172.20.0.0/16' }, logging: { clusterLogging: [{ types: ['api', 'audit'], enabled: true }] } },
        nodegroups: [
          { name: 'general', status: 'ACTIVE', desired: 3, min: 2, max: 6, instance_types: ['m6i.large'], ami_type: 'AL2023_x86_64_STANDARD' },
          { name: 'spot', status: 'ACTIVE', desired: 4, min: 0, max: 20, instance_types: ['c6i.xlarge', 'c5.xlarge'], ami_type: 'AL2023_x86_64_STANDARD' },
        ],
      },
    }),
  },
  {
    method: 'POST',
    re: /^\/aws\/accounts\/([^/]+)\/eks\/clusters\/([^/]+)\/import-kubeconfig$/,
    handle: (m) => ({ json: { id: nid('k8s'), name: m[2], source: 'eks', context_name: m[2], environment: 'dev' } }),
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
    if (typeof out.raw === 'string') return { status: out.status ?? 200, body: out.raw };
    if (out.status === 204 && out.json === undefined) return { status: 204, body: null };
    return { status: out.status ?? 200, body: JSON.stringify(out.json) };
  }
  return { status: 404, body: JSON.stringify({ code: 'not_found', message: `${method} ${path}` }) };
}

// Simulates the daemon's /ws/api-client/stream relay (SSE + WebSocket).
class MockStreamSocket {
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  readyState = 0;
  private closed = false;
  private timers: ReturnType<typeof setTimeout>[] = [];

  constructor(_url: string) {
    this.later(30, () => {
      this.readyState = 1;
      this.onopen?.(new Event('open'));
    });
  }
  private later(ms: number, fn: () => void): void {
    this.timers.push(setTimeout(() => !this.closed && fn(), ms));
  }
  private emit(obj: unknown): void {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(obj) }));
  }
  send(raw: string): void {
    let msg: { action?: string; kind?: string; data?: string };
    try { msg = JSON.parse(raw); } catch { return; }
    if (msg.action === 'open') {
      if (msg.kind === 'sse') {
        this.later(40, () => this.emit({ type: 'open', detail: '200 — streaming events (mock)' }));
        for (let i = 1; i <= 3; i++) {
          this.later(120 * i, () => this.emit({ type: 'event', event: 'tick', data: `event ${i}`, id: String(i) }));
        }
        this.later(120 * 4, () => this.emit({ type: 'closed', detail: 'stream ended' }));
      } else {
        this.later(40, () => this.emit({ type: 'open', detail: 'connected (mock echo)' }));
      }
    } else if (msg.action === 'send') {
      this.emit({ type: 'message', dir: 'out', data: msg.data ?? '', binary: false });
      this.later(60, () => this.emit({ type: 'message', dir: 'in', data: `echo: ${msg.data ?? ''}`, binary: false }));
    } else if (msg.action === 'close') {
      this.emit({ type: 'closed', detail: 'disconnected' });
      this.close();
    }
  }
  close(): void {
    this.closed = true;
    this.timers.forEach(clearTimeout);
    this.readyState = 3;
    this.onclose?.(new CloseEvent('close'));
  }
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
    if (u.includes('/ws/api-client/stream')) return new MockStreamSocket(u) as unknown as WebSocket;
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
