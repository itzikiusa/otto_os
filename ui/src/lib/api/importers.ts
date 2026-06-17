// Import API collections from Postman (v2.1), OpenAPI 3, and HAR into Otto's
// collection/request model. All parsing is client-side.

import type { ApiAuth, ApiBodyMode, ApiKeyVal, ApiCollection, ApiRequest } from './types';

export interface ImportedRequest {
  folderPath: string[];
  name: string;
  method: string;
  url: string;
  headers: ApiKeyVal[];
  query: ApiKeyVal[];
  body_mode: ApiBodyMode;
  body: string;
  auth: ApiAuth;
}

export interface ImportedCollection {
  name: string;
  format: 'postman' | 'openapi' | 'har';
  requests: ImportedRequest[];
}

const kv = (key: string, value: string): ApiKeyVal => ({ key, value, enabled: true });

export function detectAndParse(text: string, filename: string): ImportedCollection {
  let json: Record<string, unknown>;
  try {
    json = JSON.parse(text);
  } catch {
    throw new Error('File is not valid JSON');
  }
  if (json.info && Array.isArray(json.item)) return parsePostman(json);
  if (json.openapi || json.swagger) return parseOpenApi(json);
  if (json.log && (json.log as { entries?: unknown }).entries) return parseHar(json, filename);
  throw new Error('Unrecognized format — expected Postman, OpenAPI, or HAR');
}

// ── Postman v2.1 ────────────────────────────────────────────────────────────

function parsePostman(doc: Record<string, unknown>): ImportedCollection {
  const info = (doc.info ?? {}) as { name?: string };
  const requests: ImportedRequest[] = [];
  const walk = (items: unknown[], path: string[]): void => {
    for (const raw of items) {
      const item = raw as Record<string, unknown>;
      if (Array.isArray(item.item)) {
        walk(item.item, [...path, String(item.name ?? 'Folder')]);
      } else if (item.request) {
        requests.push(pmRequest(item, path));
      }
    }
  };
  walk(doc.item as unknown[], []);
  return { name: info.name ?? 'Imported Collection', format: 'postman', requests };
}

function pmRequest(item: Record<string, unknown>, path: string[]): ImportedRequest {
  const req = item.request as Record<string, unknown>;
  const method = String(req.method ?? 'GET').toUpperCase();
  const urlRaw = req.url;
  let url = '';
  const query: ApiKeyVal[] = [];
  if (typeof urlRaw === 'string') url = urlRaw;
  else if (urlRaw && typeof urlRaw === 'object') {
    const u = urlRaw as { raw?: string; query?: { key?: string; value?: string; disabled?: boolean }[] };
    url = u.raw ?? '';
    for (const q of u.query ?? []) query.push({ key: q.key ?? '', value: q.value ?? '', enabled: !q.disabled });
  }
  // strip query string off raw url (Postman duplicates it in .query)
  if (query.length && url.includes('?')) url = url.split('?')[0];

  const headers: ApiKeyVal[] = [];
  for (const h of (req.header as { key?: string; value?: string; disabled?: boolean }[]) ?? []) {
    headers.push({ key: h.key ?? '', value: h.value ?? '', enabled: !h.disabled });
  }

  let body_mode: ApiBodyMode = 'none';
  let body = '';
  const b = req.body as Record<string, unknown> | undefined;
  if (b) {
    if (b.mode === 'raw') {
      body = String(b.raw ?? '');
      const lang = ((b.options as { raw?: { language?: string } })?.raw?.language ?? '').toLowerCase();
      body_mode = lang === 'json' || body.trim().startsWith('{') || body.trim().startsWith('[') ? 'json' : 'raw';
    } else if (b.mode === 'urlencoded') {
      body_mode = 'form';
      body = ((b.urlencoded as { key?: string; value?: string }[]) ?? [])
        .map((p) => `${encodeURIComponent(p.key ?? '')}=${encodeURIComponent(p.value ?? '')}`).join('&');
    } else if (b.mode === 'formdata') {
      body_mode = 'multipart';
      body = JSON.stringify(((b.formdata as { key?: string; value?: string; type?: string }[]) ?? [])
        .map((p) => ({ key: p.key ?? '', type: p.type === 'file' ? 'file' : 'text', value: p.value ?? '' })));
    } else if (b.mode === 'graphql') {
      body_mode = 'graphql';
      body = String((b.graphql as { query?: string })?.query ?? '');
    }
  }

  return { folderPath: path, name: String(item.name ?? `${method} ${url}`), method, url, headers, query, body_mode, body, auth: pmAuth(req.auth) };
}

function pmAuth(auth: unknown): ApiAuth {
  const a = auth as { type?: string; bearer?: { value?: string }[]; basic?: { key?: string; value?: string }[] } | undefined;
  if (!a?.type) return { type: 'none' };
  if (a.type === 'bearer') {
    const token = (a.bearer ?? []).find((x) => x.value)?.value ?? '';
    return { type: 'bearer', token: String(token) };
  }
  if (a.type === 'basic') {
    const get = (k: string) => String((a.basic ?? []).find((x) => x.key === k)?.value ?? '');
    return { type: 'basic', username: get('username'), password: get('password') };
  }
  return { type: 'none' };
}

// ── OpenAPI 3 ───────────────────────────────────────────────────────────────

