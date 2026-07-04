// Minimal Jira Cloud REST v3 client on node builtins — no dependencies.
//
// Pacing is a first-class concern: every call (after the first) waits
// `paceMs`, and any 429 doubles the effective pace for the rest of the client's
// life (capped) on top of honoring Retry-After — full-project scans must never
// trip Jira's rate limits. All calls are serial with retry/backoff
// (429/5xx: honor Retry-After, else 250ms×2ⁿ, 3 attempts); shared counters
// report retries + the current pace for scan status.
//
// Notable API facts this client encodes (Jira Cloud, 2025+):
//   * /rest/api/3/search/jql paginates by nextPageToken/isLast, returns NO
//     `total`, ignores `startAt`, and defaults to id-only fields — pass an
//     explicit fields list.
//   * totals come from POST /rest/api/3/search/approximate-count (nullable).
//   * an issue's embedded changelog is capped; page /issue/{key}/changelog
//     when total > histories.length.
'use strict';

const http = require('node:http');
const https = require('node:https');

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function request(urlStr, { method = 'GET', headers = {}, body = null } = {}) {
  return new Promise((resolve, reject) => {
    const u = new URL(urlStr);
    const mod = u.protocol === 'https:' ? https : http;
    const data = body ? JSON.stringify(body) : null;
    const req = mod.request(
      {
        method,
        hostname: u.hostname,
        port: u.port || (u.protocol === 'https:' ? 443 : 80),
        path: u.pathname + u.search,
        headers: {
          Accept: 'application/json',
          ...(data ? { 'Content-Type': 'application/json' } : {}),
          ...headers,
        },
      },
      (res) => {
        let buf = '';
        res.on('data', (c) => (buf += c));
        res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body: buf }));
      },
    );
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

const MAX_PACE = 4000;

/**
 * makeClient({base_url, email, token}, {paceMs}) → Jira client.
 * Every method throws Error('<status> from <path>') after retries fail.
 */
