// Otto's own ⌥⏎ fallbacks when no server action fits a line: a missing Go / Java stdlib
// import, a JSON trailing comma, and an import a diagnostic calls unused or unresolved.
// Split out of `ideLspAndSearchHelpers.ts`.

import type { IdeCodeAction } from '../store/ideTypes';

const GO_STDLIB_BY_IDENT: Record<string, string> = {
  fmt: 'fmt', context: 'context', strings: 'strings', errors: 'errors', time: 'time',
  http: 'net/http', json: 'encoding/json', os: 'os', io: 'io', sync: 'sync', strconv: 'strconv',
  filepath: 'path/filepath', regexp: 'regexp', bytes: 'bytes', sort: 'sort', math: 'math',
};

const JAVA_UTIL_BY_IDENT: Record<string, string> = {
  List: 'java.util.List', ArrayList: 'java.util.ArrayList', Map: 'java.util.Map',
  HashMap: 'java.util.HashMap', Set: 'java.util.Set', HashSet: 'java.util.HashSet',
  Optional: 'java.util.Optional', Objects: 'java.util.Objects', UUID: 'java.util.UUID',
  Collectors: 'java.util.stream.Collectors',
};

export function buildFallbackQuickFixes(path: string, content: string, line: number, diagMsg = ''): IdeCodeAction[] {
  if (!content) return [];
  const lines = content.split('\n');
  const idx = Math.max(0, Math.min(lines.length - 1, line - 1));
  const curLine = lines[idx] ?? '';
  const actions: IdeCodeAction[] = [];

  // Only when a diagnostic on the line says the import is unused or unresolved: a used import is not a fix target.
  const isImportLine = /^\s*(?:import|use)\b/.test(curLine) || (path.endsWith('.go') && /^\s*(?:[A-Za-z_]\w*\s+)?"[^"]+"\s*$/.test(curLine));
  if (isImportLine && /unused|not used|never used|never read|unresolved|cannot find|could not be resolved|not found/i.test(diagMsg)) {
    const endLine = idx + 1 < lines.length ? line + 1 : line;
    const endCol = idx + 1 < lines.length ? 1 : curLine.length + 1;
    actions.push({
      id: 'remove-import-line',
      title: 'Remove unused / unresolved import',
      kind: 'quickfix',
      is_preferred: false,
      edits: [{ path, start_line: line, start_col: 1, end_line: endLine, end_col: endCol, new_text: '' }],
    });
  }

  if (path.endsWith('.go')) {
    const undefMatch = diagMsg.match(/(?:undefined|undeclared name):\s*([A-Za-z_]\w*)/i) ?? curLine.match(/\b(fmt|context|strings|errors|time|http|json|os|io|sync|strconv)\./);
    const ident = undefMatch?.[1] ?? '';
    const pkg = GO_STDLIB_BY_IDENT[ident];
    if (pkg && !content.includes(`"${pkg}"`)) {
      const pkgLineIdx = lines.findIndex((l) => /^\s*package\s+\w+/.test(l));
      const insertLine = pkgLineIdx >= 0 ? pkgLineIdx + 2 : 1;
      actions.push({
        id: `go-import-${pkg}`,
        title: `Import "${pkg}"`,
        kind: 'quickfix',
        is_preferred: true,
        edits: [{ path, start_line: insertLine, start_col: 1, end_line: insertLine, end_col: 1, new_text: `\nimport "${pkg}"\n` }],
      });
    }
  } else if (path.endsWith('.java')) {
    const symMatch = diagMsg.match(/(?:cannot find symbol|unresolved).*?\b([A-Z]\w*)/i) ?? curLine.match(/\b(List|ArrayList|Map|HashMap|Set|HashSet|Optional|Objects|UUID|Collectors)\b/);
    const sym = symMatch?.[1] ?? '';
    const fqn = JAVA_UTIL_BY_IDENT[sym];
    if (fqn && !content.includes(`import ${fqn};`)) {
      const pkgIdx = lines.findIndex((l) => /^\s*package\s+[\w.]+;/.test(l));
      const insertLine = pkgIdx >= 0 ? pkgIdx + 2 : 1;
      actions.push({
        id: `java-import-${fqn}`,
        title: `Import '${fqn}'`,
        kind: 'quickfix',
        is_preferred: true,
        edits: [{ path, start_line: insertLine, start_col: 1, end_line: insertLine, end_col: 1, new_text: `import ${fqn};\n` }],
      });
    }
  } else if (path.endsWith('.json') && /,\s*$/.test(curLine)) {
    const commaIdx = curLine.lastIndexOf(',');
    actions.push({
      id: 'json-remove-trailing-comma',
      title: 'Remove trailing comma',
      kind: 'quickfix',
      is_preferred: true,
      edits: [{ path, start_line: line, start_col: commaIdx + 1, end_line: line, end_col: commaIdx + 2, new_text: '' }],
    });
  }
  return actions;
}
