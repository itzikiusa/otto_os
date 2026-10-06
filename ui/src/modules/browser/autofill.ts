// Pure helpers behind the live-tab credential autofill (BrowserView). Kept
// DOM-free so `unit/browserAutofill.test.ts` can drive them under node:test.

export function hostOf(url: string): string | null {
  try {
    return new URL(url).hostname || null;
  } catch {
    return null;
  }
}

/** Mirrors `otto_state::browser_credentials::match_domain` (exact host, or
 *  any subdomain of a stored domain). Client-side only — a mismatch here
 *  just hides the key icon; the server independently re-derives its own
 *  match for the agent-facing `/browser/login` route. */
export function matchDomain(host: string, domain: string): boolean {
  const h = host.trim().replace(/\.$/, '').toLowerCase();
  const d = domain.trim().replace(/\.$/, '').toLowerCase();
  if (!d) return false;
  return h === d || h.endsWith(`.${d}`);
}

/** Only https pages (or plain-http loopback) may receive a password. */
export function fillAllowedForUrl(url: string): boolean {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return false;
  }
  if (u.protocol === 'https:') return true;
  if (u.protocol === 'http:') {
    return u.hostname === 'localhost' || u.hostname === '127.0.0.1' || u.hostname === '::1';
  }
  return false;
}

/** May `cred` be filled into the page at `url` right now? Re-checked after
 *  every await in `autofill()` — the tab can redirect (SSO hop, open
 *  redirect) while the confirmation is open. */
export function fillTargetOk(url: string | undefined, domain: string): boolean {
  if (!url || !fillAllowedForUrl(url)) return false;
  const host = hostOf(url);
  return !!host && matchDomain(host, domain);
}

/** The injected fill script. It re-checks the page's OWN origin before it
 *  touches a field — the last line of defence if the tab navigated between
 *  the host-side check and the eval — and returns 'origin-changed'.
 *
 *  The check runs inside a page the user doesn't control, so it uses no
 *  page-overridable built-ins (`String`, `.replace`, `.slice`,
 *  `.toLowerCase` — a hostile page can redefine all of them to make the
 *  comparison pass; S18-307): only `location` (unforgeable), string
 *  indexing, `.length` and `===`. `location.hostname` is already lower-case
 *  ASCII (the URL parser folds it). The host-side `fillTargetOk` re-check
 *  after every await stays the primary guard. */
export function buildFillScript(domain: string, username: string, password: string): string {
  const d = JSON.stringify(domain.trim().replace(/\.$/, '').toLowerCase());
  const userJs = JSON.stringify(username);
  const passJs = JSON.stringify(password);
  return (
    '(function(){' +
    `var D = ${d};` +
    'var h = location.hostname, p = location.protocol;' +
    "if (typeof h !== 'string' || typeof p !== 'string') return 'origin-changed';" +
    // n = host length without a trailing dot.
    "var n = h.length; if (n > 0 && h[n - 1] === '.') n = n - 1;" +
    // tailIs(off): h[off .. off+|D|) is exactly D, compared index-wise.
    'function tailIs(off) { for (var i = 0; i < D.length; i++) { if (h[off + i] !== D[i]) return false; } return true; }' +
    'var m = D.length > 0 && (n === D.length ? tailIs(0) : (n > D.length && h[n - D.length - 1] === \'.\' && tailIs(n - D.length)));' +
    "if (!m) return 'origin-changed';" +
    "var loop = false; var L = ['localhost', '127.0.0.1', '[::1]', '::1'];" +
    'for (var k = 0; k < 4; k++) { var c = L[k]; if (c.length === n) { var same = true; for (var j = 0; j < n; j++) { if (h[j] !== c[j]) { same = false; break; } } if (same) loop = true; } }' +
    "if (!(p === 'https:' || (p === 'http:' && loop))) return 'origin-changed';" +
    'var pwd = document.querySelector(\'input[type="password"]\');' +
    "if (!pwd) return 'no-password-field';" +
    'var user = document.querySelector(\'input[type="email"]\') || ' +
    'document.querySelector(\'input[autocomplete="username"]\') || ' +
    'document.querySelector(\'input[type="text"]\');' +
    'if (user) {' +
    'user.focus();' +
    `user.value = ${userJs};` +
    "user.dispatchEvent(new Event('input', {bubbles: true}));" +
    "user.dispatchEvent(new Event('change', {bubbles: true}));" +
    '}' +
    'pwd.focus();' +
    `pwd.value = ${passJs};` +
    "pwd.dispatchEvent(new Event('input', {bubbles: true}));" +
    "pwd.dispatchEvent(new Event('change', {bubbles: true}));" +
    "return 'filled';" +
    '})()'
  );
}

/** Normalise `nativeBrowser.eval`'s result (raw or JSON-quoted string). */
export function fillResult(raw: unknown): string {
  const s = String(raw ?? '');
  return s.replace(/^['"]|['"]$/g, '');
}
