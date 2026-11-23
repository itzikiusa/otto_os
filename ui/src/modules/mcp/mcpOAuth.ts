// OAuth sign-in for a governed remote MCP server. The daemon discovers the
// authorization server, registers Otto (or takes the client id the person
// types) and listens on this Mac's loopback for the browser's answer; this
// module confirms where the browser goes BEFORE opening it, opens it, and
// watches the sign-in finish. Tokens stay in the Keychain: the UI never sees them.
import { mcpCpApi } from '../../lib/api/mcp';
import type { McpOAuthStatus, McpServerDetail } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { openExternal } from '../../lib/external';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';

const NO_REGISTRATION = 'enter a client id';
const POLL_MS = 2000;
const WINDOW_MS = 5 * 60 * 1000;

/** Sign in: start → confirm the server and its authorization host → browser →
 *  wait for the flow (≤ 5 min). Resolves true once signed in. */
export async function signIn(s: McpServerDetail): Promise<boolean> {
  let started: { authorization_url: string; authorization_host: string };
  try {
    started = await mcpCpApi.cpOAuthStart(s.id, {});
  } catch (e) {
    if (!String((e as Error)?.message ?? e).includes(NO_REGISTRATION)) {
      toastError('Couldn’t start the sign-in', e);
      return false;
    }
    // No dynamic registration: the person names Otto’s client at that server.
    const id = await confirmer.promptText(`${s.name}’s authorization server doesn’t register apps itself. Enter the client ID you created for Otto there.`, { title: 'Client ID', placeholder: 'client id' });
    if (!id) return false;
    const secret = await confirmer.promptText('Client secret, if the app has one. Leave empty for a public app.', { title: 'Client secret', allowEmpty: true });
    if (secret === null) return false;
    try {
      started = await mcpCpApi.cpOAuthStart(s.id, { client_id: id, client_secret: secret || undefined });
    } catch (e2) {
      toastError('Couldn’t start the sign-in', e2);
      return false;
    }
  }
  const ok = await confirmer.ask(
    `Sign in to ${s.name}? Your browser opens ${started.authorization_host} to sign in. Otto keeps the tokens in the Keychain; agents never see them.`,
    { title: 'Sign in', confirmLabel: 'Open browser', danger: false },
  );
  if (!ok) return false;
  await openExternal(started.authorization_url);
  const until = Date.now() + WINDOW_MS;
  let st: McpOAuthStatus = { signed_in: false, pending: true };
  while (Date.now() < until) {
    await new Promise((r) => setTimeout(r, POLL_MS));
    try {
      st = await mcpCpApi.cpOAuthStatus(s.id);
    } catch {
      continue;
    }
    if (!st.pending) break;
  }
  if (st.signed_in && !st.error) {
    toasts.success('Signed in', s.name);
    return true;
  }
  toasts.warn('Sign-in didn’t finish', st.error ?? 'The browser didn’t come back in time.');
  return false;
}

export async function signOut(s: McpServerDetail): Promise<boolean> {
  const ok = await confirmer.ask(`Sign out of ${s.name}? Otto deletes its tokens from the Keychain; calls to it need a new sign-in.`, { title: 'Sign out', confirmLabel: 'Sign out', danger: false });
  if (!ok) return false;
  try {
    await mcpCpApi.cpOAuthSignOut(s.id);
    toasts.success('Signed out', s.name);
    return true;
  } catch (e) {
    toastError('Couldn’t sign out', e);
    return false;
  }
}
