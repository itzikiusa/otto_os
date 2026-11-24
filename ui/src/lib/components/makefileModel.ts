// What a Makefile defines, read from its text (pure; the IDE's Structure tab, ⌘-click /
// Go to Definition and hover for Makefiles — no language server). Recipe lines and
// define…endef bodies are skipped; a trailing backslash joins a line to the next.
import type { IdeGutterItem, IdeSymbolNode } from '../api/types';

export interface MakeDef {
  name: string;
  /** 1-based. */
  line: number;
  /** 1-based column of the name. */
  column: number;
  kind: 'target' | 'variable';
  /** The defining line, trimmed (the hover's text). */
  text: string;
  /** Targets: the last line of their recipe. */
  endLine: number;
}

export interface MakeOutline {
  targets: MakeDef[];
  variables: MakeDef[];
  /** `include` / `-include` / `sinclude` file names, as written. */
  includes: string[];
}

const ASSIGN = /^((?:(?:export|override|private)\s+)*)([^\s:#=?+!]+)\s*(?::::=|::=|:=|\?=|\+=|!=|=)/;
const SPECIAL = /^\.[A-Z_]+$/;

export function makefileOutline(text: string): MakeOutline {
  const out: MakeOutline = { targets: [], variables: [], includes: [] };
  const lines = text.split(/\r?\n/);
  let inDefine = false;
  let seenRule = false;
  let cont = false;
  let target: MakeDef[] = [];
  let recipe = false;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const ln = i + 1;
    const wasCont = cont;
    cont = /\\$/.test(raw);
    if (wasCont) {
      if (recipe) for (const t of target) t.endLine = ln;
      continue;
    }
    recipe = false;
    if (inDefine) {
      if (/^\s*endef\b/.test(raw)) inDefine = false;
      continue;
    }
    if (raw.startsWith('\t') && seenRule) {
      recipe = true;
      for (const t of target) t.endLine = ln;
      continue;
    }
    const line = raw.replace(/(^|[^\\])#.*$/, '$1');
    if (!line.trim()) continue;
    const lead = line.length - line.trimStart().length;
    const body = line.trimStart();
    const def = /^define\s+([^\s=]+)/.exec(body);
    if (def) {
      inDefine = true;
      out.variables.push(mk(def[1], ln, lead + body.indexOf(def[1], 6) + 1, 'variable', raw));
      continue;
    }
    const inc = /^(?:-include|sinclude|include)\s+(.+)$/.exec(body);
    if (inc) {
      out.includes.push(...inc[1].trim().split(/\s+/));
      continue;
    }
    if (/^(?:ifeq|ifneq|ifdef|ifndef|else|endif|unexport|vpath)\b/.test(body) || /^export\s+[^=:]*$/.test(body)) continue;
    const a = ASSIGN.exec(body);
    if (a) {
      out.variables.push(mk(a[2], ln, lead + a[1].length + 1, 'variable', raw));
      continue;
    }
    const colon = body.search(/:(?!=)/);
    if (colon <= 0) continue;
    seenRule = true;
    target = [];
    let col = lead;
    for (const name of body.slice(0, colon).split(/(\s+)/)) {
      if (name.trim() && !SPECIAL.test(name) && !name.includes('$')) {
        const t = mk(name, ln, col + 1, 'target', raw);
        out.targets.push(t);
        target.push(t);
      }
      col += name.length;
    }
  }
  return out;
}

function mk(name: string, line: number, column: number, kind: MakeDef['kind'], raw: string): MakeDef {
  return { name, line, column, kind, text: raw.trim(), endLine: line };
}

/** The Structure tab's rows: targets, then variables (one row per name). */
export function makefileSymbols(text: string): IdeSymbolNode[] {
  const o = makefileOutline(text);
  const seen = new Set<string>();
  const rows: IdeSymbolNode[] = [];
  for (const d of [...o.targets, ...o.variables]) {
    const key = `${d.kind}:${d.name}`;
    if (seen.has(key)) continue;
    seen.add(key);
    rows.push({ name: d.name, kind: d.kind, detail: null, line: d.line, end_line: d.endLine });
  }
  return rows;
}

/** A ▶ per target (pattern rules left out): `make <target>` in the Makefile's folder. */
export function makefileRunItems(text: string, path: string): IdeGutterItem[] {
  const cwd = path.slice(0, path.lastIndexOf('/')) || '/';
  const file = path.slice(path.lastIndexOf('/') + 1);
  // `-f` only for a file make would not pick by itself (Makefile.local, common.mk…).
  const pick = /^(?:GNUmakefile|makefile|Makefile)$/.test(file) ? [] : ['-f', file];
  return makefileOutline(text)
    .targets.filter((t) => !t.name.includes('%'))
    .map((t) => ({
      line: t.line,
      kind: 'main' as const,
      label: `make ${t.name}`,
      language: 'makefile',
      run_spec: { runner: 'custom', command: 'make', args: [...pick, t.name], cwd, language: 'makefile' },
    }));
}

/** The Makefile word at a 1-based column: a variable inside `$(…)` / `${…}`, a target,
 *  a prerequisite or a define's name; null on space or punctuation. */
export function makefileWordAt(lineText: string, column: number): string | null {
  const at = Math.max(0, column - 1);
  const isWord = (c: string | undefined): boolean => !!c && /[\w.%/-]/.test(c);
  let from = at, to = at;
  if (!isWord(lineText[at]) && isWord(lineText[at - 1])) from = to = at - 1;
  if (!isWord(lineText[from])) return null;
  while (from > 0 && isWord(lineText[from - 1])) from--;
  while (to < lineText.length && isWord(lineText[to])) to++;
  return lineText.slice(from, to);
}

/** Where `name` is defined in this file: its first assignment or define, else its rule. */
export function findMakefileDefinition(text: string, name: string): MakeDef | null {
  const o = makefileOutline(text);
  return o.variables.find((d) => d.name === name) ?? o.targets.find((d) => d.name === name) ?? null;
}
