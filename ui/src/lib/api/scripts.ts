// Pre-request / post-response script runtime. Runs the user's JS with a
// Postman-compatible-ish `pm` object so requests can be mutated, variables set
// (request chaining), and tests asserted. Scripts are the user's own code run
// against their own endpoints, so this is a convenience runtime, not a security
// sandbox.

import type { ApiKeyVal } from './types';

export interface PreRequestReq {
  method: string;
  url: string;
  headers: ApiKeyVal[];
  body: string;
}

export interface ResponseCtx {
  code: number;
  status: string;
  responseTime: number;
  headers: Record<string, string>;
  bodyText: string;
}

export interface TestResult {
  name: string;
  passed: boolean;
  error?: string;
}

export interface ScriptRun {
  logs: string[];
  error?: string;
  tests: TestResult[];
}

function expect(actual: unknown) {
  const assert = (cond: boolean, msg: string): void => {
    if (!cond) throw new Error(msg);
  };
  const eq = (e: unknown) => assert(JSON.stringify(actual) === JSON.stringify(e), `expected ${JSON.stringify(actual)} to equal ${JSON.stringify(e)}`);
  const api = {
    toBe: (e: unknown) => assert(actual === e, `expected ${JSON.stringify(actual)} to be ${JSON.stringify(e)}`),
    toEqual: eq,
    eql: eq,
    toContain: (e: unknown) => assert(
      (typeof actual === 'string' && actual.includes(String(e))) || (Array.isArray(actual) && actual.includes(e)),
      `expected ${JSON.stringify(actual)} to contain ${JSON.stringify(e)}`,
    ),
    toBeTruthy: () => assert(!!actual, `expected ${JSON.stringify(actual)} to be truthy`),
    toBeFalsy: () => assert(!actual, `expected ${JSON.stringify(actual)} to be falsy`),
    above: (n: number) => assert(Number(actual) > n, `expected ${actual} to be above ${n}`),
    below: (n: number) => assert(Number(actual) < n, `expected ${actual} to be below ${n}`),
  };
  return api;
}

function varApi(vars: Record<string, string>) {
  return {
    get: (k: string): string | undefined => vars[k],
    set: (k: string, v: unknown): void => { vars[k] = typeof v === 'string' ? v : JSON.stringify(v); },
    unset: (k: string): void => { delete vars[k]; },
    has: (k: string): boolean => k in vars,
    toObject: (): Record<string, string> => ({ ...vars }),
  };
}

function run(code: string, pm: unknown): ScriptRun {
  const logs: string[] = [];
  const consoleProxy = {
    log: (...a: unknown[]) => logs.push(a.map((x) => (typeof x === 'string' ? x : JSON.stringify(x))).join(' ')),
    error: (...a: unknown[]) => logs.push('ERROR: ' + a.map(String).join(' ')),
    warn: (...a: unknown[]) => logs.push('WARN: ' + a.map(String).join(' ')),
    info: (...a: unknown[]) => logs.push(a.map(String).join(' ')),
  };
  const tests: TestResult[] = [];
  (pm as { __tests: TestResult[] }).__tests = tests;
  try {
    // eslint-disable-next-line no-new-func
    const fn = new Function('pm', 'console', code);
    fn(pm, consoleProxy);
    return { logs, tests };
  } catch (e) {
    return { logs, tests, error: e instanceof Error ? `${e.name}: ${e.message}` : String(e) };
  }
}

/** Run a pre-request script. Mutates `req` (headers/url/body) and `vars` in place. */
export function runPreRequest(code: string, req: PreRequestReq, vars: Record<string, string>): ScriptRun {
  if (!code.trim()) return { logs: [], tests: [] };
  const headers = {
    add: (h: { key: string; value: string }) => req.headers.push({ key: h.key, value: h.value, enabled: true }),
    upsert: (h: { key: string; value: string }) => {
      const i = req.headers.findIndex((x) => x.key.toLowerCase() === h.key.toLowerCase());
      if (i >= 0) req.headers[i] = { ...req.headers[i], value: h.value };
      else req.headers.push({ key: h.key, value: h.value, enabled: true });
    },
    remove: (k: string) => { req.headers = req.headers.filter((x) => x.key.toLowerCase() !== k.toLowerCase()); },
    get: (k: string) => req.headers.find((x) => x.key.toLowerCase() === k.toLowerCase())?.value,
  };
  const v = varApi(vars);
  const pm = {
    environment: v,
    variables: v,
    globals: v,
    expect,
    request: {
      get method() { return req.method; },
      set method(m: string) { req.method = m; },
      get url() { return req.url; },
      set url(u: string) { req.url = u; },
      get body() { return req.body; },
      set body(b: string) { req.body = b; },
      headers,
      addHeader: headers.add,
    },
  };
  return run(code, pm);
}

/** Run a post-response (test) script. Reads the response, may set `vars`. */
export function runPostResponse(code: string, resp: ResponseCtx, vars: Record<string, string>): ScriptRun {
  if (!code.trim()) return { logs: [], tests: [] };
  const v = varApi(vars);
  const response = {
    code: resp.code,
    status: resp.status,
    responseTime: resp.responseTime,
    headers: resp.headers,
    text: () => resp.bodyText,
    json: () => JSON.parse(resp.bodyText),
  };
  const pm: { __tests: TestResult[]; [k: string]: unknown } = {
    __tests: [],
    environment: v,
    variables: v,
    globals: v,
    expect,
    response,
    test: (name: string, fn: () => void) => {
      try {
        fn();
        pm.__tests.push({ name, passed: true });
      } catch (e) {
        pm.__tests.push({ name, passed: false, error: e instanceof Error ? e.message : String(e) });
      }
    },
  };
  return run(code, pm);
}
