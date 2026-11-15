import assert from 'node:assert/strict';
import { test } from 'node:test';
import { EditorState } from '@codemirror/state';
import {
  buildLspCompletionChangeSpec,
  buildLspInitializationOptions,
  decodeLspSemanticTokens,
  detectLspCompletionTrigger,
  IDLE_CLOSE_MS,
  isPosInCommentOrString,
  LSP_CODE_LANGUAGES,
  MAX_POOLED_LSP_CLIENTS,
  resolveIdeLspModuleRoot,
  shouldAttachIdeLsp,
  toAbsoluteIdeFilePath,
} from '../src/lib/components/cm-ide-lsp-completion.ts';
import {
  clearIdeViewPositions,
  computeMinimalTextUpdate,
  computeSmartEnterEdit,
  getIdeViewPosition,
  MAX_IDE_VIEW_POSITIONS,
  resolveIndentUnit,
  saveIdeViewPosition,
  useTabsForLang,
} from '../src/lib/components/cm-editor-smart.ts';

function createMemoryStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear() {
      map.clear();
    },
    getItem(k: string) {
      return map.get(k) ?? null;
    },
    key(idx: number) {
      return Array.from(map.keys())[idx] ?? null;
    },
    removeItem(k: string) {
      map.delete(k);
    },
    setItem(k: string, v: string) {
      map.set(k, v);
    },
  };
}

test('LSP pool limits and shouldAttachIdeLsp gate code buffers strictly and skip docs, configs, and secrets', () => {
  assert.equal(MAX_POOLED_LSP_CLIENTS, 3);
  assert.equal(IDLE_CLOSE_MS, 180_000);
  assert.deepEqual([...LSP_CODE_LANGUAGES], [
    'go',
    'rust',
    'java',
    'typescript',
    'javascript',
    'python',
  ]);

  assert.equal(shouldAttachIdeLsp('/ws/cmd/main.go', 'go', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/crates/lib.rs', 'rust', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/src/App.java', 'java', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/ui/index.ts', 'typescript', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/ui/view.tsx', undefined, '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/scripts/run.py', 'python', '/ws'), true);

  const rejected = [
    ['/ws/CLAUDE.md', 'markdown', '/ws'],
    ['/ws/package.json', 'json', '/ws'],
    ['/ws/docker-compose.yaml', 'yaml', '/ws'],
    ['/ws/Cargo.toml', 'toml', '/ws'],
    ['/ws/schema.sql', 'sql', '/ws'],
    ['/ws/data.csv', 'csv', '/ws'],
    ['/ws/index.html', 'html', '/ws'],
    ['/ws/tokens.css', 'css', '/ws'],
    ['/ws/Dockerfile', 'dockerfile', '/ws'],
    ['/ws/Makefile', 'makefile', '/ws'],
    ['/ws/scripts/check.sh', 'shell', '/ws'],
    ['untitled:1', 'go', '/ws'],
    ['jdt://contents/java.base/String.class', 'java', '/ws'],
    ['/ws/.env', 'go', '/ws'],
    ['~/.ssh/id_ed25519', 'go', '/ws'],
    ['/ws/cmd/main.go', 'go', ''],
    ['', 'go', '/ws'],
  ] as const;

  for (const [file, lang, root] of rejected) {
    assert.equal(shouldAttachIdeLsp(file, lang, root), false, `expected LSP disabled for ${file} (${root})`);
  }
});

test('resolveIdeLspModuleRoot scopes multi-repo workspaces to deepest matching sub-repo and resolves absolute paths', () => {
  const folders = Array.from({ length: 50 }, (_, i) => `/ws/parent/service-${i + 1}`);
  folders.push('/ws/parent/service-12/nested-mod');

  assert.equal(
    toAbsoluteIdeFilePath('/ws/parent', 'service-12/internal/handler.go'),
    '/ws/parent/service-12/internal/handler.go',
  );
  assert.equal(
    resolveIdeLspModuleRoot('/ws/parent', 'service-12/internal/handler.go', folders),
    '/ws/parent/service-12',
  );
  assert.equal(
    resolveIdeLspModuleRoot('/ws/parent', '/ws/parent/service-12/nested-mod/pkg/a.go', folders),
    '/ws/parent/service-12/nested-mod',
  );
  assert.equal(
    resolveIdeLspModuleRoot('/ws/parent', 'README_ROOT.go', folders),
    '/ws/parent',
  );

  const goOpts = buildLspInitializationOptions('go');
  assert.ok(goOpts);
  assert.equal(goOpts.completeUnimported, true);
  assert.equal(goOpts.deepCompletion, true);
  assert.ok(Array.isArray(goOpts.directoryFilters));
});

