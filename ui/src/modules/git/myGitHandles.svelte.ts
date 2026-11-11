// The signed-in user's handles on the git forges — every connected account's
// username and display name, lower-cased. "Authored by you" in the PR list
// and "your review" on a PR both match people against this set (Bitbucket
// names reviewers by display name, GitHub/GitLab by login). Read once per app
// session; a failed read is an empty set (nothing is "yours", nothing breaks).
import { api } from '../../lib/api/client';
import type { GitAccount } from '../../lib/api/types';

class MyGitHandles {
  handles = $state.raw<ReadonlySet<string>>(new Set());
  #loading: Promise<void> | null = null;

  /** Start the read (idempotent); `handles` fills in when it lands. */
  load(): void {
    this.#loading ??= api
      .get<GitAccount[]>('/git/accounts')
      .then((accs) => {
        const s = new Set<string>();
        for (const a of accs) {
          if (a.username) s.add(a.username.toLowerCase());
          const dn = (a as GitAccount & { display_name?: string | null }).display_name;
          if (dn) s.add(dn.toLowerCase());
        }
        this.handles = s;
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
