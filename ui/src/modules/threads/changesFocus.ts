// Which Changes row a turn's changes card asked for (threads.changesFocus).
// The card's path is relative to the thread's folder, a Changes row's to the
// repo: a thread working in `repo/ui` asks for `src/a.ts`, the row is
// `ui/src/a.ts` — either may be the longer one. Pure (unit/threadsChangesFocus.test.ts).
export function focusRow<R extends { path: string }>(rows: readonly R[], want: string | null): R | null {
  if (!want) return null;
  return rows.find((r) => want === r.path || want.endsWith(`/${r.path}`) || r.path.endsWith(`/${want}`)) ?? null;
}
