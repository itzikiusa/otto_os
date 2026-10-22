// What an AWS view stores for a failed load. The views render it through
// `awsErrorText` and test it with `isLoginRequired(new Error(text))` (the
// Sign in action), so the daemon's "login required…" text is kept verbatim;
// every other failure reads as `providerErrorText` — a provider 403 keeps its
// actionable text, a dropped daemon is "Otto can’t reach the daemon." instead
// of WebKit's bare "Load failed", and a daemon 401/404 gets the human line.

import { isLoginRequired } from '../../lib/api/aws';
import { providerErrorText } from '../../lib/loadError';

export function awsLoadText(e: unknown): string {
  return isLoginRequired(e) ? (e as Error).message : providerErrorText(e);
}
