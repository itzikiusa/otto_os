import { request, type APIRequestContext } from '@playwright/test';
import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';

// Helpers to seed REAL data into the isolated test daemon so deep specs can
// exercise data-dependent behavior (terminal output/input, DB results grid,
// diff file-list scroll) — the things the empty-state baseline can't catch.
//
// Reads the daemon port + root token written by global-setup.

interface DaemonInfo {
  base: string;
  token: string;
}

export function daemonInfo(): DaemonInfo {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const dir = join(process.cwd(), 'e2e', `.auth-${slot}`);
  const meta = JSON.parse(readFileSync(join(dir, 'daemon.json'), 'utf8')) as { port: string };
  const state = JSON.parse(readFileSync(join(dir, 'state.json'), 'utf8')) as {
    origins: { localStorage: { name: string; value: string }[] }[];
  };
  const token =
    state.origins[0]?.localStorage.find((l) => l.name === 'otto_token')?.value ?? '';
  return { base: `http://127.0.0.1:${meta.port}`, token };
}

export async function apiCtx(): Promise<{ ctx: APIRequestContext; base: string; token: string }> {
  const { base, token } = daemonInfo();
  const ctx = await request.newContext({
    extraHTTPHeaders: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
  });
  return { ctx, base, token };
}

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  if (!r.ok()) throw new Error(`POST ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}

export async function seedWorkspace(ctx: APIRequestContext, base: string): Promise<string> {
  const root = mkdtempSync(join(tmpdir(), 'otto-e2e-ws-'));
  const ws = await postJson(ctx, `${base}/api/v1/workspaces`, { name: 'E2E WS', root_path: root });
  return ws.id as string;
}

/** Seed the Vault (workspace memory store) with several notes — one long body
 *  with [[backlinks]] so the reader/editor overflows and is scroll-testable — and
 *  import a small knowledge graph (nodes + edges) so the Graph view has content.
 *  Returns the created memory ids (first is the long "hub" note). */
export async function seedVaultNotes(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
): Promise<string[]> {
  const mk = (
    title: string,
    body: string,
    kind: string,
    extra: Record<string, unknown> = {},
  ) =>
    postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/memories`, {
      scope: 'workspace',
      kind,
      title,
      body,
      source_kind: 'manual',
      ...extra,
    });

  // A long hub note — body intentionally large so the reader column overflows the
  // phone viewport vertically (scroll target) and references siblings via [[..]].
  const longBody = [
    '# Architecture overview',
    '',
    'Otto is composed of a Rust daemon (`ottod`) and a Svelte UI. See [[Daemon design]]',
    'and [[UI conventions]] for the split. The vault stores workspace knowledge as',
    'notes with backlinks, hybrid search, and a knowledge graph.',
    '',
    ...Array.from({ length: 40 }, (_, i) =>
      `- Detail line ${i + 1}: the quick brown fox jumps over the lazy dog, repeatedly, to fill vertical space and force the note body to overflow a phone viewport so scrolling can be asserted.`,
    ),
    '',
    'Related: [[Daemon design]], [[UI conventions]], [[Testing strategy]].',
  ].join('\n');

  const ids: string[] = [];
  const hub = await mk('Architecture overview', longBody, 'entity', {
    tags: ['architecture', 'overview'],
    entities: ['otto'],
  });
  ids.push(hub.id as string);
  const daemon = await mk(
    'Daemon design',
    'The daemon is an Axum HTTP+WebSocket server on 127.0.0.1:7700, loopback only by default. It owns sessions, PTYs, git, reviews, channels, and SQLite state.',
    'decision',
    { tags: ['daemon', 'rust'] },
  );
  ids.push(daemon.id as string);
  const ui = await mk(
    'UI conventions',
    'The UI is Svelte 5 + Vite + TypeScript. Contracts in docs/contracts are authoritative; the TS types mirror them. Match the surrounding code.',
    'constraint',
    { tags: ['ui', 'svelte'] },
  );
  ids.push(ui.id as string);
  const testing = await mk(
    'Testing strategy',
    'Mobile/tablet E2E runs the real UI against an isolated throwaway daemon. Verify both portrait and landscape, no horizontal overflow, sections independently scrollable.',
    'qa',
    { tags: ['testing', 'e2e'] },
  );
  ids.push(testing.id as string);

  // Import a knowledge graph (edges) so the Graph view shows links, not just nodes.
  try {
    await postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/memory/import-graph`, {
      collection: 'default',
      graph: {
        nodes: [
          { id: 'otto', label: 'Otto', kind: 'entity' },
          { id: 'daemon', label: 'Daemon', kind: 'entity' },
          { id: 'ui', label: 'UI', kind: 'entity' },
          { id: 'vault', label: 'Vault', kind: 'entity' },
        ],
        edges: [
          { source: 'otto', target: 'daemon', rel: 'has', certainty: 'EXTRACTED' },
          { source: 'otto', target: 'ui', rel: 'has', certainty: 'EXTRACTED' },
          { source: 'ui', target: 'vault', rel: 'contains', certainty: 'INFERRED' },
          { source: 'daemon', target: 'vault', rel: 'serves', certainty: 'EXTRACTED' },
        ],
      },
    });
  } catch {
    // import-graph is best-effort; the graph view still renders memory nodes.
  }

  return ids;
}

/**
 * Seed a swarm with a real org tree + a project + tasks (some with dependencies)
 * + a few board posts, so the Swarm page's views render with content:
 *   - Org tree:  multi-level hierarchy from the `engineering-squad` preset
 *   - Kanban:    a project with cards across several status columns
 *   - Graph:     tasks + depends-on edges → a real DAG
 *   - Board:     a handful of messages
 * Live coordinator data (runs with live sessions/tokens) isn't produced — the
 * isolated daemon doesn't actually spawn agent CLIs — so the Runs/Graph "run"
 * surfaces show their empty/idle layout, which is what these layout tests assert.
 * Returns the ids callers need to drive the UI.
 */
export async function seedSwarm(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
): Promise<{ swarmId: string; projectId: string; taskIds: string[] }> {
  // Preset gives a deep org tree (vp → lead → be1/be2/fe/devops/reviewer + pm)
  // and at least one project — independent of which agent CLIs are installed.
  const sw = await postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/swarm/swarms`, {
    name: 'E2E Swarm',
    preset_slug: 'engineering-squad',
  });
  const swarmId = sw.id as string;

  // Re-fetch detail to discover the preset's project + agents.
  const detailRes = await ctx.get(`${base}/api/v1/swarm/swarms/${swarmId}`);
  const d = (await detailRes.json()) as {
    projects: { id: string }[];
    agents: { id: string }[];
  };
  let projectId = d.projects[0]?.id ?? '';
  if (!projectId) {
    const p = await postJson(ctx, `${base}/api/v1/swarm/swarms/${swarmId}/projects`, {
      name: 'E2E Project',
      goal_md: 'Ship the mobile swarm page.',
    });
    projectId = p.id as string;
  }
  const agentIds = d.agents.map((a) => a.id);
  const pick = (i: number): string | undefined =>
    agentIds.length ? agentIds[i % agentIds.length] : undefined;

  // Tasks spread across columns + a dependency chain so the Kanban has cards in
  // multiple columns and the Graph has a non-trivial DAG.
  const taskDefs: { title: string; status?: string; priority?: string; assignee?: string }[] = [
    { title: 'Design data model', status: 'done', priority: 'high', assignee: pick(0) },
    { title: 'Build API endpoints', status: 'in_progress', priority: 'urgent', assignee: pick(1) },
    { title: 'Wire up frontend', status: 'todo', priority: 'medium', assignee: pick(2) },
    { title: 'Write integration tests', status: 'todo', priority: 'low', assignee: pick(3) },
    { title: 'Security review', status: 'in_review', priority: 'high', assignee: pick(4) },
    { title: 'Plan release', status: 'backlog', priority: 'medium', assignee: pick(5) },
    { title: 'Fix flaky CI', status: 'blocked', priority: 'urgent', assignee: pick(0) },
  ];
  const taskIds: string[] = [];
  for (const t of taskDefs) {
    const created = await postJson(ctx, `${base}/api/v1/swarm/projects/${projectId}/tasks`, {
      title: t.title,
      priority: t.priority ?? 'medium',
      ...(t.assignee ? { assignee_agent_id: t.assignee } : {}),
    });
    const id = created.id as string;
    taskIds.push(id);
    // Move to the target column (create defaults to backlog).
    if (t.status && t.status !== 'backlog') {
      await ctx.patch(`${base}/api/v1/swarm/tasks/${id}`, { data: { status: t.status } });
    }
  }
  // Wire a couple of dependencies so the graph DAG has edges.
  if (taskIds.length >= 4) {
    await ctx.patch(`${base}/api/v1/swarm/tasks/${taskIds[1]}`, {
      data: { depends_on: [taskIds[0]] },
    });
    await ctx.patch(`${base}/api/v1/swarm/tasks/${taskIds[2]}`, {
      data: { depends_on: [taskIds[1]] },
    });
    await ctx.patch(`${base}/api/v1/swarm/tasks/${taskIds[3]}`, {
      data: { depends_on: [taskIds[1]] },
    });
  }

  // A few board posts so the Feed renders messages.
  for (const m of [
    { body: 'Kicking off the project — assignments are out.', kind: 'message' },
    { body: 'Proposal: use the new layout engine for the graph.', kind: 'idea' },
    { body: 'Decided to ship behind a flag first.', kind: 'decision' },
    { body: 'Heads up: CI is flaky on the security job.', kind: 'concern' },
  ]) {
    await ctx.post(`${base}/api/v1/swarm/swarms/${swarmId}/board`, {
      data: { ...m, project_id: projectId },
    });
  }

  return { swarmId, projectId, taskIds };
}

