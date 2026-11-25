// The IDE's project list keeps one order: the server's (pinned first, then the order the
// projects were added). Nothing on the UI side re-sorts it, so re-saving a project (a rename,
// a folder attached: a newer `updated_at`) or reloading never moves it in the switcher or tabs.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mergeBackendIdeProjects, type NamedIdeProject } from '../src/lib/ideProjectsModel.ts';
import { projectListEntries, projectTabs } from '../src/modules/ide/shell/ideProjectList.ts';

const row = (id: string, updated_at: string) => ({ id, name: id, primary_root: `/w/${id}`, folders: [`/w/${id}`], pinned: true, updated_at });
const asIde = (p: NamedIdeProject) => ({ id: p.id, name: p.name, primary_root: p.primaryRoot, folders: p.folders });

test('a re-saved project keeps its place in the switcher and the tabs', () => {
  const local = mergeBackendIdeProjects([], [row('alpha', '2026-11-01T00:00:00Z'), row('beta', '2026-11-02T00:00:00Z'), row('gamma', '2026-11-03T00:00:00Z')]);
  // alpha was renamed just now: newest updated_at, still first in the server's list.
  const next = mergeBackendIdeProjects(local, [row('alpha', '2026-11-25T00:00:00Z'), row('beta', '2026-11-02T00:00:00Z'), row('gamma', '2026-11-03T00:00:00Z')]);
  assert.deepEqual(next.map((p) => p.id), ['alpha', 'beta', 'gamma']);
  const projects = next.map(asIde);
  assert.deepEqual(projectTabs(projects, 'gamma', false).map((t) => t.id), ['scratches', 'alpha', 'beta', 'gamma']);
  assert.deepEqual(
    projectListEntries(projects, '').map((e) => (e.kind === 'named' ? e.project.id : e.kind)),
    ['scratches', 'alpha', 'beta', 'gamma'],
  );
  // Switching project (another one active) changes nothing either.
  assert.deepEqual(projectTabs(projects, 'alpha', false).map((t) => t.id), ['scratches', 'alpha', 'beta', 'gamma']);
});
