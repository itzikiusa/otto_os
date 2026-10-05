// The `?` shortcut sheet (KEYMAP in lib/keys.ts) and the Help guide's tables
// (modules/help/sections/keyboard-shortcuts.md) list the same chords: same
// groups, same rows, same order. A chord added to one and not the other — or a
// label edited in one place — fails here instead of drifting.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { KEYMAP } from '../src/lib/keys.ts';

const md = readFileSync(new URL('../src/modules/help/sections/keyboard-shortcuts.md', import.meta.url), 'utf8');

/** `**Group**` + a `| Keys | Action |` table → { category, rows }. Keys cells
 *  drop their backticks and write ranges (`⌃1` – `⌃4`) as KEYMAP does (⌃1…⌃4). */
function helpTables(): { category: string; rows: string[] }[] {
  const start = md.indexOf('## Keyboard shortcuts');
  const end = md.indexOf('\n## ', start + 1);
  assert.ok(start >= 0, 'the guide has a "## Keyboard shortcuts" section');
  const section = md.slice(start, end < 0 ? undefined : end);
  const out: { category: string; rows: string[] }[] = [];
  for (const m of section.matchAll(/\*\*(.+?)\*\*\n\n\| Keys \| Action \|\n\|---\|---\|\n((?:\|.*\|\n?)+)/g)) {
    const rows = m[2]
      .trim()
      .split('\n')
      .map((line) => {
        const [keys, action] = line.replace(/^\||\|$/g, '').split(' | ').map((c) => c.trim());
        return `${keys.replace(/` – `/g, '…').replace(/`/g, '')} → ${action.replace(/`/g, '')}`;
      });
    out.push({ category: m[1], rows });
  }
  return out;
}

const sheet = KEYMAP.map((g) => ({ category: g.category, rows: g.bindings.map((b) => `${b.keys} → ${b.label}`) }));

test('the shortcut sheet and the Help guide have the same groups, in order', () => {
  assert.deepEqual(
    sheet.map((g) => g.category),
    helpTables().map((g) => g.category),
  );
});

test('every group lists the same chords and actions in both places', () => {
  const help = new Map(helpTables().map((g) => [g.category, g.rows]));
  for (const g of sheet) assert.deepEqual(g.rows, help.get(g.category), g.category);
});

test('page-scoped chords that exist in code are on the sheet', () => {
  const all = sheet.flatMap((g) => g.rows).join('\n');
  // ⌘S (Vault note, Skills editor, Design Hall artifact, brand kit), ⌘E (Vault
  // note), ⌘F (Git graph search), Vault ⌘O / ⌘N, Database ⌘B and ⌥⌘V, the
  // live browser's ⌘R, ⌘\, ⌥-click, ⌃⇥ / ⌃⇧⇥ and ⌥↑ / ⌥↓ (sidebar moves).
  for (const chord of ['⌘S →', '⌘E →', '⌘O / ⌘N →', '⌘B →', '⌥⌘V →', '⌘R →', '⌘\\ →', '⌥-click →', '⌃⇥ / ⌃⇧⇥ →', '⌥↑ / ⌥↓ →']) {
    assert.ok(all.includes(chord), `${chord} is on the sheet`);
  }
  assert.ok(/⌘F → In the Git graph/.test(all), 'Git graph ⌘F is on the sheet');
});