export async function seedShellSession(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
): Promise<string> {
  const s = await postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/sessions`, {
    kind: 'agent',
    provider: 'shell',
    title: 'E2E Shell',
    cwd: '/tmp',
    meta: { origin: 'manual' },
  });
  return s.id as string;
}

/** Create a temp git repo with one commit that touches MANY files (so the diff
 *  file navigator overflows and scroll behavior can be asserted). Returns repoId. */
export async function seedGitRepo(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
): Promise<{ repoId: string; dir: string }> {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-repo-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  for (let i = 0; i < 25; i++) {
    writeFileSync(join(dir, `file_${String(i).padStart(2, '0')}.txt`), `line ${i}\nmore ${i}\n`);
  }
  git('add', '-A');
  git('commit', '-q', '-m', 'E2E: many files');
  const repo = await postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/repos`, {
    path: dir,
    name: 'e2e-repo',
  });
  return { repoId: repo.id as string, dir };
}

/** Create a temp git repo left MID-MERGE with a real conflict in one file, so
 *  the Conflict Resolver view renders (ours/theirs/base segments) and its mobile
 *  3-pane→stacked layout can be asserted. The conflicted line is intentionally
 *  long (140 chars) so a wrap/overflow regression would cut it off off-screen.
 *  Returns the registered repoId + the conflicted file path. */
