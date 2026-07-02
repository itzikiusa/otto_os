// Fixtures for the runtime-plugins E2E: a mock Jira Cloud (REST v3 subset the
// team-performance sidecar consumes), a scripted git repo carrying BOTH
// team-performance signals (issue-key merges into develop) and DORA signals
// (deploy tags + hotfix/release/feature merges), and install/enable helpers.
//
// The mock dataset mirrors examples/plugins/team-performance/test/fixtures/
// mock-jira.js (kept independent on purpose — neither suite reaches into the
// other's tree): 2 assignees, 6 done + 2 in-progress issues, June 2026.
import http from 'node:http';
import { execFileSync } from 'node:child_process';
import { appendFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { APIRequestContext } from '@playwright/test';

// ---------------------------------------------------------------------------
// Mock Jira
// ---------------------------------------------------------------------------

type History = { created: string; items: { field: string; fromString: string; toString: string }[] };
type Issue = {
  key: string;
  fields: Record<string, unknown> & { updated: string };
  changelog: { histories: History[] };
};

const h = (created: string, from: string, to: string): History => ({
  created,
  items: [{ field: 'status', fromString: from, toString: to }],
});

const ALICE = { accountId: 'u-alice', displayName: 'Alice' };
const BOB = { accountId: 'u-bob', displayName: 'Bob' };

function issue(
  key: string,
  type: string,
  points: number | null,
  assignee: typeof ALICE,
  status: string,
  category: string,
  created: string,
  resolved: string | null,
  updated: string,
  histories: History[],
): Issue {
  return {
    key,
    fields: {
      summary: `${key}: fixture ${type.toLowerCase()}`,
      issuetype: { name: type },
      status: { name: status, statusCategory: { key: category } },
      assignee,
      created,
      resolutiondate: resolved,
      updated,
      customfield_10016: points,
      timeoriginalestimate: null,
    },
    changelog: { histories },
  };
}

function dataset(): Issue[] {
  return [
    issue('TP-1', 'Story', 3, ALICE, 'Done', 'done', '2026-06-01T09:00:00Z', '2026-06-05T10:00:00Z', '2026-06-05T10:00:00Z', [
      h('2026-06-01T10:00:00Z', 'To Do', 'In Design'),
      h('2026-06-02T10:00:00Z', 'In Design', 'In Progress'),
      h('2026-06-04T10:00:00Z', 'In Progress', 'In Review'),
      h('2026-06-05T10:00:00Z', 'In Review', 'Done'),
    ]),
    issue('TP-2', 'Story', 3, ALICE, 'Done', 'done', '2026-06-08T09:00:00Z', '2026-06-10T15:30:00Z', '2026-06-10T15:30:00Z', [
      h('2026-06-08T09:30:00Z', 'To Do', 'In Design'),
      h('2026-06-08T15:30:00Z', 'In Design', 'In Progress'),
      h('2026-06-10T15:30:00Z', 'In Progress', 'Done'),
    ]),
    issue('TP-3', 'Story', 5, ALICE, 'Done', 'done', '2026-06-08T09:00:00Z', '2026-06-16T10:00:00Z', '2026-06-16T10:00:00Z', [
      h('2026-06-09T10:00:00Z', 'To Do', 'In Design'),
      h('2026-06-10T10:00:00Z', 'In Design', 'In Progress'),
      h('2026-06-16T10:00:00Z', 'In Progress', 'Done'),
    ]),
    issue('TP-4', 'Story', 3, BOB, 'Done', 'done', '2026-06-01T09:00:00Z', '2026-06-10T09:00:00Z', '2026-06-10T09:00:00Z', [
      h('2026-06-02T09:00:00Z', 'To Do', 'In Design'),
      h('2026-06-04T09:00:00Z', 'In Design', 'In Progress'),
      h('2026-06-10T09:00:00Z', 'In Progress', 'Done'),
    ]),
    issue('TP-5', 'Task', null, BOB, 'Done', 'done', '2026-06-15T09:00:00Z', '2026-06-17T10:00:00Z', '2026-06-17T10:00:00Z', [
      h('2026-06-15T10:00:00Z', 'To Do', 'In Progress'),
      h('2026-06-17T10:00:00Z', 'In Progress', 'Done'),
    ]),
    issue('TP-6', 'Story', 3, BOB, 'Done', 'done', '2026-06-15T09:00:00Z', '2026-06-19T09:00:00Z', '2026-06-19T09:00:00Z', [
      h('2026-06-16T09:00:00Z', 'To Do', 'In Design'),
      h('2026-06-17T09:00:00Z', 'In Design', 'In Progress'),
      h('2026-06-19T09:00:00Z', 'In Progress', 'Done'),
    ]),
    issue('TP-7', 'Story', 3, ALICE, 'In Progress', 'indeterminate', '2026-06-20T09:00:00Z', null, '2026-06-24T10:00:00Z', [
      h('2026-06-22T10:00:00Z', 'To Do', 'In Design'),
      h('2026-06-24T10:00:00Z', 'In Design', 'In Progress'),
    ]),
    issue('TP-8', 'Task', null, BOB, 'In Progress', 'indeterminate', '2026-06-26T09:00:00Z', null, '2026-06-29T09:00:00Z', [
      h('2026-06-29T09:00:00Z', 'To Do', 'In Progress'),
    ]),
  ];
}

export interface MockJira {
  port: number;
  baseUrl: string;
  close(): Promise<void>;
}

/** Loopback mock Jira the sidecar can call with the credentials Otto hands it. */
export function startMockJira(): Promise<MockJira> {
  const issues = dataset();

  const jqlFilter = (jql: string | null): Issue[] => {
    const m = /updated >= "([0-9-]+ [0-9:]+)"/.exec(jql ?? '');
    if (!m) return issues;
    const cutoff = Date.parse(m[1].replace(' ', 'T') + ':00Z');
    return issues.filter((i) => Date.parse(i.fields.updated) >= cutoff);
  };

  const server = http.createServer((req, res) => {
    const u = new URL(req.url ?? '/', 'http://localhost');
    const send = (code: number, obj: unknown) => {
      res.writeHead(code, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify(obj));
    };

    if (u.pathname === '/rest/api/3/field') {
      return send(200, [
        { id: 'summary', name: 'Summary' },
        { id: 'customfield_10016', name: 'Story point estimate' },
      ]);
    }
    if (u.pathname === '/rest/api/3/search/approximate-count') {
      let body = '';
      req.on('data', (c) => (body += c));
      req.on('end', () => {
        let jql = '';
        try {
          jql = (JSON.parse(body) as { jql?: string }).jql ?? '';
        } catch {
          /* empty body */
        }
        send(200, { count: jqlFilter(jql).length });
      });
      return;
    }
    if (u.pathname === '/rest/api/3/search/jql') {
      const matched = jqlFilter(u.searchParams.get('jql'));
      const pageSize = 5;
      const start = parseInt(u.searchParams.get('nextPageToken') ?? '0', 10);
      const slice = matched.slice(start, start + pageSize);
      const out: Record<string, unknown> = {
        issues: slice.map((i) => ({ key: i.key, fields: { updated: i.fields.updated } })),
        isLast: start + pageSize >= matched.length,
      };
      if (!out.isLast) out.nextPageToken = String(start + pageSize);
      return send(200, out);
    }
    const im = /^\/rest\/api\/3\/issue\/([A-Z0-9-]+)$/.exec(u.pathname);
    if (im) {
      const found = issues.find((i) => i.key === im[1]);
      return found ? send(200, found) : send(404, { errorMessages: ['no issue'] });
    }
    if (/^\/rest\/api\/3\/project\/[^/]+\/statuses$/.test(u.pathname)) {
      return send(200, [
        {
          name: 'Story',
          statuses: [
            { name: 'To Do', statusCategory: { key: 'new' } },
            { name: 'In Design', statusCategory: { key: 'indeterminate' } },
            { name: 'In Progress', statusCategory: { key: 'indeterminate' } },
            { name: 'In Review', statusCategory: { key: 'indeterminate' } },
            { name: 'Done', statusCategory: { key: 'done' } },
          ],
        },
      ]);
    }
    if (u.pathname === '/rest/api/3/project/search') {
      return send(200, { values: [{ key: 'TP', name: 'Team Performance Fixture' }] });
    }
    return send(404, { errorMessages: ['not found'] });
  });

  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const port = (server.address() as { port: number }).port;
      resolve({
        port,
        baseUrl: `http://127.0.0.1:${port}`,
        close: () => new Promise<void>((r) => server.close(() => r())),
      });
    });
  });
}

