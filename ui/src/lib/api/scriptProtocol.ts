import type { PreRequestReq, ResponseCtx, ScriptRun } from './scripts';

export type ScriptInput = { code: string; vars: Record<string, string> } & (
  { kind: 'pre'; request: PreRequestReq } | { kind: 'post'; response: ResponseCtx }
);
export interface ScriptOutput { run: ScriptRun; vars: Record<string, string>; request?: PreRequestReq; }
export const SCRIPT_PAYLOAD_BYTES = 8 * 1024 * 1024;

/** Conservative retained-data accounting; bounded traversal, no giant JSON copy.
 * Runs before each structured-clone boundary. Not an absolute JS heap limit. */
export function assertScriptPayload(value: unknown): void {
  let bytes = 0, entries = 0;
  const active = new Set<object>();
  const visit = (v: unknown, depth: number): void => {
    if (++entries > 10000 || depth > 64) throw new Error('Script payload budget exceeded (too many nested values).');
    if (typeof v === 'string') bytes += v.length * 2;
    else if (v === null || v === undefined || typeof v === 'boolean' || typeof v === 'number') bytes += 8;
    else if (typeof v === 'object') {
      if (active.has(v)) throw new Error('Script payload cannot contain a cycle.');
      const keys = Object.keys(v);
      if (keys.length > 10000 - entries) throw new Error('Script payload budget exceeded (too many values).');
      active.add(v); bytes += 32;
      for (const key of keys) { bytes += key.length * 2; visit((v as Record<string, unknown>)[key], depth + 1); }
      active.delete(v);
    } else throw new Error('Script payload contains a value that cannot be cloned.');
    if (bytes > SCRIPT_PAYLOAD_BYTES) throw new Error('Script payload budget exceeded (8 MiB).');
  };
  visit(value, 0);
}