export async function seedConflictRepo(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
): Promise<{ repoId: string; dir: string; file: string }> {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-conflict-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  const file = 'conflict.txt';
  const LONG = 'x'.repeat(140);
  // Long, IDENTICAL surrounding lines so the conflict view's context rows (not
  // just the conflicting line) also carry 140-char content — exercising context
  // wrapping too. Only the middle line differs between the branches → conflicts.
  const pre = `unchanged context above ${LONG}`;
  const post = `unchanged context below ${LONG}`;
  // Base commit on the default branch.
  writeFileSync(join(dir, file), `${pre}\nshared value ${LONG}\n${post}\n`);
  git('add', '-A');
  git('commit', '-q', '-m', 'base');
  // A feature branch changes the shared line.
  git('checkout', '-q', '-b', 'feature');
  writeFileSync(join(dir, file), `${pre}\nfeature change ${LONG}\n${post}\n`);
  git('add', '-A');
  git('commit', '-q', '-m', 'feature edit');
  // Back to the default branch (checkout '-'), change the SAME line differently.
  git('checkout', '-q', '-');
  writeFileSync(join(dir, file), `${pre}\nmainline change ${LONG}\n${post}\n`);
  git('add', '-A');
  git('commit', '-q', '-m', 'mainline edit');
  // Merge feature → conflict. `git merge` exits non-zero and leaves the tree
  // mid-merge (MERGE_HEAD + unmerged paths); swallow the non-zero exit.
  try {
    git('merge', '--no-edit', 'feature');
  } catch {
    /* expected: the merge conflicts, which is exactly what we want to seed */
  }
  const repo = await postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/repos`, {
    path: dir,
    name: 'e2e-conflict',
  });
  return { repoId: repo.id as string, dir, file };
}

/** Create a temp git repo with a committed base AND a DIRTY working tree (one
 *  modified tracked file + one untracked file), so the Changes view shows real
 *  staging rows + an enabled commit composer — letting the mobile staging→commit
 *  layout (touch targets, the no-iOS-zoom 16px textarea) be asserted. Returns
 *  repoId + dir. */
export async function seedDirtyRepo(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
): Promise<{ repoId: string; dir: string }> {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-dirty-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  for (let i = 0; i < 4; i++) {
    writeFileSync(join(dir, `tracked_${i}.txt`), `original ${i}\n`);
  }
  git('add', '-A');
  git('commit', '-q', '-m', 'base');
  // Leave the tree dirty: modify a tracked file + add an untracked one. No commit.
  writeFileSync(join(dir, 'tracked_0.txt'), `original 0\nMODIFIED on the working tree\n`);
  writeFileSync(join(dir, 'untracked_new.txt'), `brand new file\n`);
  const repo = await postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/repos`, {
    path: dir,
    name: 'e2e-dirty',
  });
  return { repoId: repo.id as string, dir };
}

