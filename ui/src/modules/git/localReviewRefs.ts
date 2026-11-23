// Pure branch-option helpers for LocalReviewPanel.svelte.
import type { RefsResp } from '../../lib/api/types';

export interface LocalBranchOption {
  name: string;
  value: string;
  label: string;
}

const PREFERRED_BASES = ['origin/develop', 'origin/main', 'origin/master', 'develop', 'main', 'master'];

/** Build unambiguous branch options when a local branch shares a name with a remote ref. */
export function buildBranchOptions(refs: RefsResp | null): LocalBranchOption[] {
  if (!refs) return [];
  const locals = new Set(refs.local.map((b) => b.name));
  const remotes = new Set(refs.remote.map((b) => b.name));
  return [
    ...refs.remote.map((b) => ({
      name: b.name,
      value: locals.has(b.name) ? `refs/remotes/${b.name}` : b.name,
      label: locals.has(b.name) ? `${b.name} (remote)` : b.name,
    })),
    ...refs.local.map((b) => ({
      name: b.name,
      value: remotes.has(b.name) ? `refs/heads/${b.name}` : b.name,
      label: remotes.has(b.name) ? `${b.name} (local)` : b.name,
    })),
  ];
}

/** Pick the default comparison base branch from refs + options. */
export function pickDefaultBase(refs: RefsResp | null, options: LocalBranchOption[]): string {
  const allNames = [...(refs?.remote ?? []).map((r) => r.name), ...(refs?.local ?? []).map((l) => l.name)];
  const hit = PREFERRED_BASES.find((p) => allNames.includes(p));
  return options.find((b) => b.name === hit)?.value ?? options[0]?.value ?? '';
}
