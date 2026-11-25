// When the editor re-reads a file's HEAD copy (`/ide/fs/git-base`, the change bars' base),
// keyed by the git status digest of the file (`ideGit.fileStatusDigest`).

/** A digest from the git state's first load (module not loaded, repo not found yet, status not
 *  read yet): reaching the real digest from one of these does not move HEAD. */
export function isSettlingDigest(d: string | undefined): boolean {
  return d === undefined || d === 'pending' || d.startsWith('norepo:') || d.includes(':nostatus:');
}

/** Read the HEAD copy again? Not while one is held (or being read) and only the first load
 *  moved the digest; any later change (commit, checkout, stash, pull) re-reads. */
export function needsGitBaseRead(prev: string | undefined, digest: string, held: boolean): boolean {
  return !held || (prev !== digest && !isSettlingDigest(prev));
}

/** The digest a finished read stands for: one started during the first load stands for the
 *  digest that load reached meanwhile (else the next real change would look like settling). */
export function settledGitBaseDigest(started: string, now: string): string {
  return isSettlingDigest(started) ? now : started;
}
