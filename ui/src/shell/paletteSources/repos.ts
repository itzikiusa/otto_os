// ⌘K "Open repository <name>" commands (architecture-2-05). Kept beside the
// shell's palette sources rather than in modules/git for now; pure, so the
// source effect only re-registers when the repo list changes.

import type { Command } from '../../lib/commands.svelte';
import type { Repo } from '../../lib/api/types';

export function repoCommands(repos: readonly Repo[], go: (route: string) => void): Command[] {
  return repos.map((r) => ({
    id: `repo.${r.id}`,
    title: `Open repository ${r.name}`,
    group: 'Git',
    keywords: `repository ${r.path}`,
    run: () => go(`git/${r.id}`),
  }));
}
