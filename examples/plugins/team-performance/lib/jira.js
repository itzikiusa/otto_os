// Minimal Jira Cloud REST v3 client on node builtins — no dependencies.
// All calls are serial with retry/backoff (429/5xx: honor Retry-After, else
// 250ms×2ⁿ, 3 attempts); a shared counter reports retries for scan status.
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

/**
 * makeClient({base_url, email, token}) → Jira client.
 * Every method throws Error('<status> from <path>') after retries fail.
 */
function makeClient(creds) {
  const base = String(creds.base_url || '').replace(/\/$/, '');
  const auth = 'Basic ' + Buffer.from(`${creds.email}:${creds.token}`).toString('base64');
  const state = { retries: 0 };

  async function call(pathname, opts = {}) {
    let lastErr = null;
    for (let attempt = 0; attempt < 3; attempt++) {
      let res;
      try {
        res = await request(base + pathname, { ...opts, headers: { Authorization: auth, ...(opts.headers || {}) } });
      } catch (e) {
        lastErr = e;
        state.retries++;
        await sleep(250 * 2 ** attempt);
        continue;
      }
      if (res.status === 429 || res.status >= 500) {
        state.retries++;
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

    /** Paginated enhanced-JQL search. onPage(issues) per page; capped at maxIssues. */
    async searchAll(jql, fields, { maxIssues = 1000, onPage } = {}) {
      const out = [];
      let token = null;
      while (out.length < maxIssues) {
        const params = new URLSearchParams({
          jql,
          maxResults: '100',
          fields: fields.join(','),
        });
        if (token) params.set('nextPageToken', token);
        const page = await call(`/rest/api/3/search/jql?${params}`);
        const issues = (page && page.issues) || [];
        out.push(...issues.slice(0, maxIssues - out.length));
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

module.exports = { makeClient, detectPointsField };
