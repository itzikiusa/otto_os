import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  SETTINGS_SECTIONS,
  availableSections,
  canOpenSection,
  filterSections,
  findSection,
  sectionLabel,
  type SettingsAccess,
} from '../src/modules/settings/sections.ts';

const member: SettingsAccess = { can: () => false, isRoot: false };
const root: SettingsAccess = { can: () => true, isRoot: true };
// A users-admin who isn't root and has no settings:admin.
const usersAdmin: SettingsAccess = {
  can: (f, level) => f === 'users' && level === 'admin',
  isRoot: false,
};

test('section ids are unique and labels are sentence case', () => {
  const ids = SETTINGS_SECTIONS.map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length);
  // Sentence case: no word after the first starts upper-case unless it is a
  // proper noun / acronym on the allow-list.
  const proper = new Set(['MCP', 'Jira', 'Git']);
  for (const s of SETTINGS_SECTIONS) {
    const [, ...rest] = s.label.split(/\s+/);
    for (const w of rest) {
      if (/^[A-Z]/.test(w)) assert.ok(proper.has(w), `"${s.label}" is not sentence case (${w})`);
    }
  }
});

test('access gates: ungated for members, admin sections for settings admins, root-only groups', () => {
  const memberIds = availableSections(member).map((s) => s.id);
  assert.ok(memberIds.includes('appearance'));
  assert.ok(memberIds.includes('jira'));
  assert.ok(!memberIds.includes('daemon'));
  assert.ok(!memberIds.includes('browser'));
  assert.ok(!memberIds.includes('access-groups'));

  const ua = availableSections(usersAdmin).map((s) => s.id);
  assert.ok(ua.includes('users') && ua.includes('sessions'));
  assert.ok(!ua.includes('access-groups'), 'groups are root-only even for users admins');

  assert.equal(availableSections(root).length, SETTINGS_SECTIONS.length);
  assert.equal(canOpenSection(findSection('access-groups')!, root), true);
});

test('filter matches label, keywords and group, ranking label prefixes first', () => {
  const all = availableSections(root);
  assert.equal(filterSections(all, '').length, all.length);
  assert.equal(filterSections(all, 'jira')[0].id, 'jira');
  // Keyword hit: "pat" finds the tokens page.
  assert.ok(filterSections(all, 'pat').some((s) => s.id === 'tokens'));
  assert.deepEqual(filterSections(all, 'gmail').map((s) => s.id), ['sharing']);
  // Multi-term AND, case-insensitive.
  assert.deepEqual(
    filterSections(all, 'SLACK telegram').map((s) => s.id),
    ['channels'],
  );
  assert.deepEqual(filterSections(all, 'zzz-nothing'), []);
  // Filtering never reveals a section the caller can't open.
  assert.ok(!filterSections(availableSections(member), 'daemon').some((s) => s.id === 'daemon'));
});

test('every section page titles itself from the registry', () => {
  const base = new URL('../src/modules/', import.meta.url);
  const settings = readFileSync(new URL('settings/Settings.svelte', base), 'utf8');
  for (const s of SETTINGS_SECTIONS) {
    // The component mapped to this id in Settings.svelte's VIEWS table…
    const key = s.id.includes('-') ? `'${s.id}'` : s.id;
    // Sections load lazily (`id: lazyComponent(() => import('./X.svelte'))`);
    // the default one wraps a static import (`{ default: Appearance }`).
    const m = settings.match(new RegExp(`\\n\\s*${key}: lazyComponent\\((.*)\\),\\n`));
    assert.ok(m, `no view mapped for ${s.id}`);
    let path = m[1].match(/import\('(.+?)'\)/)?.[1];
    const local = m[1].match(/default: (\w+)/)?.[1];
    if (!path && local) path = settings.match(new RegExp(`import ${local} from '(.+?)';`))?.[1];
    assert.ok(path, `no import for ${s.id}`);
    const src = readFileSync(new URL(path, new URL('settings/', base)), 'utf8');
    // …renders a PageHeader whose title is the registry label.
    assert.ok(
      src.includes(`title={sectionLabel('${s.id}')}`),
      `${path} does not title its PageHeader with sectionLabel('${s.id}')`,
    );
  }
  assert.equal(sectionLabel('tokens'), 'Personal access tokens');
});

// S17-14: these sections only call root-only handlers (`GET|PUT /settings`,
// `/audit-log`, `/security-posture`, `/logs/daemon` → `require_root`). A
// non-root settings admin must not get nav entries that can only 403.
test('sections backed by root-only handlers are gated on root, not settings:admin', () => {
  const settingsAdmin: SettingsAccess = { can: (f, level) => f === 'settings' && level === 'admin', isRoot: false };
  const ids = availableSections(settingsAdmin).map((s) => s.id);
  for (const id of ['providers', 'daemon', 'trust-safety', 'logs']) {
    assert.equal(findSection(id)!.gate, 'root', `${id} must be root-gated`);
    assert.ok(!ids.includes(id as never), `${id} shown to a non-root settings admin`);
  }
  // Sections whose handlers accept settings admins stay open to them.
  assert.ok(ids.includes('skills'));
});

// S17-303: a section's UI gate must match its handlers' real gate. These
// sections' handlers are all `require_root`; gating them `settings:admin`
// showed a non-root settings admin a page where every action 403'd.
const ROOT_ONLY_HANDLERS: Record<string, string[]> = {
  plugins: ['crates/otto-server/src/plugins.rs'],
  backup: [
    'crates/otto-server/src/routes/backup.rs',
    'crates/otto-server/src/routes/backup_git.rs',
    'crates/otto-server/src/routes/connection_export.rs',
  ],
};

test('root-only handlers ↔ root-gated sections (S17-303)', () => {
  const settingsAdmin: SettingsAccess = { can: (f, level) => f === 'settings' && level === 'admin', isRoot: false };
  const visible = availableSections(settingsAdmin).map((s) => s.id);
  for (const [id, files] of Object.entries(ROOT_ONLY_HANDLERS)) {
    const section = findSection(id);
    assert.ok(section, id);
    assert.equal((section as { gate?: unknown }).gate, 'root', `${id} must be gated 'root'`);
    assert.ok(!visible.includes(id as never), `a non-root settings admin must not see ${id}`);
    for (const f of files) {
      const src = readFileSync(new URL(`../../${f}`, import.meta.url), 'utf8');
      assert.match(src, /require_root\(/, `${f} is root-gated`);
    }
  }
});