/** Spawn an ephemeral redis-server, seed keys, and register a Dev connection.
 *  Returns the child (to kill in teardown) + connectionId, or null if redis
 *  isn't available / the connection endpoint rejects it. */
export async function seedRedis(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
  port = Number(process.env.OTTO_E2E_REDIS_PORT ?? '6399'),
): Promise<{ proc: ChildProcess; connId: string } | null> {
  let proc: ChildProcess;
  try {
    proc = spawn('redis-server', ['--port', String(port), '--save', '', '--appendonly', 'no'], {
      stdio: 'ignore',
    });
  } catch {
    return null;
  }
  // wait for redis to accept connections
  let up = false;
  for (let i = 0; i < 40; i++) {
    try {
      const out = execFileSync('redis-cli', ['-p', String(port), 'ping'], { encoding: 'utf8' });
      if (out.trim() === 'PONG') {
        up = true;
        break;
      }
    } catch {
      /* not up */
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  if (!up) {
    proc.kill('SIGKILL');
    return null;
  }
  for (let i = 0; i < 40; i++) {
    execFileSync('redis-cli', ['-p', String(port), 'SET', `e2e:key:${i}`, `value-${i}`], {
      stdio: 'ignore',
    });
  }
  try {
    const conn = await postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/connections`, {
      name: 'e2e-redis',
      kind: 'redis',
      params: { host: '127.0.0.1', port, db: 0 },
      secret: null,
      environment: 'dev',
      read_only: false,
    });
    return { proc, connId: conn.id as string };
  } catch (e) {
    proc.kill('SIGKILL');
    throw e;
  }
}

/**
 * Register a connection against the seeded Docker dev DB stack
 * (`dev/dbviewer/docker-compose.yml`) and verify it's reachable.
 *
 * Engines + endpoints (all loopback, non-standard ports so they never clash
 * with the user's own local DBs):
 *   - mysql      127.0.0.1:13306  otto/ottopw   db `shopdb`
 *   - redis      127.0.0.1:16379  (auth ottoredis) db 0
 *   - mongodb    127.0.0.1:17017  otto/ottopw   db `shopdb` (authSource admin)
 *   - clickhouse 127.0.0.1:18123  otto/ottopw   db `analytics`
 *
 * Creates the profile as `environment: 'dev'`, `read_only: false` so the sweep
 * can exercise writes (INSERT/UPDATE), then calls `/db/.../test` via the
 * connection `/test` endpoint. Returns the connection id, or `null` when the
 * engine isn't reachable (stack not up) so specs can `test.skip` cleanly
 * instead of failing the whole run.
 */
export async function seedDockerConnection(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
  kind: 'mysql' | 'redis' | 'mongodb' | 'clickhouse',
): Promise<string | null> {
  const specs: Record<
    string,
    { name: string; params: Record<string, unknown>; secret: string | null }
  > = {
    mysql: {
      name: 'e2e-mysql',
      params: { host: '127.0.0.1', port: 13306, user: 'otto', db: 'shopdb' },
      secret: 'ottopw',
    },
    redis: {
      name: 'e2e-redis-docker',
      params: { host: '127.0.0.1', port: 16379, db: 0 },
      secret: 'ottoredis',
    },
    mongodb: {
      name: 'e2e-mongodb',
      params: {
        host: '127.0.0.1',
        port: 17017,
        user: 'otto',
        db: 'shopdb',
        auth_source: 'admin',
      },
      secret: 'ottopw',
    },
    clickhouse: {
      name: 'e2e-clickhouse',
      params: { host: '127.0.0.1', port: 18123, user: 'otto', db: 'analytics' },
      secret: 'ottopw',
    },
  };
  const s = specs[kind];
  let connId: string;
  try {
    const conn = await postJson(ctx, `${base}/api/v1/workspaces/${workspaceId}/connections`, {
      name: s.name,
      kind,
      params: s.params,
      secret: s.secret,
      environment: 'dev',
      read_only: false,
    });
    connId = conn.id as string;
  } catch {
    return null;
  }
  // Confirm the engine is actually reachable before any spec depends on it.
  try {
    const r = await ctx.post(`${base}/api/v1/connections/${connId}/test`, { data: {} });
    if (!r.ok()) return null;
    const body = (await r.json()) as { ok?: boolean };
    if (body.ok === false) return null;
  } catch {
    return null;
  }
  return connId;
}

export function writeSeed(obj: Record<string, unknown>): void {
  writeFileSync(join(process.cwd(), 'e2e', '.auth', 'seed.json'), JSON.stringify(obj, null, 2));
}

export function readSeed(): Record<string, any> {
  return JSON.parse(readFileSync(join(process.cwd(), 'e2e', '.auth', 'seed.json'), 'utf8'));
}

// (homedir import kept for parity with global-setup; not used directly here.)
void homedir;
