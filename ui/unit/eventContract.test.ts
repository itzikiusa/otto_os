// S13-06: the `/ws/events` contract has three copies — the Rust `Event` enum
// (crates/otto-core/src/event.rs, the source of truth), the `OttoEvent` union
// in ui/src/lib/api/types.ts and docs/contracts/ws.md — and nothing generated
// or compared them, so a new variant or field drifted silently. This reads
// all three from source and fails on any mismatch:
//   • every Rust variant's wire tag exists in `OttoEvent` and in ws.md, and
//     every `OttoEvent` tag still exists in Rust (no stale UI members);
//   • `type_name()` (an exhaustive match the compiler keeps complete) agrees
//     with the serde snake_case tag of every variant;
//   • each variant's serialized field names equal the TS member's keys, and a
//     field the daemon OMITS when empty (`skip_serializing_if`) is optional
//     (`?:`) in TS — the classic "required in TS, absent on the wire" drift.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const root = new URL('../../', import.meta.url);
const rust = readFileSync(new URL('crates/otto-core/src/event.rs', root), 'utf8');
// Comments removed up front: their prose (parentheses, `;`, `|`) would
// otherwise unbalance the scanners below.
const tsSrc = readFileSync(new URL('ui/src/lib/api/types.ts', root), 'utf8')
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/(^|\s)\/\/.*$/gm, '$1');
const wsMd = readFileSync(new URL('docs/contracts/ws.md', root), 'utf8');

/** Index of the bracket closing the one at `open` (same kind). */
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

/** Split at top-level `sep` (outside (), [], {}). Angle brackets are not
 *  tracked: `=>` and comparison text would unbalance them. */
function splitTop(src: string, sep: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = '';
  for (const ch of src) {
    if ('([{'.includes(ch)) depth++;
    else if (')]}'.includes(ch)) depth--;
    if (ch === sep && depth === 0) {
      out.push(cur);
      cur = '';
    } else cur += ch;
  }
  out.push(cur);
  return out.map((s) => s.trim()).filter(Boolean);
}

const snake = (name: string) =>
  name
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .toLowerCase();

interface RustField { name: string; omittedWhenEmpty: boolean; ty: string }
interface RustVariant { name: string; tag: string; fields: RustField[] | null }

