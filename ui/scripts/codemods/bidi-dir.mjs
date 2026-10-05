#!/usr/bin/env node
// Codemod: give every free-text field an explicit base direction (a11y P2,
// accessibility.md §6). Idempotent — re-run it after merges:
//
//   node scripts/codemods/bidi-dir.mjs          # rewrite src/ in place
//   node scripts/codemods/bidi-dir.mjs --dry    # list what would change
//
// A <textarea> or a text-like <input> (no type, or text/search/url/email/tel)
// with no `dir` gets:
//   dir="ltr"   when it holds code, a path, a URL, a command, a key or other
//               machine text (judged from its type / class / bind / name /
//               placeholder / aria-label — see CODEISH below), so a Hebrew UI
//               never reorders `src/app.ts` or `SELECT *`;
//   dir="auto"  otherwise — human language, so a Hebrew/Arabic message lays
//               out right-to-left and an English one left-to-right whatever
//               the UI direction is.
// Tags with a spread (`{...rest}`) or a dynamic `type={…}` are left alone (the
// spread may carry `dir`). ui-guards' `bidi-dir` rule keeps the tree here.
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { startTags, attrValue } from './markup.mjs';

const UI = fileURLToPath(new URL('../..', import.meta.url));
const SRC = join(UI, 'src');
const DRY = process.argv.includes('--dry');

/** Words that mark a field as machine text (case-insensitive, on the tag's
 *  identifying attributes). */
export const CODEISH =
  /\b(?:code|json|sql|yaml|toml|regexp?|url|uri|href|path|dir|cwd|cmd|command|shell|script|cron|host(?:name)?|port|token|secret|api[-_ ]?key|password|email|branch|repo|glob|selector|endpoint|cidr|ip|dsn|env|arn|bucket|topic|namespace|cluster|curl|jql|cql|xpath|jsonpath|filename|slug|sha|kube|pod|docker|ssh|schema|mongo|redis|kafka|expr|args?|argv|webhook|model|jq|grep|filter|headers?|statement|sink)(?:s|text)?\b/i;
/** A placeholder only counts by its SHAPE (prose placeholders mention
 *  "endpoint" or "schema" all the time): a URL, a path, JSON, KEY=value, a
 *  flag, a template. */
const CODE_PLACEHOLDER = /^\s*['"`]?\s*(?:e\.g\.\s*)?(?:https?:|[/~{[]|[A-Z][A-Z0-9_]*=|-{1,2}\w|\w+\.(?:ts|js|py|rs|go|md|json|ya?ml|sh|sql|txt|log|toml)\b|SELECT\b|ALTER\b|[\w.-]+:\s*\w)|\{\{|&#123;/;
/** Class names are noisier (layout words like `header`/`container`), so only
 *  explicit code styling counts there… */
const CODE_CLASS = /(?:^|[\s-])(?:mono|code|json|sql|cm|monospace|url|path|cmd|regex)(?:$|[\s-])/i;
/** …unless the field is plainly prose written in a mono face (markdown notes,
 *  agent instructions). */
const PROSE = /\b(?:md|markdown|instructions?|notes?|prompt|brief|message|comment|description|desc|body_md)\b/i;

/** The words of an identifying attribute: `bind:value={evidence[c.id]}` reads
 *  as `evidence` (index expressions dropped), `{`${label} Markdown`}` as
 *  `label Markdown`. */
const words = (v) => (v ?? '').replace(/\[[^\]]*\]/g, ' ').replace(/[^A-Za-z0-9]+/g, ' ').replace(/([a-z])([A-Z])/g, '$1 $2');

export function dirFor(tag, attrs) {
  if (tag === 'input') {
    const type = attrValue(attrs, 'type');
    if (type && /^(url|email|tel)$/.test(type)) return 'ltr';
  }
  const ident = ['bind:value', 'value', 'name', 'id', 'aria-label', 'autocomplete', 'inputmode'].map((a) => words(attrValue(attrs, a))).join(' ');
  if (PROSE.test(ident)) return 'auto';
  if (CODE_CLASS.test(attrValue(attrs, 'class') ?? '') || /class:mono\b/.test(attrs)) return 'ltr';
  if (CODE_PLACEHOLDER.test(attrValue(attrs, 'placeholder') ?? '')) return 'ltr';
  return CODEISH.test(ident) ? 'ltr' : 'auto';
}

/** The tags this codemod (and the guard) cover. */
export function needsDir(tag, attrs) {
  if (/(^|\s)dir\s*=|\{dir\}/.test(attrs)) return false;
  if (/\{\s*\.\.\./.test(attrs)) return false;
  if (tag === 'textarea') return true;
  if (tag !== 'input') return false;
  if (/(^|\s)type\s*=\s*\{/.test(attrs)) return false; // dynamic type
  const type = attrValue(attrs, 'type');
  return !type || /^(text|search|url|email|tel)$/.test(type);
}

function walk(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.svelte')) out.push(p);
  }
  return out;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  let changed = 0;
  let files = 0;
  for (const p of walk(SRC)) {
    const text = readFileSync(p, 'utf8');
    const edits = [];
    for (const t of startTags(text, ['textarea', 'input'])) {
      if (!needsDir(t.tag, t.attrs)) continue;
      edits.push({ at: t.nameEnd, insert: ` dir="${dirFor(t.tag, t.attrs)}"` });
    }
    if (!edits.length) continue;
    files++;
    changed += edits.length;
    let out = text;
    for (const e of edits.sort((a, b) => b.at - a.at)) out = out.slice(0, e.at) + e.insert + out.slice(e.at);
    if (DRY) console.log(`${relative(UI, p)}: ${edits.length}`);
    else writeFileSync(p, out);
  }
  console.log(`bidi-dir: ${DRY ? 'would add' : 'added'} dir to ${changed} field(s) in ${files} file(s)`);
}