// ---------------------------------------------------------------------------
// Fixture git repo (team-performance delivery + DORA signals)
// ---------------------------------------------------------------------------

/** Scripted repo: issue-key feature merges into develop, release/hotfix merges,
 *  and `*-deployed` tags — deterministic timestamps in June 2026. */
export function makeFixtureRepo(): string {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-plugins-repo-'));
  const g = (args: string[], when?: string) =>
    execFileSync('git', ['-C', dir, ...args], {
      encoding: 'utf8',
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: 'e2e',
        GIT_AUTHOR_EMAIL: 'e2e@otto.local',
        GIT_COMMITTER_NAME: 'e2e',
        GIT_COMMITTER_EMAIL: 'e2e@otto.local',
        ...(when ? { GIT_AUTHOR_DATE: when, GIT_COMMITTER_DATE: when } : {}),
      },
    });
  const commit = (msg: string, when: string) => {
    appendFileSync(join(dir, 'f.txt'), msg + '\n');
    g(['add', '.']);
    g(['commit', '-q', '-m', msg], when);
  };
  const mergeBranch = (branch: string, msg: string, when: string) =>
    g(['merge', '-q', '--no-ff', '-m', msg, branch], when);

  g(['init', '-q', '-b', 'main']);
  commit('init', '2026-05-01T09:00:00Z');
  g(['checkout', '-q', '-b', 'develop']);

  // Team-performance deliveries (feature merges carry both the issue key and
  // the feature/ prefix, so they double as DORA feature merges).
  const features: Array<[string, string, string]> = [
    ['TP-1', '2026-06-02T12:00:00Z', '2026-06-05T11:00:00Z'],
    ['TP-2', '2026-06-09T10:00:00Z', '2026-06-10T16:00:00Z'],
    ['TP-3', '2026-06-11T10:00:00Z', '2026-06-16T11:00:00Z'],
    ['TP-4', '2026-06-05T10:00:00Z', '2026-06-10T10:00:00Z'],
    ['TP-6', '2026-06-18T10:00:00Z', '2026-06-19T10:00:00Z'],
  ];
  for (const [key, dev, merge] of features) {
    g(['checkout', '-q', '-b', `feature/${key}-work`]);
    commit(`${key}: implement`, dev);
    g(['checkout', '-q', 'develop']);
    mergeBranch(`feature/${key}-work`, `Merge branch 'feature/${key}-work' into develop`, merge);
  }

  // DORA: releases + deploy tags + a hotfix (deploy on Jun 12 fails, recovers Jun 17).
  const release = (name: string, work: string, merge: string) => {
    g(['checkout', '-q', '-b', `release/${name}`]);
    commit(`release ${name} prep`, work);
    g(['checkout', '-q', 'develop']);
    mergeBranch(`release/${name}`, `Merge branch 'release/${name}' into develop`, merge);
    g(['tag', `v${name}-deployed`]);
  };
  release('1.0', '2026-06-11T09:00:00Z', '2026-06-12T09:00:00Z');
  g(['checkout', '-q', '-b', 'hotfix/crash']);
  commit('hotfix: crash on save', '2026-06-15T09:00:00Z');
  g(['checkout', '-q', 'develop']);
  mergeBranch('hotfix/crash', "Merge branch 'hotfix/crash' into develop", '2026-06-15T10:00:00Z');
  release('1.1', '2026-06-16T16:00:00Z', '2026-06-17T09:00:00Z');
  release('1.2', '2026-06-23T09:00:00Z', '2026-06-24T09:00:00Z');

  // In-flight branch (TP-7): first commit signal only, never merged.
  g(['checkout', '-q', '-b', 'feature/TP-7-wip']);
  commit('TP-7: wip', '2026-06-24T15:00:00Z');
  g(['checkout', '-q', 'develop']);

  return dir;
}

// ---------------------------------------------------------------------------
// Install / enable helpers (root API)
// ---------------------------------------------------------------------------

export async function installPlugin(ctx: APIRequestContext, base: string, source: string): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/plugin-admin/install`, { data: { source } });
  if (!r.ok()) throw new Error(`install ${source} → ${r.status()} ${await r.text()}`);
}

export async function enablePlugin(ctx: APIRequestContext, base: string, slug: string): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/plugin-admin/${slug}/enable`, { data: {} });
  if (!r.ok()) throw new Error(`enable ${slug} → ${r.status()} ${await r.text()}`);
}

/** Poll the proxied sidecar health until it answers 200 (compile-tolerant). */
export async function waitPluginHealthy(
  ctx: APIRequestContext,
  base: string,
  slug: string,
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const r = await ctx.get(`${base}/api/v1/plugins/${slug}/health`, { timeout: 2_000 });
      if (r.ok()) return;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) throw new Error(`plugin ${slug} never became healthy`);
    await new Promise((r) => setTimeout(r, 500));
  }
}