test('completion trigger detection, comment/string guard, additionalTextEdits auto-import, and semantic token decoding', () => {
  const goSrc = [
    'package main',
    '',
    'func main() {',
    '\tfmt.',
    '\t// comment.',
    '}',
  ].join('\n');
  const state = EditorState.create({ doc: goSrc });
  const dotPos = goSrc.indexOf('fmt.') + 4;
  const trig = detectLspCompletionTrigger(state.doc, dotPos, false, 'go');
  assert.deepEqual(trig, {
    from: dotPos,
    context: { triggerKind: 2, triggerCharacter: '.' },
  });
  assert.equal(isPosInCommentOrString(goSrc, dotPos), false);

  const commentDotPos = goSrc.indexOf('comment.') + 8;
  assert.equal(isPosInCommentOrString(goSrc, commentDotPos), true);

  const spec = buildLspCompletionChangeSpec(
    state.doc,
    dotPos,
    dotPos,
    'Println()',
    { start: { line: 3, character: 5 }, end: { line: 3, character: 5 } },
    [
      {
        range: { start: { line: 1, character: 0 }, end: { line: 1, character: 0 } },
        newText: '\nimport "fmt"\n',
      },
    ],
  );
  const updated = state.update({ changes: spec.changes, selection: { anchor: spec.cursorPos } }).state;
  const finalText = updated.doc.toString();
  assert.ok(finalText.includes('import "fmt"'));
  assert.ok(finalText.includes('\tfmt.Println()'));
  assert.equal(updated.selection.main.head, finalText.indexOf('fmt.Println()') + 'fmt.Println()'.length);

  // Relative 5-tuple semantic token array: line 2 `func` (keyword=15) + `main` (function=12 + declaration bit 0 = 1).
  const decoded = decodeLspSemanticTokens(state.doc, [
    2, 0, 4, 15, 0,
    0, 5, 4, 12, 1,
  ]);
  assert.deepEqual(decoded, [
    { from: state.doc.line(3).from, to: state.doc.line(3).from + 4, className: 'cm-syn-keyword' },
    { from: state.doc.line(3).from + 5, to: state.doc.line(3).from + 9, className: 'cm-syn-func-decl' },
  ]);
});

test('smart indentation, minimal external-format cursor preservation, and 300-entry sessionStorage view-position LRU', () => {
  assert.equal(useTabsForLang('cmd/main.go', 'go'), true);
  assert.equal(useTabsForLang('Makefile'), true);
  assert.equal(useTabsForLang('src/app.ts', 'typescript'), false);
  assert.equal(resolveIndentUnit('cmd/main.go', 'go', 4), '\t');
  assert.equal(resolveIndentUnit('src/app.ts', 'typescript', 2), '  ');

  const blockDoc = 'func Run() {}';
  const betweenBraces = blockDoc.indexOf('{}') + 1;
  const splitEdit = computeSmartEnterEdit(blockDoc, betweenBraces, '\t', 'go');
  const splitApplied =
    blockDoc.slice(0, splitEdit.from) + splitEdit.insert + blockDoc.slice(splitEdit.to);
  assert.equal(splitApplied, 'func Run() {\n\t\n}');
  assert.equal(splitEdit.cursorPos, 'func Run() {\n\t'.length);

  const pyDoc = 'def handle(req):';
  const pyEdit = computeSmartEnterEdit(pyDoc, pyDoc.length, '    ', 'python');
  assert.equal(pyEdit.insert, '\n    ');

  const unformatted = 'package main\nfunc main() {\n\tx:=1\n\tprintln(x)\n}\n';
  const formatted = 'package main\n\nfunc main() {\n\tx := 1\n\tprintln(x)\n}\n';
  const cursorBefore = unformatted.indexOf('println') + 3;
  const minDiff = computeMinimalTextUpdate(unformatted, formatted, cursorBefore, cursorBefore);
  assert.ok(minDiff);
  assert.equal(minDiff.head, formatted.indexOf('println') + 3);

  const sessionMem = createMemoryStorage();
  clearIdeViewPositions(sessionMem);
  for (let i = 0; i < MAX_IDE_VIEW_POSITIONS + 10; i++) {
    saveIdeViewPosition(
      '/ws/repo',
      `pkg/file_${i}.go`,
      { anchor: i * 10, head: i * 10 + 2, scrollTop: i * 25 },
      { immediate: true, storage: sessionMem },
    );
  }
  assert.equal(getIdeViewPosition('/ws/repo', 'pkg/file_0.go', sessionMem), null, 'oldest beyond 300 evicted');
  assert.deepEqual(getIdeViewPosition('/ws/repo', `pkg/file_${MAX_IDE_VIEW_POSITIONS + 9}.go`, sessionMem), {
    anchor: (MAX_IDE_VIEW_POSITIONS + 9) * 10,
    head: (MAX_IDE_VIEW_POSITIONS + 9) * 10 + 2,
    scrollTop: (MAX_IDE_VIEW_POSITIONS + 9) * 25,
  });

  saveIdeViewPosition('/ws/repo', '.env', { anchor: 5, head: 5, scrollTop: 10 }, { immediate: true, storage: sessionMem });
  assert.equal(getIdeViewPosition('/ws/repo', '.env', sessionMem), null);
  clearIdeViewPositions(sessionMem);
});