function rustVariants(): RustVariant[] {
  const start = rust.indexOf('pub enum Event {');
  assert.ok(start >= 0, 'pub enum Event not found');
  const open = rust.indexOf('{', start);
  const body = rust
    .slice(open + 1, closing(rust, open))
    .split('\n')
    .map((l) => l.replace(/\/\/.*$/, ''))
    .join('\n');
  const out: RustVariant[] = [];
  let i = 0;
  while (i < body.length) {
    const m = /^\s*(#\[[^\]]*\]\s*)*([A-Z][A-Za-z0-9]*)\s*/.exec(body.slice(i));
    if (!m) break;
    const name = m[2];
    i += m[0].length;
    let fields: RustField[] | null = [];
    if (body[i] === '{') {
      const end = closing(body, i);
      fields = [];
      let pendingSkip = false;
      for (const part of splitTop(body.slice(i + 1, end), ',')) {
        let p = part;
        // Attributes before a field.
        for (;;) {
          const a = /^#\[serde\(([^\]]*)\)\]\s*/.exec(p) ?? /^#\[[^\]]*\]\s*/.exec(p);
          if (!a) break;
          if (/skip_serializing_if/.test(a[1] ?? '')) pendingSkip = true;
          if (/flatten/.test(a[1] ?? '')) fields = null;
          p = p.slice(a[0].length);
        }
        const f = /^(?:pub\s+)?([a-z_][a-z0-9_]*)\s*:\s*([\s\S]*)$/.exec(p);
        if (f && fields) fields.push({ name: f[1], omittedWhenEmpty: pendingSkip, ty: f[2].trim() });
        pendingSkip = false;
      }
      i = end + 1;
    } else if (body[i] === '(') {
      fields = null; // newtype variant: its struct's fields — not compared here
      i = closing(body, i) + 1;
    }
    const comma = /^\s*,?/.exec(body.slice(i));
    i += comma ? comma[0].length : 0;
    out.push({ name, tag: snake(name), fields });
  }
  return out;
}

interface TsMember { tag: string; keys: Map<string, { optional: boolean; ty: string }> }

function tsInterfaceKeys(name: string): string {
  const m = new RegExp(`export interface ${name}\\s*(?:extends[^{]*)?\\{`).exec(tsSrc);
  assert.ok(m, `interface ${name} not found in types.ts`);
  const open = m.index + m[0].length - 1;
  return tsSrc.slice(open + 1, closing(tsSrc, open));
}

function tsMembers(): TsMember[] {
  const start = tsSrc.indexOf('export type OttoEvent =');
  assert.ok(start >= 0, 'OttoEvent not found');
  let i = start + 'export type OttoEvent ='.length;
  // The union ends at the first top-level `;`.
  let depth = 0;
  let end = i;
  for (; end < tsSrc.length; end++) {
    const ch = tsSrc[end];
    if ('([{'.includes(ch)) depth++;
    else if (')]}'.includes(ch)) depth--;
    else if (ch === ';' && depth === 0) break;
  }
  const union = tsSrc
    .slice(i, end)
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .split('\n')
    .map((l) => l.replace(/\/\/.*$/, ''))
    .join('\n');
  return splitTop(union, '|').map((member) => {
    const body = member.startsWith('{') ? member.slice(1, closing(member, 0)) : tsInterfaceKeys(member);
    const keys = new Map<string, { optional: boolean; ty: string }>();
    let tag = '';
    for (const part of splitTop(body.replace(/\/\*[\s\S]*?\*\//g, ''), ';').flatMap((p) => p.split('\n').length > 1 ? splitTop(p, '\n') : [p])) {
      const k = /^(?:readonly\s+)?([a-z_][a-z0-9_]*)(\?)?\s*:\s*([\s\S]*)$/.exec(part.trim());
      if (!k) continue;
      if (k[1] === 'type') {
        tag = /'([^']+)'/.exec(k[3])?.[1] ?? '';
        continue;
      }
      keys.set(k[1], { optional: !!k[2], ty: k[3].trim() });
    }
    assert.ok(tag, `OttoEvent member without a literal type tag: ${member.slice(0, 80)}`);
    return { tag, keys };
  });
}

const variants = rustVariants();
const members = tsMembers();
const byTag = new Map(members.map((m) => [m.tag, m]));

test('the parsers see the whole contract (sanity)', () => {
  assert.ok(variants.length >= 79, `parsed ${variants.length} Rust variants`);
  assert.equal(new Set(variants.map((v) => v.tag)).size, variants.length, 'unique tags');
  assert.ok(members.length >= variants.length - 2, `parsed ${members.length} OttoEvent members`);
});

test('type_name() agrees with the serde tag of every variant', () => {
  const arms = new Map([...rust.matchAll(/Event::([A-Za-z0-9]+)\s*\{\s*\.\.\s*\}\s*=>\s*"([a-z0-9_]+)"/g)].map((m) => [m[1], m[2]]));
  for (const v of variants) assert.equal(arms.get(v.name), v.tag, `type_name() arm for ${v.name}`);
});

test('every Rust event is in OttoEvent and ws.md; OttoEvent has no stale tags', () => {
  const rustTags = new Set(variants.map((v) => v.tag));
  const missingTs = variants.filter((v) => !byTag.has(v.tag)).map((v) => v.tag);
  const stale = members.filter((m) => !rustTags.has(m.tag)).map((m) => m.tag);
  // A whole wire frame, not a substring: `notification` used to pass only
  // because `notifications_changed` is documented (S13-307).
  const missingDoc = variants.filter((v) => !docTags.has(v.tag)).map((v) => v.tag);
  assert.deepEqual(missingTs, [], 'add these to OttoEvent in ui/src/lib/api/types.ts');
  assert.deepEqual(stale, [], 'OttoEvent members with no Rust variant');
  assert.deepEqual(missingDoc, [], 'document these in docs/contracts/ws.md');
});

// Event-stream section of ws.md: every `{"type":"…"}` frame it shows, and
// every backticked tag in its headings.
const wsEvents = wsMd.slice(wsMd.indexOf('## 2. Event stream'));
const docTags = new Set([...wsEvents.matchAll(/"type"\s*:\s*"([a-z0-9_]+)"/g)].map((m) => m[1]));
const headingTags = new Set(
  [...wsEvents.matchAll(/^#+ .*$/gm)].flatMap((h) => [...h[0].matchAll(/`([a-z0-9_]+)`/g)].map((m) => m[1])),
);
/** /ws/events protocol frames that are not `Event` variants. */
const PROTOCOL_FRAMES = new Set(['hello', 'hello_ack', 'presence', 'resync', 'subscribe', 'subscribe_ack', 'ui_command', 'ui_command_cancel']);

test('ws.md documents no stale event tags', () => {
  assert.ok(wsEvents.length > 1000, 'the "## 2. Event stream" section was found');
  const rustTags = new Set(variants.map((v) => v.tag));
  const stale = [...new Set([...docTags, ...headingTags])].filter((t) => !rustTags.has(t) && !PROTOCOL_FRAMES.has(t));
  assert.deepEqual(stale, [], 'ws.md shows frames no Rust Event variant sends (remove them, or list a protocol frame above)');
});

test('each event\'s fields match its OttoEvent member (names + omitted-when-empty → optional)', () => {
  const problems: string[] = [];
  let nullable = 0;
  for (const v of variants) {
    const m = byTag.get(v.tag);
    if (!m || v.fields === null) continue;
    const rustNames = new Set(v.fields.map((f) => f.name));
    for (const f of v.fields) {
      const k = m.keys.get(f.name);
      if (!k) problems.push(`${v.tag}.${f.name}: in Rust, missing in TS`);
      else if (f.omittedWhenEmpty && !k.optional) problems.push(`${v.tag}.${f.name}: omitted when empty on the wire but required in TS`);
      // `Option<T>` serialized without skip_serializing_if is a literal
      // `null` on the wire: the TS type must admit it.
      else if (/^Option\s*</.test(f.ty) && !f.omittedWhenEmpty && ++nullable && !/\bnull\b|\bunknown\b|\bany\b/.test(k.ty))
        problems.push(`${v.tag}.${f.name}: Option<…> is sent as null but the TS type (${k.ty}) has no null`);
    }
    for (const key of m.keys.keys()) if (!rustNames.has(key)) problems.push(`${v.tag}.${key}: in TS, not sent by Rust`);
  }
  assert.deepEqual(problems, []);
  assert.ok(nullable >= 5, `the Option→null check saw ${nullable} fields (scanner sanity)`);
});
