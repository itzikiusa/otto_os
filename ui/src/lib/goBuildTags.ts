// Pure Go build constraint helpers (`//go:build` and `// +build`), CLI `-tags`
// flag merging, and per-module override resolution shared between IDE UI and
// lazy CodeMirror LSP without pulling CodeMirror into `page:ide`'s eager bundle.

import { normalizeMacPrivatePath } from './fileUri.ts';

const GO_STD_BUILD_CONSTRAINTS = new Set([
  'linux', 'darwin', 'windows', 'freebsd', 'netbsd', 'openbsd', 'plan9', 'solaris',
  'aix', 'zos', 'js', 'wasip1', 'unix', 'android', 'ios', 'illumos', 'dragonfly',
  'amd64', 'arm64', '386', 'arm', 'ppc64', 'ppc64le', 'mips', 'mipsle', 'mips64',
  'mips64le', 'riscv64', 's390x', 'wasm', 'loong64', 'cgo', 'gc', 'gccgo', 'ignore',
]);

export const VALID_GO_BUILD_TAG_RE = /^[A-Za-z0-9_.-]+$/;

export function sanitizeGoBuildTags(tags: readonly string[] | null | undefined): string[] {
  if (!tags || tags.length === 0) return [];
  const out: string[] = [];
  const seen = new Set<string>();
  for (let i = 0; i < tags.length; i++) {
    const stripped = (tags[i] ?? '').replace(/^\s*--?tags=/, '');
    for (const part of stripped.split(/[,\s]+/)) {
      const clean = part.trim().replace(/^--?tags=/, '');
      if (!clean || !VALID_GO_BUILD_TAG_RE.test(clean) || seen.has(clean)) continue;
      seen.add(clean);
      out.push(clean);
    }
  }
  return out;
}

/** Parse a `//go:build` boolean expression and push only non-negated tag atoms,
 *  tracking `!` across parenthesized groups so `!(slow || flaky)` never extracts
 *  `slow` or `flaky` as positive required build tags. */
function collectPositiveGoBuildExprTags(expr: string, pushTag: (t: string) => void): void {
  const groupNegatedStack: boolean[] = [false];
  let pendingBang = false;
  let i = 0;
  while (i < expr.length) {
    const ch = expr[i];
    if (/\s/.test(ch)) {
      i++;
      continue;
    }
    if (ch === '/' && expr[i + 1] === '/') break;
    if (ch === '!') {
      pendingBang = !pendingBang;
      i++;
      continue;
    }
    if (ch === '(') {
      const parentNeg = groupNegatedStack[groupNegatedStack.length - 1] ?? false;
      groupNegatedStack.push(pendingBang ? !parentNeg : parentNeg);
      pendingBang = false;
      i++;
      continue;
    }
    if (ch === ')') {
      if (groupNegatedStack.length > 1) groupNegatedStack.pop();
      pendingBang = false;
      i++;
      continue;
    }
    if (ch === '&' || ch === '|') {
      pendingBang = false;
      i++;
      continue;
    }
    const m = /^[A-Za-z][A-Za-z0-9_.-]*/.exec(expr.slice(i));
    if (m) {
      const groupNeg = groupNegatedStack[groupNegatedStack.length - 1] ?? false;
      const effectiveNeg = pendingBang ? !groupNeg : groupNeg;
      if (!effectiveNeg) pushTag(m[0]);
      pendingBang = false;
      i += m[0].length;
      continue;
    }
    pendingBang = false;
    i++;
  }
}

/** Extract custom Go build tags (`//go:build ...` / `// +build ...`) from file header before `package`. */
export function extractGoBuildTagsFromSource(docText: string): string[] {
  if (!docText) return [];
  const lines = docText.slice(0, 4096).split(/\r?\n/);
  const found: string[] = [];
  const seen = new Set<string>();
  const pushTag = (raw: string) => {
    const t = raw.trim();
    if (!t || GO_STD_BUILD_CONSTRAINTS.has(t) || /^go1\.\d+$/.test(t) || seen.has(t)) return;
    seen.add(t);
    found.push(t);
  };
  for (let i = 0; i < Math.min(lines.length, 40); i++) {
    const trimmed = lines[i].trim();
    if (!trimmed) continue;
    if (/^package\s+\w+/.test(trimmed)) break;
    const mGoBuild = /^\/\/go:build\s+(.+)$/.exec(trimmed);
    if (mGoBuild) {
      collectPositiveGoBuildExprTags(mGoBuild[1], pushTag);
      continue;
    }
    const mPlusBuild = /^\/\/\s*\+build\s+(.+)$/.exec(trimmed);
    if (mPlusBuild) {
      for (const clause of mPlusBuild[1].split(/\s+/)) {
        for (const term of clause.split(',')) {
          if (term && !term.startsWith('!')) pushTag(term);
        }
      }
      continue;
    }
    if (!trimmed.startsWith('//') && !trimmed.startsWith('/*') && !trimmed.startsWith('*')) break;
  }
  return found;
}

