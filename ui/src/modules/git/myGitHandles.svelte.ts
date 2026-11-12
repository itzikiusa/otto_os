// The signed-in user's handles on the git forges — every connected account's
// username and display name, lower-cased. "Authored by you" in the PR list
// and "your review" on a PR both match people against this set (Bitbucket
// names reviewers by display name, GitHub/GitLab by login). This is only the
// fallback: the daemon's `is_me` / `is_mine` flags win wherever they are sent
// (see pr-review-state.ts, pr-query.ts). Read once per app session; a failed
// read is an empty set (nothing is "yours", nothing breaks).
import { api } from '../../lib/api/client';
import type { GitAccount } from '../../lib/api/types';

class MyGitHandles {
  handles = $state.raw<ReadonlySet<string>>(new Set());
  /** How to show you when the optimistic patch adds you as a reviewer: the
   *  first account's display name, else its username (as typed, not lowered). */
  displayName = $state<string | null>(null);
  #loading: Promise<void> | null = null;

  /** Start the read (idempotent); `handles` fills in when it lands. */
  load(): void {
    this.#loading ??= api
      .get<GitAccount[]>('/git/accounts')
      .then((accs) => {
        const s = new Set<string>();
        let name: string | null = null;
        for (const a of accs) {
          if (a.username) s.add(a.username.toLowerCase());
          const dn = (a as GitAccount & { display_name?: string | null }).display_name;
          if (dn) s.add(dn.toLowerCase());
          name ??= dn || a.username || null;
        }
        this.handles = s;
        this.displayName = name;
      })
      .catch(() => {
        this.#loading = null;
      });
  }

  isMe(name: string | null | undefined): boolean {
    return !!name && this.handles.has(name.toLowerCase());
  }
}

export const myGitHandles = new MyGitHandles();
