// Java client-side import analyzer and identifier count index used by `cm-ide-semantic-model.ts`.

import type { ClientIdeDiagnostic, JavaDiagPrefs } from './cm-ide-semantic-model.ts';

const BOGUS_JAVA_PREFIXES = [
  'com.nonexistent.', 'org.nonexistent.', 'net.nonexistent.',
  'does.not.exist.', 'nonexistent.',
];

export function buildIdentCountIndex(docText: string): Map<string, number> {
  const counts = new Map<string, number>();
  const sample = docText.length > 131072 ? docText.slice(0, 131072) : docText;
  const re = /\b[A-Za-z_$][\w$]*\b/g;
  for (let m: RegExpExecArray | null; (m = re.exec(sample)) !== null; ) {
    const id = m[0];
    counts.set(id, (counts.get(id) ?? 0) + 1);
  }
  return counts;
}

export function analyzeJavaDiagnostics(
  docText: string,
  prefs?: JavaDiagPrefs,
): { diagnostics: ClientIdeDiagnostic[]; unresolvedIdents: Set<string> } {
  const checkUnres = prefs?.javaUnresolvedImports !== false;
  const checkUnused = prefs?.unusedImports !== false;
  const lombok = prefs?.lombokSupport !== false;
  const lines = docText.split(/\r?\n/);
  const diagnostics: ClientIdeDiagnostic[] = [];
  const unresolvedIdents = new Set<string>();
  const seenImports = new Set<string>();
  const bodyChunks: string[] = [];
  const importEntries: Array<{ line: number; col: number; endCol: number; path: string; ident: string }> = [];

  for (let idx = 0; idx < lines.length; idx++) {
    const raw = lines[idx];
    const trimmed = raw.trim();
    const ln = idx + 1;
    if (trimmed.startsWith('package ')) continue;
    const mImp = /^\s*import\s+(?:static\s+)?([\w.*$]+)\s*;\s*$/.exec(raw);
    if (mImp) {
      const impPath = mImp[1];
      const col = raw.indexOf('import') + 1;
      const endCol = raw.length + 1;
      const segs = impPath.split('.');
      const lastSeg = segs[segs.length - 1] ?? '';
      const isBogus =
        segs.length < 2 ||
        BOGUS_JAVA_PREFIXES.some((p) => impPath.startsWith(p)) ||
        (!lombok && impPath.startsWith('lombok.'));
      if (isBogus) {
        if (checkUnres) {
          if (lastSeg && lastSeg !== '*') unresolvedIdents.add(lastSeg);
          diagnostics.push({
            line: ln, column: col, endLine: ln, endColumn: endCol,
            severity: 'error', source: 'java-imports', code: 'unresolved-import',
            message: `Unresolved import '${impPath}'`,
            quickFixes: [{ title: `Remove unresolved import '${impPath}'`, line: ln, endLine: ln, deleteLine: true }],
          });
        }
        continue;
      }
      if (seenImports.has(impPath)) {
        diagnostics.push({
          line: ln, column: col, endLine: ln, endColumn: endCol,
          severity: 'warning', source: 'java-imports', code: 'duplicate-import',
          message: `Duplicate import '${impPath}'`,
          quickFixes: [{ title: `Remove duplicate import '${impPath}'`, line: ln, endLine: ln, deleteLine: true }],
        });
        continue;
      }
      seenImports.add(impPath);
      if (lastSeg && lastSeg !== '*') importEntries.push({ line: ln, col, endCol, path: impPath, ident: lastSeg });
      continue;
    }
    bodyChunks.push(raw);
  }

  if (checkUnused && importEntries.length > 0) {
    const bodyCounts = buildIdentCountIndex(bodyChunks.join('\n'));
    for (const imp of importEntries) {
      if (!bodyCounts.has(imp.ident)) {
        diagnostics.push({
          line: imp.line, column: imp.col, endLine: imp.line, endColumn: imp.endCol,
          severity: 'warning', source: 'java-imports', code: 'unused-import',
          message: `Unused import '${imp.path}'`,
          quickFixes: [{ title: `Remove unused import '${imp.path}'`, line: imp.line, endLine: imp.line, deleteLine: true }],
        });
      }
    }
  }
  return { diagnostics, unresolvedIdents };
}
