// Static-import closure over ui/src source (no build): which files a module
// pulls in eagerly. `import type` and dynamic `import()` are not edges.
// Used by the shell/startup guards (perf F2/F4) to keep heavy stores and
// editors out of the shell and the Home chunk.
import { existsSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';

const EXTS = ['', '.ts', '.svelte', '.js', '/index.ts'];

function resolve(from: string, spec: string): string | null {
  if (!spec.startsWith('.')) return null; // packages are out of scope
  const base = join(dirname(from), spec);
  for (const e of EXTS) {
    const p = base + e;
    if (existsSync(p) && statSync(p).isFile()) return p;
  }
  return null;
}

/** The static import specifiers of one source file (scripts only for .svelte). */
export function staticImports(file: string): string[] {
  let src = readFileSync(file, 'utf8');
  if (file.endsWith('.svelte')) {
    src = [...src.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script\b[^>]*>/gi)].map((m) => m[1]).join('\n');
  }
  src = src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  const out: string[] = [];
  for (const m of src.matchAll(/^\s*(import|export)\s+(type\s+)?([^'";]*?)\s*(?:from\s+)?['"]([^'"]+)['"]/gm)) {
    if (m[2]) continue; // `import type … from` / `export type … from`
    if (m[1] === 'export' && !/\bfrom\b|\*/.test(m[0])) continue;
    out.push(m[4]);
  }
  return out;
}

/** Every file reachable from `entry` through static imports (absolute paths). */
export function staticClosure(entry: string): Set<string> {
  const seen = new Set<string>();
  const stack = [entry];
  while (stack.length) {
    const f = stack.pop()!;
    if (seen.has(f)) continue;
    seen.add(f);
    if (!/\.(ts|svelte|js)$/.test(f)) continue;
    for (const spec of staticImports(f)) {
      const r = resolve(f, spec);
      if (r && !seen.has(r)) stack.push(r);
    }
  }
  return seen;
}

/** The import chain from `entry` to `target` (for a readable failure). */
export function chainTo(entry: string, target: string, root: string): string[] {
  const prev = new Map<string, string | null>([[entry, null]]);
  const queue = [entry];
  while (queue.length) {
    const f = queue.shift()!;
    if (f === target) break;
    if (!/\.(ts|svelte|js)$/.test(f)) continue;
    for (const spec of staticImports(f)) {
      const r = resolve(f, spec);
      if (r && !prev.has(r)) {
        prev.set(r, f);
        queue.push(r);
      }
    }
  }
  const chain: string[] = [];
  for (let at: string | null | undefined = target; at; at = prev.get(at)) chain.unshift(relative(root, at));
  return chain;
}
