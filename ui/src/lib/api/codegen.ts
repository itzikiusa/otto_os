// Code-snippet generation for the API client. Turns the current draft into a
// ready-to-run request in several languages. Variables ({{var}}) are left
// as-is (the user can paste real values).

import type { ApiDraft } from '../stores/apiClient.svelte';
import type { ApiKeyVal } from './types';

export type CodeLang = 'curl' | 'javascript' | 'typescript' | 'python' | 'go';

export const CODE_LANGS: { id: CodeLang; label: string }[] = [
  { id: 'curl', label: 'cURL' },
  { id: 'javascript', label: 'JavaScript (fetch)' },
  { id: 'typescript', label: 'TypeScript (fetch)' },
  { id: 'python', label: 'Python (requests)' },
  { id: 'go', label: 'Go (net/http)' },
];

interface Effective {
  method: string;
  url: string;
  headers: [string, string][];
  body: string | null;
}

function live(rows: ApiKeyVal[]): ApiKeyVal[] {
  return rows.filter((r) => r.enabled !== false && r.key.trim() !== '');
}

/** Resolve the draft into a concrete request (url+query, headers+auth, body). */
function effective(d: ApiDraft): Effective {
  let url = d.url;
  const qs = live(d.query)
    .map((q) => `${encodeURIComponent(q.key)}=${encodeURIComponent(q.value)}`)
    .join('&');
  if (qs) url += (url.includes('?') ? '&' : '?') + qs;

  const headers: [string, string][] = live(d.headers).map((h) => [h.key, h.value]);
  const hasCt = headers.some(([k]) => k.toLowerCase() === 'content-type');
  if (d.body_mode === 'json' && d.body.trim() && !hasCt) headers.push(['Content-Type', 'application/json']);
  if (d.body_mode === 'form' && d.body.trim() && !hasCt) headers.push(['Content-Type', 'application/x-www-form-urlencoded']);

  // Auth → header / query (api_key in query handled inline below)
  const a = d.auth;
  if (a.type === 'bearer' && a.token) headers.push(['Authorization', `Bearer ${a.token}`]);
  else if (a.type === 'basic') headers.push(['Authorization', `Basic <base64(${a.username}:${a.password})>`]);
  else if (a.type === 'api_key' && a.key) {
    if (a.in === 'header') headers.push([a.key, a.value]);
    else url += (url.includes('?') ? '&' : '?') + `${encodeURIComponent(a.key)}=${encodeURIComponent(a.value)}`;
  }
  else if (a.type === 'oauth2' && a.access_token) headers.push(['Authorization', `${a.token_type || 'Bearer'} ${a.access_token}`]);

  const body = d.body_mode !== 'none' && d.body.trim() ? d.body : null;
  return { method: d.method.toUpperCase(), url, headers, body };
}

export function generateCode(d: ApiDraft, lang: CodeLang): string {
  const e = effective(d);
  switch (lang) {
    case 'curl': return genCurl(e);
    case 'javascript': return genFetch(e, false);
    case 'typescript': return genFetch(e, true);
    case 'python': return genPython(e);
    case 'go': return genGo(e);
  }
}

function sh(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

function genCurl(e: Effective): string {
  const parts = [`curl -X ${e.method} ${sh(e.url)}`];
  for (const [k, v] of e.headers) parts.push(`  -H ${sh(`${k}: ${v}`)}`);
  if (e.body) parts.push(`  --data ${sh(e.body)}`);
  return parts.join(' \\\n');
}

function genFetch(e: Effective, ts: boolean): string {
  const headerObj = e.headers.length
    ? `{\n${e.headers.map(([k, v]) => `    ${JSON.stringify(k)}: ${JSON.stringify(v)},`).join('\n')}\n  }`
    : '{}';
  const init = [`  method: ${JSON.stringify(e.method)},`, `  headers: ${headerObj},`];
  if (e.body) init.push(`  body: ${JSON.stringify(e.body)},`);
  const typed = ts ? ': Response' : '';
  return `const res${typed} = await fetch(${JSON.stringify(e.url)}, {\n${init.join('\n')}\n});\nconst data = await res.json();\nconsole.log(data);`;
}

function genPython(e: Effective): string {
  const headers = `{\n${e.headers.map(([k, v]) => `    ${JSON.stringify(k)}: ${JSON.stringify(v)},`).join('\n')}\n}`;
  const lines = ['import requests', ''];
  lines.push(`url = ${JSON.stringify(e.url)}`);
  lines.push(`headers = ${e.headers.length ? headers : '{}'}`);
  if (e.body) lines.push(`payload = ${JSON.stringify(e.body)}`);
  const args = ['url', 'headers=headers'];
  if (e.body) args.push('data=payload');
  lines.push(`resp = requests.request(${JSON.stringify(e.method)}, ${args.join(', ')})`);
  lines.push('print(resp.status_code)');
  lines.push('print(resp.text)');
  return lines.join('\n');
}

function genGo(e: Effective): string {
  const lines = [
    'package main',
    '',
    'import (',
    '\t"fmt"',
    '\t"io"',
    e.body ? '\t"strings"' : '',
    '\t"net/http"',
    ')',
    '',
    'func main() {',
  ].filter(Boolean);
  const bodyArg = e.body ? `strings.NewReader(${JSON.stringify(e.body)})` : 'nil';
  lines.push(`\treq, _ := http.NewRequest(${JSON.stringify(e.method)}, ${JSON.stringify(e.url)}, ${bodyArg})`);
  for (const [k, v] of e.headers) lines.push(`\treq.Header.Set(${JSON.stringify(k)}, ${JSON.stringify(v)})`);
  lines.push('\tresp, err := http.DefaultClient.Do(req)');
  lines.push('\tif err != nil { panic(err) }');
  lines.push('\tdefer resp.Body.Close()');
  lines.push('\tbody, _ := io.ReadAll(resp.Body)');
  lines.push('\tfmt.Println(resp.Status)');
  lines.push('\tfmt.Println(string(body))');
  lines.push('}');
  return lines.join('\n');
}