/** Merge Go `-tags=...` into CLI args while respecting the `--` end-of-flags separator
 *  (anything after `--` belongs to the target binary and is never stripped or modified). */
export function mergeGoTagsCliArgs(rawArgs: readonly string[], extraTags: readonly string[]): string[] {
  const cleanExtra = sanitizeGoBuildTags(extraTags);
  const existingTags: string[] = [];
  const head: string[] = [];
  let tailAfterDoubleDash: string[] | null = null;

  for (let i = 0; i < rawArgs.length; i++) {
    const arg = rawArgs[i];
    if (arg === '--') {
      tailAfterDoubleDash = rawArgs.slice(i);
      break;
    }
    if (arg === '-tags' || arg === '--tags') {
      if (i + 1 < rawArgs.length && rawArgs[i + 1] !== '--') {
        existingTags.push(...sanitizeGoBuildTags([rawArgs[i + 1]]));
        i++;
      }
      continue;
    }
    const eqMatch = /^--?tags=(.*)$/.exec(arg);
    if (eqMatch) {
      existingTags.push(...sanitizeGoBuildTags([eqMatch[1]]));
      continue;
    }
    head.push(arg);
  }

  const merged = sanitizeGoBuildTags([...existingTags, ...cleanExtra]);
  const out = [...head];
  if (merged.length > 0) {
    out.push(`-tags=${merged.join(',')}`);
  }
  if (tailAfterDoubleDash) {
    out.push(...tailAfterDoubleDash);
  }
  return out;
}

/** Resolve effective Go build tags for `repoOrFilePath` given `primaryRoot`,
 *  `primaryGoBuildTags`, and per-module overrides (`moduleGoBuildTags`), honoring
 *  explicit empty `[]` overrides on sibling/nested modules and never leaking
 *  `primaryGoBuildTags` onto unrelated attached sibling roots outside `primaryRoot`. */
export function resolveEffectiveRepoGoBuildTags(
  repoOrFilePath: string | null | undefined,
  primaryRoot: string | null | undefined,
  primaryGoBuildTags: readonly string[],
  moduleGoBuildTags?: Record<string, string[]> | null,
): string[] {
  const targetNorm = repoOrFilePath ? normalizeMacPrivatePath(repoOrFilePath) : '';
  const primaryNorm = primaryRoot ? normalizeMacPrivatePath(primaryRoot) : '';

  if (targetNorm && moduleGoBuildTags) {
    let bestRoot = '';
    let bestTags: string[] | null = null;
    for (const [modRoot, modTags] of Object.entries(moduleGoBuildTags)) {
      const normMod = normalizeMacPrivatePath(modRoot);
      if (
        (targetNorm === normMod || targetNorm.startsWith(`${normMod}/`)) &&
        normMod.length > bestRoot.length
      ) {
        bestRoot = normMod;
        bestTags = modTags;
      }
    }
    if (bestTags !== null) {
      return sanitizeGoBuildTags(bestTags);
    }
  }

  if (!targetNorm || !primaryNorm) {
    return sanitizeGoBuildTags(primaryGoBuildTags);
  }
  if (targetNorm === primaryNorm || targetNorm.startsWith(`${primaryNorm}/`)) {
    return sanitizeGoBuildTags(primaryGoBuildTags);
  }
  return [];
}

export const GO_BUILD_TAG_DIAG_RE =
  /No packages found for open file|build constraints exclude all Go files|no buildable Go source files/i;

export function rewriteGoBuildTagDiagnostic(
  rawMessage: string,
  rawCode?: string | number | { value?: string | number } | null,
  docText?: string | null,
): { message: string; isBuildTagDiag: boolean; tags: string[] } {
  const codeStr =
    rawCode != null && typeof rawCode === 'object' && 'value' in rawCode
      ? String(rawCode.value ?? '')
      : rawCode != null
        ? String(rawCode)
        : '';
  const isBuildTag = codeStr === 'otto_go_build_tag' || GO_BUILD_TAG_DIAG_RE.test(rawMessage);
  if (!isBuildTag) return { message: rawMessage, isBuildTagDiag: false, tags: [] };
  const tags = docText ? extractGoBuildTagsFromSource(docText) : [];
  if (codeStr === 'otto_go_build_tag' && !GO_BUILD_TAG_DIAG_RE.test(rawMessage)) {
    return { message: rawMessage, isBuildTagDiag: true, tags };
  }
  if (tags.length > 0) {
    const tagList = tags.join(',');
    return {
      message: `File requires Go build tag "${tagList}" (//go:build ${tags.join(', ')}) — enable "${tagList}" in Go Build Tags so gopls and Run/Debug include this file.`,
      isBuildTagDiag: true,
      tags,
    };
  }
  return {
    message:
      'File is excluded by Go build constraints (//go:build) or outside active Go packages — add its build tag in Go Build Tags to analyze, run, or debug it.',
    isBuildTagDiag: true,
    tags: [],
  };
}
