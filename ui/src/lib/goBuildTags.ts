// Pure Go build constraint helpers (`//go:build` and `// +build`) shared between
// the lightweight IDE subbar (`IdeEditorSubbar.svelte`) and the lazy CodeMirror
// LSP pipeline (`cm-ide-lsp-completion.ts`) without pulling CodeMirror into
// `page:ide`'s eager bundle.

const GO_STD_BUILD_CONSTRAINTS = new Set([
  'linux', 'darwin', 'windows', 'freebsd', 'netbsd', 'openbsd', 'plan9', 'solaris',
  'aix', 'zos', 'js', 'wasip1', 'unix', 'android', 'ios', 'illumos', 'dragonfly',
  'amd64', 'arm64', '386', 'arm', 'ppc64', 'ppc64le', 'mips', 'mipsle', 'mips64',
  'mips64le', 'riscv64', 's390x', 'wasm', 'loong64', 'cgo', 'gc', 'gccgo', 'ignore',
]);

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
      const expr = mGoBuild[1];
      for (const m of expr.matchAll(/(^|[^!A-Za-z0-9_.-])([A-Za-z][A-Za-z0-9_.-]*)/g)) {
        pushTag(m[2]);
      }
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
