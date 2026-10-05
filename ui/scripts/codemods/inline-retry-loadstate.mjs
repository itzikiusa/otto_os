#!/usr/bin/env node
// Codemod: the most common hand-rolled failed-load block → the shared
// LoadState (compact variant), which owns the one "Couldn’t load {what}.
// {detail} [Retry]" look, the role="alert" and the Retrying… state
// (components.md §11; ui-guards `inline-retry`). Idempotent; re-run after
// merges:
//
//   node scripts/codemods/inline-retry-loadstate.mjs [--dry]
//
// The shape it rewrites (an optional leading <Icon>, inline <strong>/<span>
// wrappers allowed, ghost/small button classes allowed):
//
//   <div|p class="…" role="alert">
//     Couldn’t load tasks. {tasks.error}          ← "Couldn’t / Could not /
//     <button class="btn small" onclick={retry}>Retry</button>   Failed to load"
//   </div|p>
//
//   → <LoadState variant="compact" what="tasks" error={tasks.error} empty onretry={retry} />
//
// and the EmptyState spelling of the same thing (a panel-sized failure):
//
//   <EmptyState icon="warning" title="Couldn’t load Usage" body={error}>
//     <button class="btn small" onclick={reload}><Icon … />Retry</button>
//   </EmptyState>
//
//   → <LoadState what="Usage" error={error} empty onretry={reload} />
//
// When the enclosing `{#if …}` doesn't test the detail expression itself (a
// `state === 'error'` flag), the detail can be empty while the block shows —
// LoadState would then render nothing — so it gets a fallback text.
//
// Skipped (left for a hand migration, still counted by the ratchet): blocks
// with no `{detail}` expression (LoadState needs the error text to render),
// several expressions, other controls next to Retry, or a message that isn't
// a "load" failure (a review run, a form submit), and wrappers carrying
// `use:` / `bind:` / `data-*` (virtual rows, spec hooks). The wrapper's CSS rules are
// removed when nothing else in the file uses that class any more; a rule that
// shares a selector list with a live class is reported for a hand edit.
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = fileURLToPath(new URL('../..', import.meta.url));
const SRC = join(UI, 'src');
const DRY = process.argv.includes('--dry');
const LOADSTATE = join(SRC, 'lib/components/LoadState.svelte');

function walk(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.svelte')) out.push(p);
  }
  return out;
}

