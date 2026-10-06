// In-memory stand-in for the daemon's PR routes (fetch-compatible). Records
// every call; can answer the first request with a 429 to exercise the pacer.
'use strict';

function res(status, body, headers = {}) {
  return {
    status,
    headers: { get: (k) => headers[k.toLowerCase()] ?? null },
    json: async () => body,
    text: async () => JSON.stringify(body),
  };
}

function createMockDaemon({ repos = [], prs = {}, throttleFirst = false, retryAfter = '3' } = {}) {
  const calls = [];
  let throttled = !throttleFirst;
  const state = { prs }; // {[repoId]: [{summary, detail, commits, diff}]}
  async function fetch(url, init) {
    const u = new URL(url);
    const route = u.pathname.replace(/^\/api\/v1/, '');
    calls.push({ route, search: u.search, auth: init && init.headers && init.headers.authorization });
    if (!throttled) {
      throttled = true;
      return res(429, { title: 'rate limited' }, { 'retry-after': retryAfter });
    }
    if (route === '/repos') return res(200, repos);
    let m = route.match(/^\/repos\/([^/]+)\/prs$/);
    if (m) {
      const all = [...(state.prs[decodeURIComponent(m[1])] || [])].sort((a, b) =>
        b.summary.updated_at.localeCompare(a.summary.updated_at));
      const page = Number(u.searchParams.get('page')) || 1;
      const per = Number(u.searchParams.get('per_page')) || 50;
      const items = all.slice((page - 1) * per, page * per).map((x) => x.summary);
      return res(200, { items, has_more: page * per < all.length, page, per_page: per });
    }
    m = route.match(/^\/repos\/([^/]+)\/prs\/(\d+)(\/commits|\/diff)?$/);
    if (m) {
      const pr = (state.prs[decodeURIComponent(m[1])] || []).find((x) => x.summary.number === Number(m[2]));
      if (!pr) return res(404, { title: 'not found' });
      if (m[3] === '/commits') return res(200, pr.commits || []);
      if (m[3] === '/diff') return res(200, pr.diff || { files: [] });
      return res(200, { ...pr.summary, ...(pr.detail || {}) });
    }
    return res(404, { title: 'no route' });
  }
  return { fetch, calls, state };
}

module.exports = { createMockDaemon, res };
