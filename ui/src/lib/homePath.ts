/** A path under the user's home (`/Users/<name>` or `/home/<name>`) shown as `~/…`. */
export function shortenHomePath(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~');
}
