// Which Changes row a turn's changes card asked for (threads.changesFocus).
// The card's path is relative to the thread's folder, a Changes row's to the
// repo: a thread working in `repo/ui` asks for `src/a.ts`, the row is
// `ui/src/a.ts` — either may be the longer one. Pure (unit/threadsChangesFocus.test.ts).
/** Whether the ask is done: shown, or the list for THIS thread's repo has
 *  loaded and the file is not in it. Rows still loading, or left over from
 *  another repo, never consume it. */
export function focusSettled(o: { hit: boolean; loading: boolean; loadedFor: string | null; repoId: string | null }): boolean {
  return o.hit || (!o.loading && o.repoId !== null && o.loadedFor === o.repoId);
}

export function focusRow<R extends { path: string }>(rows: readonly R[], want: string | null): R | null {
  if (!want) return null;
  return rows.find((r) => want === r.path || want.endsWith(`/${r.path}`) || r.path.endsWith(`/${want}`)) ?? null;
}