// A `{…}` expression with one level of nested braces (`{() => void x({a})}`).
const EXPR = String.raw`\{((?:[^{}]|\{[^{}]*\})*)\}`;
const BLOCK = new RegExp(
  String.raw`<(div|p)\b([^>]*?\brole="alert"[^>]*)>` + // wrapper
    String.raw`((?:(?!<button|\{[#:/@])[\s\S])*?)` + // message (no blocks/controls; nesting checked below)
    String.raw`<button\s+(?:type="button"\s+)?class="[^"]*"\s+onclick=` +
    EXPR +
    String.raw`\s*>\s*Retry\s*</button>\s*</\1>`,
  'g',
);
const EMPTY_STATE = new RegExp(
  String.raw`<EmptyState\s+icon="warning"\s+title="(?:Couldn[’']t|Could not|Failed to) load ([^"{}<>]+?)"\s+body=` +
    EXPR +
    String.raw`\s*>\s*<button\s+(?:type="button"\s+)?class="[^"]*"\s+onclick=` +
    EXPR +
    String.raw`\s*>\s*(?:<Icon\b[^>]*/>\s*)?Retry\s*</button>\s*</EmptyState>`,
  'g',
);
const LOAD = /^(?:Couldn[’']t|Could not|Failed to) load ([^.:{}]+?)\s*[.:—-]?\s*$/i;

/** Plain-text message + the single `{detail}` expression, or null. */
function parseMessage(raw, tag) {
  const noIcon = raw.replace(/<Icon\b[^>]*\/>/g, '');
  // Text wrappers only, balanced — and never the wrapper's own closing tag
  // (the lazy match must not run past the block it started in).
  if (/<(?!\/?(?:strong|span|b|em|div|p)\b)[a-zA-Z/]/.test(noIcon)) return null;
  for (const t of ['div', 'p', 'span', 'strong']) {
    const open = (noIcon.match(new RegExp(`<${t}\\b`, 'g')) ?? []).length;
    const close = (noIcon.match(new RegExp(`</${t}>`, 'g')) ?? []).length;
    if (open !== close) return null;
  }
  if (/\brole="alert"/.test(noIcon)) return null;
  const text = noIcon.replace(/<\/?(?:strong|span|b|em|div|p)\b[^>]*>/g, ' ').replace(/\s+/g, ' ').trim();
  const exprs = [...text.matchAll(new RegExp(EXPR, 'g'))];
  if (exprs.length !== 1) return null;
  const before = text.slice(0, exprs[0].index).trim();
  const after = text.slice(exprs[0].index + exprs[0][0].length).trim();
  if (after && !/^[.)]?$/.test(after)) return null;
  const m = LOAD.exec(before);
  if (!m) return null;
  const what = m[1].trim();
  if (!what || /["{}<>]/.test(what)) return null;
  return { what, error: exprs[0][1].trim() };
}

/** Every static class named in a removed block (wrapper and children). */
function classesOf(markup) {
  const out = [];
  for (const m of markup.matchAll(/\bclass="([^"]*)"/g)) out.push(...m[1].split(/\s+/).filter((c) => /^[\w-]+$/.test(c)));
  for (const m of markup.matchAll(/\bclass:([\w-]+)/g)) out.push(m[1]);
  return out;
}

/** Drop `<style>` rules that only target classes no longer used in markup. */
function pruneCss(src, gone, rel, report) {
  const styleAt = src.search(/<style\b[^>]*>/);
  if (styleAt === -1) return src;
  const markup = src.slice(0, styleAt);
  // Still used = named in a class attribute, a class: directive, or as a
  // quoted word inside a class={…} expression (not any 'error' string).
  const dead = gone.filter(
    (c) => !new RegExp(String.raw`class="[^"]*(?<![\w-])${c}(?![\w-])|class:${c}(?![\w-])|class=\{[^}]*['"\`\s]${c}['"\`\s]`).test(markup),
  );
  if (dead.length === 0) return src;
  const deadRe = new RegExp(String.raw`\.(?:${dead.map((c) => c.replace(/[-]/g, '\\-')).join('|')})(?![\w-])`);
  // Rules start after the <style> tag itself; a selector never holds `<`.
  const open = /<style\b[^>]*>/.exec(src.slice(styleAt))[0];
  const head = src.slice(0, styleAt + open.length);
  let style = src.slice(styleAt + open.length);
  style = style.replace(/(\n)([ \t]*)([^{}\n@<][^{}<]*?)\s*\{[^{}]*\}[ \t]*(?=\n)/g, (whole, nl, _ind, sel) => {
    const parts = sel.split(',').map((s) => s.trim());
    const hits = parts.filter((s) => deadRe.test(s));
    if (hits.length === 0) return whole;
    if (hits.length === parts.length) return '';
    report.push(`${rel}: selector list mixes a removed class — edit by hand: ${sel.trim()}`);
    return whole;
  });
  // A style block left with nothing in it goes too.
  if (/^\s*<\/style>\s*$/.test(style)) return src.slice(0, styleAt).replace(/\n+$/, '\n');
  return head + style;
}

function importPath(file) {
  let p = relative(dirname(file), LOADSTATE).split('\\').join('/');
  if (!p.startsWith('.')) p = './' + p;
  return p;
}

function addImport(src, file) {
  if (/import\s+LoadState\b/.test(src)) return src;
  const m = /<script\b(?![^>]*\bmodule\b)[^>]*>\n/.exec(src);
  if (!m) return null;
  // After the last top-level import, else right after <script>.
  const body = src.slice(m.index + m[0].length);
  const imports = [...body.matchAll(/^  import [\s\S]*?;\n/gm)];
  const at = m.index + m[0].length + (imports.length ? imports[imports.length - 1].index + imports[imports.length - 1][0].length : 0);
  return src.slice(0, at) + `  import LoadState from '${importPath(file)}';\n` + src.slice(at);
}

/** The detail expression, with a fallback unless the enclosing `{#if}` /
 *  `{:else if}` condition already guarantees it is set. */
function errorAttr(src, at, expr) {
  const conds = [...src.slice(0, at).matchAll(/\{(?:#if|:else if)\s+((?:[^{}]|\{[^{}]*\})*)\}/g)];
  const cond = conds.length ? conds[conds.length - 1][1] : '';
  const guarded = cond.includes(expr) && !/[!]\s*$/.test(cond.slice(0, cond.indexOf(expr)));
  return guarded ? `{${expr}}` : `{${expr} || 'No details were reported.'}`;
}

let changedFiles = 0;
let rewrites = 0;
const report = [];
for (const file of walk(SRC)) {
  const rel = relative(UI, file);
  if (rel.startsWith('src/lib/components/')) continue;
  const src = readFileSync(file, 'utf8');
  const styleAt = src.search(/<style\b[^>]*>/);
  const markupEnd = styleAt === -1 ? src.length : styleAt;
  const gone = new Set();
  let n = 0;
  const markup = src.slice(0, markupEnd);
  let head = markup.replace(BLOCK, (whole, _tag, attrs, message, handler, at) => {
    // A wrapper doing more than styling (a virtual-list row measured with
    // use:, a data-testid a spec reads) stays as it is.
    if (/\b(?:use|bind):|\bdata-/.test(attrs)) return whole;
    const parsed = parseMessage(message, _tag);
    if (!parsed) return whole;
    for (const c of classesOf(whole)) gone.add(c);
    n += 1;
    const what = parsed.what.replace(/&/g, '&amp;');
    return `<LoadState variant="compact" what="${what}" error=${errorAttr(markup, at, parsed.error)} empty onretry={${handler.trim()}} />`;
  });
  head = head.replace(EMPTY_STATE, (whole, what, body, handler, at) => {
    for (const c of classesOf(whole)) gone.add(c);
    n += 1;
    return `<LoadState what="${what.trim()}" error=${errorAttr(head, at, body.trim())} empty onretry={${handler.trim()}} />`;
  });
  if (n === 0) continue;
  // An EmptyState import nothing renders any more would fail the type check.
  if (!/<EmptyState\b/.test(head)) head = head.replace(/^  import EmptyState from '[^']+';\n/m, '');
  let out = addImport(head + src.slice(markupEnd), file);
  if (out == null) {
    report.push(`${rel}: no instance <script> to import LoadState into — skipped`);
    continue;
  }
  out = pruneCss(out, [...gone], rel, report);
  rewrites += n;
  changedFiles += 1;
  console.log(`${DRY ? 'would rewrite' : 'rewrote'} ${n} in ${rel}`);
  if (!DRY) writeFileSync(file, out);
}
for (const r of report) console.log('  ! ' + r);
console.log(`inline-retry → LoadState: ${rewrites} block(s) in ${changedFiles} file(s)${DRY ? ' (dry run)' : ''}`);
