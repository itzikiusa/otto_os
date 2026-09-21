import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { scanTerminalLinks, resolveTerminalFile, oscTerminalLink, terminalLinksForRow } from '../src/lib/components/terminalLinks.ts';

test('relative paths, source basenames and locations from actual CLI output', () => {
  const hits = scanTerminalLinks('Read materialize.rs and ui/src/App.svelte:42:3 then ../Cargo.toml#L8');
  assert.deepEqual(hits.map(h => [h.path, h.line, h.col]), [['materialize.rs', undefined, undefined], ['ui/src/App.svelte', 42, 3], ['../Cargo.toml', 8, undefined]]);
  assert.equal(resolveTerminalFile('../Cargo.toml', '/work/ui'), '/work/ui/../Cargo.toml');
  assert.equal(resolveTerminalFile('app.ts', ''), null);
});

test('quoted and markdown paths with spaces retain full destinations', () => {
  const hits = scanTerminalLinks('Read "/Users/me/Application Support/test.md:4" and [report](/tmp/my report.txt#L9).');
  assert.deepEqual(hits.map(h => [h.path, h.line]), [['/Users/me/Application Support/test.md', 4], ['/tmp/my report.txt', 9]]);
});

test('URL overlap, punctuation, task names, hosts and command schemes', () => {
  const hits = scanTerminalLinks('https://example.com/a.ts:4?x=1. /root/git_fetch example.com:8080 javascript:alert(1) src/view.ts');
  assert.deepEqual(hits.map(h => [h.kind, h.text]), [['url', 'https://example.com/a.ts:4?x=1'], ['file', 'src/view.ts']]);
  assert.deepEqual(scanTerminalLinks('https://example.com src/view.ts', false).map(h => h.kind), ['url']);
});

test('OSC8 dispatch permits only HTTP(S) and local file targets', () => {
  assert.equal(oscTerminalLink('file:///tmp/Application%20Support/readme.md#L3', '/work', true)?.path, '/tmp/Application Support/readme.md');
  assert.equal(oscTerminalLink('file://remote/tmp/a.ts', '/work', true), null);
  assert.equal(oscTerminalLink('javascript:alert(1)', '/work', true), null);
  assert.equal(oscTerminalLink('file:///tmp/a.ts', '/work', false), null);
  assert.equal(oscTerminalLink('https://example.com', '/work', false)?.kind, 'url');
});

function buffer(rows: { text: string; wrapped?: boolean; color?: number; bold?: boolean }[], cols: number) {
  return { length: rows.length, getLine(index: number) {
    const row = rows[index]; if (!row) return undefined;
    const cells: { getChars(): string; getWidth(): number; getFgColorMode?(): number; getFgColor?(): number; isBold?(): number }[] = [];
    for (const char of row.text) {
      const width = char === '界' || char === '😀' ? 2 : 1;
      cells.push({ getChars: () => char, getWidth: () => width, getFgColorMode: () => row.color ? 1 : 0, getFgColor: () => row.color ?? 0, isBold: () => row.bold ? 1 : 0 });
      if (width === 2) cells.push({ getChars: () => '', getWidth: () => 0 });
    }
    while (cells.length < cols) cells.push({ getChars: () => '', getWidth: () => 1 });
    return { isWrapped: row.wrapped ?? false, length: cols, getCell: (x: number) => cells[x] };
  }};
}

test('wrapped logical paths map inclusive ranges across terminal rows', () => {
  const b = buffer([{ text: 'Read ui/src/' }, { text: 'App.ts:12', wrapped: true }], 12);
  const hits = terminalLinksForRow(b, 2, true);
  assert.equal(hits[0].text, 'ui/src/App.ts:12');
  assert.deepEqual(hits[0].range, { start: { x: 6, y: 1 }, end: { x: 9, y: 2 } });
});

test('wide Unicode and surrogate pairs do not shift file ranges', () => {
  const hits = terminalLinksForRow(buffer([{ text: '界😀 src/a.ts' }], 25), 1, true);
  assert.deepEqual(hits[0].range, { start: { x: 6, y: 1 }, end: { x: 13, y: 1 } });
});

test('dot segments preserve filesystem symlink semantics until daemon canonicalization', () => {
  const root = mkdtempSync(join(tmpdir(), 'otto-terminal-links-'));
  try {
    mkdirSync(join(root, 'real', 'deep'), { recursive: true });
    writeFileSync(join(root, 'real', 'target.ts'), 'synthetic fixture');
    symlinkSync(join(root, 'real', 'deep'), join(root, 'link'));
    const requested = resolveTerminalFile('link/../target.ts', root)!;
    assert.equal(readFileSync(requested, 'utf8'), 'synthetic fixture');
    const osc = oscTerminalLink(`file://${root}/link/../target.ts`, root, true)!;
    assert.equal(readFileSync(osc.path!, 'utf8'), 'synthetic fixture');
    const absolute = join(root, 'link') + '/../target.ts';
    assert.equal(resolveTerminalFile(absolute, '/unrelated'), absolute);
  } finally { rmSync(root, { recursive: true, force: true }); }
});