function parseOpenApi(doc: Record<string, unknown>): ImportedCollection {
  const info = (doc.info ?? {}) as { title?: string };
  const servers = (doc.servers as { url?: string }[]) ?? [];
  const base = servers[0]?.url ?? '';
  const paths = (doc.paths ?? {}) as Record<string, Record<string, unknown>>;
  const requests: ImportedRequest[] = [];
  const METHODS = ['get', 'post', 'put', 'patch', 'delete', 'head', 'options'];
  for (const [path, ops] of Object.entries(paths)) {
    for (const m of METHODS) {
      const op = ops[m] as Record<string, unknown> | undefined;
      if (!op) continue;
      const headers: ApiKeyVal[] = [];
      const query: ApiKeyVal[] = [];
      for (const param of (op.parameters as { in?: string; name?: string }[]) ?? []) {
        if (param.in === 'header') headers.push(kv(param.name ?? '', ''));
        else if (param.in === 'query') query.push(kv(param.name ?? '', ''));
      }
      let body_mode: ApiBodyMode = 'none';
      let body = '';
      const rb = (op.requestBody as { content?: Record<string, { example?: unknown; schema?: unknown }> })?.content;
      if (rb && rb['application/json']) {
        body_mode = 'json';
        const ex = rb['application/json'].example;
        body = ex !== undefined ? JSON.stringify(ex, null, 2) : '{\n  \n}';
        headers.push(kv('Content-Type', 'application/json'));
      }
      const tags = (op.tags as string[]) ?? [];
      requests.push({
        folderPath: tags.length ? [tags[0]] : [],
        name: String(op.summary ?? op.operationId ?? `${m.toUpperCase()} ${path}`),
        method: m.toUpperCase(),
        url: base + path,
        headers, query, body_mode, body, auth: { type: 'none' },
      });
    }
  }
  return { name: info.title ?? 'OpenAPI Import', format: 'openapi', requests };
}

// ── HAR ─────────────────────────────────────────────────────────────────────

function parseHar(doc: Record<string, unknown>, filename: string): ImportedCollection {
  const entries = ((doc.log as { entries?: unknown[] }).entries ?? []) as Record<string, unknown>[];
  const requests: ImportedRequest[] = [];
  for (const e of entries) {
    const req = e.request as Record<string, unknown>;
    if (!req) continue;
    const url = String(req.url ?? '');
    const headers: ApiKeyVal[] = ((req.headers as { name?: string; value?: string }[]) ?? [])
      .filter((h) => !(h.name ?? '').startsWith(':'))
      .map((h) => kv(h.name ?? '', h.value ?? ''));
    const query: ApiKeyVal[] = ((req.queryString as { name?: string; value?: string }[]) ?? []).map((q) => kv(q.name ?? '', q.value ?? ''));
    const post = req.postData as { text?: string; mimeType?: string } | undefined;
    let body_mode: ApiBodyMode = 'none';
    let body = '';
    if (post?.text) {
      body = post.text;
      body_mode = (post.mimeType ?? '').includes('json') ? 'json' : (post.mimeType ?? '').includes('urlencoded') ? 'form' : 'raw';
    }
    let name = url;
    try { name = `${req.method} ${new URL(url).pathname}`; } catch { /* keep */ }
    requests.push({ folderPath: [], name: String(name), method: String(req.method ?? 'GET').toUpperCase(), url: url.split('?')[0], headers, query, body_mode, body, auth: { type: 'none' } });
  }
  return { name: filename.replace(/\.har$/i, '') || 'HAR Import', format: 'har', requests };
}

// ── Export → Postman v2.1 ─────────────────────────────────────────────────────

/** Build a Postman v2.1 collection document for one root collection. */
export function collectionToPostman(
  rootId: string,
  collections: ApiCollection[],
  requests: ApiRequest[],
): Record<string, unknown> {
  const root = collections.find((c) => c.id === rootId);
  const buildItems = (parentId: string): unknown[] => {
    const folders = collections
      .filter((c) => (c.parent_id ?? null) === parentId)
      .map((c) => ({ name: c.name, item: buildItems(c.id) }));
    const reqs = requests
      .filter((r) => r.collection_id === parentId)
      .map((r) => pmItem(r));
    return [...folders, ...reqs];
  };
  return {
    info: {
      name: root?.name ?? 'Collection',
      schema: 'https://schema.getpostman.com/json/collection/v2.1.0/collection.json',
    },
    item: buildItems(rootId),
  };
}

function pmItem(r: ApiRequest): Record<string, unknown> {
  const header = (r.headers as ApiKeyVal[]).map((h) => ({ key: h.key, value: h.value, disabled: h.enabled === false }));
  const query = (r.query as ApiKeyVal[]).map((q) => ({ key: q.key, value: q.value, disabled: q.enabled === false }));
  const rawUrl = query.length ? `${r.url}?${query.map((q) => `${q.key}=${q.value}`).join('&')}` : r.url;
  const request: Record<string, unknown> = {
    method: r.method,
    header,
    url: { raw: rawUrl, query },
  };
  if (r.body_mode !== 'none' && r.body) {
    if (r.body_mode === 'json' || r.body_mode === 'raw') {
      request.body = { mode: 'raw', raw: r.body, options: { raw: { language: r.body_mode === 'json' ? 'json' : 'text' } } };
    } else if (r.body_mode === 'form') {
      request.body = { mode: 'urlencoded', urlencoded: r.body.split('&').filter(Boolean).map((p) => {
        const [k, v] = p.split('='); return { key: decodeURIComponent(k ?? ''), value: decodeURIComponent(v ?? '') };
      }) };
    } else if (r.body_mode === 'graphql') {
      request.body = { mode: 'graphql', graphql: { query: r.body } };
    }
  }
  const auth = r.auth as ApiAuth;
  if (auth.type === 'bearer') request.auth = { type: 'bearer', bearer: [{ key: 'token', value: auth.token }] };
  else if (auth.type === 'basic') request.auth = { type: 'basic', basic: [{ key: 'username', value: auth.username }, { key: 'password', value: auth.password }] };
  return { name: r.name, request };
}