function makeClient(creds, opts = {}) {
  const base = String(creds.base_url || '').replace(/\/$/, '');
  const auth = 'Basic ' + Buffer.from(`${creds.email}:${creds.token}`).toString('base64');
  const state = { retries: 0, paceMs: Math.max(0, Number(opts.paceMs) || 0), calls: 0 };

  async function call(pathname, opts2 = {}) {
    let lastErr = null;
    for (let attempt = 0; attempt < 3; attempt++) {
      if (state.calls++ > 0 && state.paceMs > 0) await sleep(state.paceMs);
      let res;
      try {
        res = await request(base + pathname, { ...opts2, headers: { Authorization: auth, ...(opts2.headers || {}) } });
      } catch (e) {
        lastErr = e;
        state.retries++;
        await sleep(250 * 2 ** attempt);
        continue;
      }
      if (res.status === 429 || res.status >= 500) {
        state.retries++;
        // Getting throttled means our pace is too hot — back off permanently.
        if (res.status === 429) state.paceMs = Math.min(MAX_PACE, Math.max(500, state.paceMs * 2));
        const ra = parseFloat(res.headers['retry-after']);
        await sleep(Number.isFinite(ra) ? Math.min(ra * 1000, 30000) : 250 * 2 ** attempt);
        lastErr = new Error(`${res.status} from ${pathname}`);
        continue;
      }
      if (res.status >= 200 && res.status < 300) {
        try {
          return res.body ? JSON.parse(res.body) : null;
        } catch {
          throw new Error(`bad JSON from ${pathname}`);
        }
      }
      throw new Error(`${res.status} from ${pathname}: ${res.body.slice(0, 200)}`);
    }
    throw lastErr || new Error(`request failed: ${pathname}`);
  }

  return {
    get retries() {
      return state.retries;
    },
    get paceMs() {
      return state.paceMs;
    },

    /** Paginated enhanced-JQL search. onPage(count) per page; maxIssues 0 = unlimited. */
    async searchAll(jql, fields, { maxIssues = 0, onPage } = {}) {
      const cap = maxIssues > 0 ? maxIssues : Infinity;
      const out = [];
      let token = null;
      while (out.length < cap) {
        const params = new URLSearchParams({
          jql,
          maxResults: '100',
          fields: fields.join(','),
        });
        if (token) params.set('nextPageToken', token);
        const page = await call(`/rest/api/3/search/jql?${params}`);
        const issues = (page && page.issues) || [];
        out.push(...(cap === Infinity ? issues : issues.slice(0, cap - out.length)));
        if (onPage) onPage(out.length);
        token = page && page.nextPageToken;
        if (!token || page.isLast || issues.length === 0) break;
      }
      return out;
    },

    /** Full issue incl. complete changelog (pages the tail when capped). */
    async issueWithChangelog(key, fields) {
      const issue = await call(
        `/rest/api/3/issue/${encodeURIComponent(key)}?expand=changelog&fields=${encodeURIComponent(fields.join(','))}`,
      );
      const cl = issue.changelog || { histories: [] };
      if (typeof cl.total === 'number' && cl.total > (cl.histories || []).length) {
        const histories = [];
        let startAt = 0;
        for (;;) {
          const page = await call(`/rest/api/3/issue/${encodeURIComponent(key)}/changelog?startAt=${startAt}&maxResults=100`);
          histories.push(...((page && page.values) || []));
          startAt = (page.startAt || 0) + ((page.values && page.values.length) || 0);
          if (page.isLast || !page.values || page.values.length === 0) break;
        }
        issue.changelog = { histories };
      }
      return issue;
    },

    /** All fields (for story-points autodetect). */
    fields() {
      return call('/rest/api/3/field');
    },

    /** Project statuses, deduped by name (statusCategory kept when present). */
    async projectStatuses(projectKey) {
      const types = await call(`/rest/api/3/project/${encodeURIComponent(projectKey)}/statuses`);
      const byName = new Map();
      for (const t of types || []) {
        for (const s of t.statuses || []) {
          if (!byName.has(s.name)) {
            byName.set(s.name, {
              name: s.name,
              category: s.statusCategory ? s.statusCategory.key : null,
            });
          }
        }
      }
      return [...byName.values()];
    },

    /** Project list for the picker. */
    async searchProjects(query = '') {
      const page = await call(`/rest/api/3/project/search?maxResults=50&query=${encodeURIComponent(query)}`);
      return ((page && page.values) || []).map((p) => ({ key: p.key, name: p.name }));
    },

    /** Assignable users of a project (people picker before the first scan). */
    async assignableUsers(projectKey) {
      const out = [];
      let startAt = 0;
      for (;;) {
        const page = await call(
          `/rest/api/3/user/assignable/search?project=${encodeURIComponent(projectKey)}&startAt=${startAt}&maxResults=100`,
        );
        const users = Array.isArray(page) ? page : [];
        out.push(...users.filter((u) => u.accountType !== 'app').map((u) => ({ id: u.accountId, name: u.displayName })));
        if (users.length < 100) break;
        startAt += users.length;
      }
      return out;
    },

    /** Approximate issue count for a JQL (progress denominator; null on failure). */
    async approxCount(jql) {
      try {
        const r = await call('/rest/api/3/search/approximate-count', { method: 'POST', body: { jql } });
        return r && typeof r.count === 'number' ? r.count : null;
      } catch {
        return null;
      }
    },
  };
}

/** Find the story-points field id by conventional names (fallback 10016). */
function detectPointsField(fields) {
  const names = ['story points', 'story point estimate'];
  for (const f of fields || []) {
    if (f && f.name && names.includes(String(f.name).toLowerCase())) return f.id;
  }
  return 'customfield_10016';
}

/** Flatten an ADF document (Jira v3 rich text) to plain text. */
function adfToText(node) {
  if (node == null) return '';
  if (typeof node === 'string') return node;
  if (Array.isArray(node)) return node.map(adfToText).join('');
  let out = '';
  if (node.type === 'text') out += node.text || '';
  if (node.type === 'hardBreak') out += '\n';
  if (node.content) out += adfToText(node.content);
  if (['paragraph', 'heading', 'listItem', 'codeBlock', 'blockquote'].includes(node.type)) out += '\n';
  return out;
}

module.exports = { makeClient, detectPointsField, adfToText };