test('unquoted rooted path with spaces wins over a misleading relative suffix', () => {
  const path = '/Users/itziklavon/Library/Application Support/Otto/snips/01M311BBDY6KH1PAE81J9PTTY4.png';
  assert.deepEqual(scanTerminalLinks(`  ${path}`).map(h => h.path), [path]);
  assert.deepEqual(scanTerminalLinks('/tmp/report.md and src/foo.ts').map(h => h.path), ['/tmp/report.md', 'src/foo.ts']);
  assert.deepEqual(scanTerminalLinks('~/Application Support/a.json, ./my folder/b.ts').map(h => h.path), ['~/Application Support/a.json', './my folder/b.ts']);
});

test('Codex parenthesized cyan hardwrap retains whole target on either row without linking indent', () => {
  const first = 'installation status (/Users/itziklavon/Library/Logs/Otto/uncommitted-20260921-';
  const b = buffer([{ text: first, color: 6 }, { text: '  corrections/status.md).', color: 6 }], 100);
  for (const row of [1, 2]) {
    const hits = terminalLinksForRow(b, row, true);
    assert.equal(hits.length, 1);
    assert.equal(hits[0].path, '/Users/itziklavon/Library/Logs/Otto/uncommitted-20260921-corrections/status.md');
    assert.equal(hits[0].range.start.y, row);
    assert.equal(hits[0].range.end.y, row);
  }
  assert.equal(terminalLinksForRow(b, 2, true)[0].range.start.x, 3);
  assert.equal(terminalLinksForRow(b, 2, true)[0].range.end.x, 23);
});

test('Claude bold edge-filled relative path joins slash continuation from cursor-addressed next row', () => {
  const prefix = '../../../private/tmp/claude-501/-Users-itziklavon-games-management';
  const first = `Referenced file ${prefix}`;
  const b = buffer([{ text: first, bold: true }, { text: '     /9cb793be/scratchpad/ui-design-spec.md', bold: true }], first.length);
  for (const row of [1, 2]) assert.equal(terminalLinksForRow(b, row, true)[0].path, `${prefix}/9cb793be/scratchpad/ui-design-spec.md`);
  assert.equal(terminalLinksForRow(b, 2, true)[0].range.start.x, 6);
});

test('hardwrap recognition does not combine separate files, styles, or unrelated prose', () => {
  const first = 'Referenced file /tmp/incomplete-';
  for (const rows of [
    [{ text: first, color: 6 }, { text: '  folder/report.md', color: 2 }],
    [{ text: first }, { text: '  folder/report.md' }],
    [{ text: 'Read /tmp/first.md', color: 6 }, { text: '  /other/second.md', color: 6 }],
    [{ text: first, color: 6 }, { text: '  now read folder/report.md', color: 6 }],
  ]) {
    const hits = terminalLinksForRow(buffer(rows, first.length), 2, true);
    assert.ok(hits.every(h => !h.path?.includes('incomplete-') && !h.path?.includes('first.md/')));
  }
});


test('rooted spaced paths stop at sentence punctuation and a subsequent explicit root', () => {
  assert.deepEqual(scanTerminalLinks('/Users/me/Application Support/report.md.').map(h => h.path), ['/Users/me/Application Support/report.md']);
  assert.deepEqual(scanTerminalLinks('/tmp/report.md. Next /tmp/other.ts').map(h => h.path), ['/tmp/report.md', '/tmp/other.ts']);
  assert.deepEqual(scanTerminalLinks('/tmp/cache /other/report.md').map(h => h.path), ['/other/report.md']);
});

test('mixed soft and hard wraps preserve the original absolute prefix on all three rows', () => {
  const b = buffer([{ text: 'See (/Users/name/Library/Log' }, { text: 's/Otto/uncommitted-', wrapped: true }, { text: '  corrections/status.md).' }], 28);
  for (const row of [1, 2, 3]) {
    const hits = terminalLinksForRow(b, row, true);
    assert.equal(hits.length, 1);
    assert.equal(hits[0].path, '/Users/name/Library/Logs/Otto/uncommitted-corrections/status.md');
    assert.equal(hits[0].range.start.y, row);
    assert.equal(hits[0].range.end.y, row);
  }
});


test('painted trailing spaces do not make an incomplete path edge-filled', () => {
  const text = 'Read /tmp/incomplete-' + ' '.repeat(40);
  const hits = terminalLinksForRow(buffer([{text, color:6}, {text:'  folder/report.md',color:6}], text.length), 2, true);
  assert.deepEqual(hits.map(h => h.path), ['folder/report.md']);
});
