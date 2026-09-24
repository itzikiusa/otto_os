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
    const m = settings.match(new RegExp(`\\n\\s*${key}: (\\w+),`));
    assert.ok(m, `no view mapped for ${s.id}`);
    const imp = settings.match(new RegExp(`import ${m[1]} from '(.+?)';`));
    assert.ok(imp, `no import for ${m[1]}`);
    const src = readFileSync(new URL(imp[1], new URL('settings/', base)), 'utf8');
    // …renders a PageHeader whose title is the registry label.
    assert.ok(
      src.includes(`title={sectionLabel('${s.id}')}`),
      `${m[1]} does not title its PageHeader with sectionLabel('${s.id}')`,
    );
  }
  assert.equal(sectionLabel('tokens'), 'Personal access tokens');
});
