// S13-307: REST DTOs had no parity guard at all — `otto-core` (the source of
// truth) and their hand-mirrored interfaces in ui/src/lib/api/types.ts only
// agreed because someone checked by hand. This reads both from source for the
// high-traffic DTOs (boot, identity, the session/workspace lists, notices,
// repos) and fails on drift:
//   • every serialized Rust field exists in the TS interface, and every TS key
//     is sent by Rust (or is a documented transient/extension field);
//   • a field the daemon OMITS when empty (`skip_serializing_if`) is optional;
//   • an `Option<T>` the daemon always sends (no skip) admits `null` in TS.
// A struct using `#[serde(flatten)]` / renames is not compared field-by-field
// — the test fails loudly rather than guess.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const root = new URL('../../', import.meta.url);
const stripRs = (s: string) => s.split('\n').map((l) => l.replace(/\/\/.*$/, '')).join('\n');
const rust = stripRs(
  ['crates/otto-core/src/api.rs', 'crates/otto-core/src/domain.rs']
    .map((p) => readFileSync(new URL(p, root), 'utf8'))
    .join('\n'),
);
const ts = readFileSync(new URL('ui/src/lib/api/types.ts', root), 'utf8')
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/(^|\s)\/\/.*$/gm, '$1');

function closing(src: string, open: number): number {
  const o = src[open];
  const c = o === '{' ? '}' : o === '(' ? ')' : ']';
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === o) depth++;
    else if (src[i] === c && --depth === 0) return i;
  }
  throw new Error(`unbalanced ${o} at ${open}`);
}

/** Split at top-level `sep`, tracking (), [], {} and — for Rust types — <>. */
function splitTop(src: string, sep: string, angles = false): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = '';
  for (const ch of src) {
    if ('([{'.includes(ch) || (angles && ch === '<')) depth++;
    else if (')]}'.includes(ch) || (angles && ch === '>')) depth--;
    if (ch === sep && depth === 0) {
      out.push(cur);
      cur = '';
    } else cur += ch;
  }
  out.push(cur);
  return out.map((s) => s.trim()).filter(Boolean);
}

interface Field { name: string; ty: string; skipIfEmpty: boolean }

function rustStruct(name: string): Field[] {
  const m = new RegExp(`pub struct ${name}\\s*\\{`).exec(rust);
  assert.ok(m, `pub struct ${name} not found in otto-core`);
  const attrs = rust.slice(Math.max(0, m.index - 300), m.index);
  assert.ok(!/rename_all\s*=\s*"(?!snake_case)/.test(attrs.slice(attrs.lastIndexOf('pub struct') + 1)), `${name}: container rename_all — compare by hand`);
  const open = m.index + m[0].length - 1;
  const fields: Field[] = [];
  for (const part of splitTop(rust.slice(open + 1, closing(rust, open)), ',', true)) {
    let p = part;
    let skipIfEmpty = false;
    let skip = false;
    for (;;) {
      const a = /^#\[([^\]]*)\]\s*/.exec(p);
      if (!a) break;
      if (/skip_serializing_if/.test(a[1])) skipIfEmpty = true;
      else if (/\bskip(_serializing)?\b/.test(a[1])) skip = true;
      assert.ok(!/flatten|rename\s*=/.test(a[1]), `${name}: #[${a[1]}] — compare by hand`);
      p = p.slice(a[0].length);
    }
    const f = /^pub(?:\([^)]*\))?\s+([a-z_][a-z0-9_]*)\s*:\s*([\s\S]+)$/.exec(p);
    if (f && !skip) fields.push({ name: f[1].replace(/^r#/, ''), ty: f[2].trim(), skipIfEmpty });
  }
  return fields;
}

function tsInterface(name: string): Map<string, { optional: boolean; ty: string }> {
  const m = new RegExp(`export interface ${name}\\s*(?:extends[^{]*)?\\{`).exec(ts);
  assert.ok(m, `interface ${name} not found in types.ts`);
  assert.ok(!/extends/.test(m[0]), `${name}: extends — compare by hand`);
  const open = m.index + m[0].length - 1;
  const keys = new Map<string, { optional: boolean; ty: string }>();
  for (const part of splitTop(ts.slice(open + 1, closing(ts, open)).replace(/\n/g, ';'), ';')) {
    const k = /^(?:readonly\s+)?([a-z_][a-z0-9_]*)(\?)?\s*:\s*([\s\S]*)$/.exec(part.trim());
    if (k) keys.set(k[1], { optional: !!k[2], ty: k[3].trim() });
  }
  return keys;
}

/** Rust struct → its TS mirror, plus TS-only keys the daemon adds outside the
 *  struct (documented in api.md) that the mirror may carry. */
const DTOS: { rust: string; ts?: string; extraTs?: string[] }[] = [
  { rust: 'MetaResp' },
  { rust: 'ToolStatus' },
  { rust: 'MeResp' },
  { rust: 'CapabilitiesResp' },
  { rust: 'NotificationSettings' },
  { rust: 'User' },
  { rust: 'Workspace' },
  // `live` / `viewers` are transient columns the list/get routes add.
  { rust: 'Session', extraTs: ['live', 'viewers'] },
  { rust: 'Notice' },
  { rust: 'Repo', extraTs: ['forge'] },
];

test('REST DTOs: otto-core structs and their types.ts mirrors agree', () => {
  const problems: string[] = [];
  let compared = 0;
  for (const d of DTOS) {
    const fields = rustStruct(d.rust);
    const keys = tsInterface(d.ts ?? d.rust);
    assert.ok(fields.length > 0, `${d.rust}: no fields parsed`);
    const names = new Set(fields.map((f) => f.name));
    for (const f of fields) {
      compared++;
      const k = keys.get(f.name);
      if (!k) problems.push(`${d.rust}.${f.name}: in Rust, missing in TS`);
      else if (f.skipIfEmpty && !k.optional) problems.push(`${d.rust}.${f.name}: omitted when empty on the wire but required in TS`);
      else if (/^Option\s*</.test(f.ty) && !f.skipIfEmpty && !/\bnull\b|\bunknown\b|\bany\b/.test(k.ty))
        problems.push(`${d.rust}.${f.name}: Option<…> is sent as null but the TS type (${k.ty}) has no null`);
    }
    for (const key of keys.keys()) {
      if (!names.has(key) && !(d.extraTs ?? []).includes(key)) problems.push(`${d.rust}.${key}: in TS, not sent by Rust`);
    }
  }
  assert.deepEqual(problems, []);
  assert.ok(compared >= 50, `compared ${compared} fields (scanner sanity)`);
});
