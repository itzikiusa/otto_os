// IntelliJ-style file nesting for the project tree: a lockfile / generated sibling folds
// under the file it belongs to (go.sum under go.mod), only when both sit in the same folder.

import type { IdeFsEntry } from '../store/ideTypes';

const PARENT_BY_NAME: Readonly<Record<string, string>> = {
  'go.sum': 'go.mod',
  'go.work.sum': 'go.work',
  'package-lock.json': 'package.json',
  'yarn.lock': 'package.json',
  'pnpm-lock.yaml': 'package.json',
  'npm-shrinkwrap.json': 'package.json',
  'Cargo.lock': 'Cargo.toml',
  'poetry.lock': 'pyproject.toml',
  'uv.lock': 'pyproject.toml',
  'pdm.lock': 'pyproject.toml',
  'Gemfile.lock': 'Gemfile',
  'composer.lock': 'composer.json',
};

/** The sibling name `name` nests under, if a rule covers it. */
export function nestParentName(name: string): string | null {
  const exact = PARENT_BY_NAME[name];
  if (exact) return exact;
  if (/^tsconfig\..+\.json$/.test(name)) return 'tsconfig.json';
  if (/^\.env\..+/.test(name)) return '.env';
  const compiled = /^(.+)\.(?:js|js\.map|d\.ts)$/.exec(name);
  return compiled ? `${compiled[1]}.ts` : null;
}

export interface TreeNesting {
  /** Parent file path → its nested siblings, in listing order. */
  childrenOf: Map<string, IdeFsEntry[]>;
  /** Nested file path → its parent file path. */
  parentOf: Map<string, string>;
}

/** Nesting for ONE folder's listing (so nothing ever nests across folders); `enabled` is
 *  the "Nest related files" setting. */
export function nestSiblings(entries: readonly IdeFsEntry[], enabled = true): TreeNesting {
  const childrenOf = new Map<string, IdeFsEntry[]>();
  const parentOf = new Map<string, string>();
  if (!enabled) return { childrenOf, parentOf };
  const files = new Map<string, IdeFsEntry>();
  for (const e of entries) if (e.kind === 'file') files.set(e.name, e);
  for (const e of entries) {
    if (e.kind !== 'file') continue;
    const parentName = nestParentName(e.name);
    const parent = parentName ? files.get(parentName) : undefined;
    if (!parent || parent === e) continue;
    parentOf.set(e.path, parent.path);
    const kids = childrenOf.get(parent.path);
    if (kids) kids.push(e);
    else childrenOf.set(parent.path, [e]);
  }
  return { childrenOf, parentOf };
}
