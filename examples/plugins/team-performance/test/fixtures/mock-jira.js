// Mock Jira Cloud REST v3 server (node:http, zero deps) with a deterministic
// project dataset and per-endpoint hit counters, for the sidecar E2E test.
// Faithful to the real API where it matters: /search/jql paginates by
// nextPageToken/isLast and returns NO total; fields are explicit.
'use strict';

const http = require('node:http');

// ---- canned dataset (June 2026; Jun 1 is a Monday) --------------------------

function h(created, from, to, field = 'status') {
  return { created, items: [{ field, fromString: from, toString: to }] };
}

function issue({ key, type, points, assignee, created, resolved, updated, histories, status, category }) {
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

const ALICE = { accountId: 'u-alice', displayName: 'Alice' };
const BOB = { accountId: 'u-bob', displayName: 'Bob' };

function dataset() {
  return [
    issue({
      key: 'TP-1', type: 'Story', points: 3, assignee: ALICE, status: 'Done', category: 'done',
      created: '2026-06-01T09:00:00Z', resolved: '2026-06-05T10:00:00Z', updated: '2026-06-05T10:00:00Z',
      histories: [
        h('2026-06-01T10:00:00Z', 'To Do', 'In Design'),
        h('2026-06-02T10:00:00Z', 'In Design', 'In Progress'),
        h('2026-06-04T10:00:00Z', 'In Progress', 'In Review'),
        h('2026-06-05T10:00:00Z', 'In Review', 'Done'),
      ],
    }),
    issue({
      key: 'TP-2', type: 'Story', points: 3, assignee: ALICE, status: 'Done', category: 'done',
      created: '2026-06-08T09:00:00Z', resolved: '2026-06-10T15:30:00Z', updated: '2026-06-10T15:30:00Z',
      histories: [
        h('2026-06-08T09:30:00Z', 'To Do', 'In Design'),
        h('2026-06-08T15:30:00Z', 'In Design', 'In Progress'),
        h('2026-06-10T15:30:00Z', 'In Progress', 'Done'),
      ],
    }),
    issue({
      key: 'TP-3', type: 'Story', points: 5, assignee: ALICE, status: 'Done', category: 'done',
      created: '2026-06-08T09:00:00Z', resolved: '2026-06-16T10:00:00Z', updated: '2026-06-16T10:00:00Z',
      histories: [
        h('2026-06-09T10:00:00Z', 'To Do', 'In Design'),
        h('2026-06-10T10:00:00Z', 'In Design', 'In Progress'),
        h('2026-06-16T10:00:00Z', 'In Progress', 'Done'),
      ],
    }),
    issue({
      key: 'TP-4', type: 'Story', points: 3, assignee: BOB, status: 'Done', category: 'done',
      created: '2026-06-01T09:00:00Z', resolved: '2026-06-10T09:00:00Z', updated: '2026-06-10T09:00:00Z',
      histories: [
        h('2026-06-02T09:00:00Z', 'To Do', 'In Design'),
        h('2026-06-04T09:00:00Z', 'In Design', 'In Progress'),
        h('2026-06-10T09:00:00Z', 'In Progress', 'Done'),
      ],
    }),
    issue({
      key: 'TP-5', type: 'Task', points: null, assignee: BOB, status: 'Done', category: 'done',
      created: '2026-06-15T09:00:00Z', resolved: '2026-06-17T10:00:00Z', updated: '2026-06-17T10:00:00Z',
      histories: [
        h('2026-06-15T10:00:00Z', 'To Do', 'In Progress'),
        h('2026-06-17T10:00:00Z', 'In Progress', 'Done'),
      ],
    }),
    issue({
      key: 'TP-6', type: 'Story', points: 3, assignee: BOB, status: 'Done', category: 'done',
      created: '2026-06-15T09:00:00Z', resolved: '2026-06-19T09:00:00Z', updated: '2026-06-19T09:00:00Z',
      histories: [
        h('2026-06-16T09:00:00Z', 'To Do', 'In Design'),
        h('2026-06-17T09:00:00Z', 'In Design', 'In Progress'),
        h('2026-06-19T09:00:00Z', 'In Progress', 'Done'),
      ],
    }),
    issue({
      key: 'TP-7', type: 'Story', points: 3, assignee: ALICE, status: 'In Progress', category: 'indeterminate',
      created: '2026-06-20T09:00:00Z', resolved: null, updated: '2026-06-24T10:00:00Z',
      histories: [
        h('2026-06-22T10:00:00Z', 'To Do', 'In Design'),
        h('2026-06-24T10:00:00Z', 'In Design', 'In Progress'),
      ],
    }),
    issue({
      key: 'TP-8', type: 'Task', points: null, assignee: BOB, status: 'In Progress', category: 'indeterminate',
      created: '2026-06-26T09:00:00Z', resolved: null, updated: '2026-06-29T09:00:00Z',
      histories: [h('2026-06-29T09:00:00Z', 'To Do', 'In Progress')],
    }),
  ];
}

const FIELDS = [
  { id: 'summary', name: 'Summary' },
  { id: 'customfield_10016', name: 'Story point estimate' },
  { id: 'customfield_10999', name: 'Sprint' },
];

const STATUSES = [
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
];

// ---- server -----------------------------------------------------------------

/** Start the mock; resolves {port, hits, issues, touch(), delayMs, close()}. */
function startMockJira() {
  const issues = dataset();
  const hits = { search: 0, issue: new Map(), fields: 0, statuses: 0, projects: 0, count: 0 };
  const state = { delayMs: 0 };

  function jqlFilter(jql) {
    // Naive `updated >= "yyyy-MM-dd HH:mm"` filter (mock interprets it as UTC).
    const m = /updated >= "([0-9-]+ [0-9:]+)"/.exec(jql || '');
    if (!m) return issues;
    const cutoff = Date.parse(m[1].replace(' ', 'T') + ':00Z');
    return issues.filter((i) => Date.parse(i.fields.updated) >= cutoff);
  }

  const server = http.createServer(async (req, res) => {
    if (state.delayMs) await new Promise((r) => setTimeout(r, state.delayMs));
    const u = new URL(req.url, 'http://localhost');
    const send = (code, obj) => {
      res.writeHead(code, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify(obj));
    };

    if (u.pathname === '/rest/api/3/field') {
      hits.fields++;
      return send(200, FIELDS);
    }
    if (u.pathname === '/rest/api/3/search/approximate-count') {
      hits.count++;
      let body = '';
      req.on('data', (c) => (body += c));
      req.on('end', () => {
        const jql = (() => {
          try {
            return JSON.parse(body).jql;
          } catch {
            return '';
          }
        })();
        send(200, { count: jqlFilter(jql).length });
      });
      return undefined;
    }
    if (u.pathname === '/rest/api/3/search/jql') {
      hits.search++;
      const matched = jqlFilter(u.searchParams.get('jql'));
      // Two-page pagination to exercise nextPageToken (no `total` on purpose).
      const pageSize = 5;
      const token = u.searchParams.get('nextPageToken');
      const start = token ? parseInt(token, 10) : 0;
      const slice = matched.slice(start, start + pageSize);
      const out = {
        issues: slice.map((i) => ({ key: i.key, fields: { updated: i.fields.updated, summary: i.fields.summary } })),
        isLast: start + pageSize >= matched.length,
      };
      if (!out.isLast) out.nextPageToken = String(start + pageSize);
      return send(200, out);
    }
    const issueMatch = /^\/rest\/api\/3\/issue\/([A-Z0-9-]+)$/.exec(u.pathname);
    if (issueMatch) {
      const key = issueMatch[1];
      hits.issue.set(key, (hits.issue.get(key) || 0) + 1);
      const found = issues.find((i) => i.key === key);
      return found ? send(200, found) : send(404, { errorMessages: ['no issue'] });
    }
    if (/^\/rest\/api\/3\/project\/[^/]+\/statuses$/.test(u.pathname)) {
      hits.statuses++;
      return send(200, STATUSES);
    }
    if (u.pathname === '/rest/api/3/project/search') {
      hits.projects++;
      return send(200, { values: [{ key: 'TP', name: 'Team Performance Fixture' }] });
    }
    return send(404, { errorMessages: ['not found'] });
  });

  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({
        port: server.address().port,
        hits,
        issues,
        /** Mark an issue as updated at `iso` (drives the incremental-scan test). */
        touch(key, iso) {
          const found = issues.find((i) => i.key === key);
          if (found) found.fields.updated = iso;
        },
        set delayMs(v) {
          state.delayMs = v;
        },
        close: () => new Promise((r) => server.close(r)),
      });
    });
  });
}

module.exports = { startMockJira, dataset };
