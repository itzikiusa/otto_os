// Language servers that run the project's own build code — zls (build.zig),
// kotlin-language-server (the Gradle / Maven import), vue-language-server (the plugins
// tsconfig.json names), csharp-ls / fsautocomplete (MSBuild targets) — start only once the
// project is trusted: a non-secret OTTO_IDE_TRUST_PROJECT_EXECUTION=1 (or the per-language
// key) in its saved environment (crates/otto-ide/src/lsp_host/ws_bridge.rs). A browser can't
// read a WebSocket's 403, so "refused for trust" is: the server closed before any session
// (`trust_or_unavailable`) while the saved environment has no opt-in.
// Pure (no store, no runes) so node unit tests run it directly.
import { loadErrorText } from '../../../lib/loadError.ts';
import type { IdeEnvVar } from '../store/ideTypes';
import { IDE_TRUST_PROJECT_ENV, ideLspServerName, type IdeLspRecord } from './ideStatusModel.ts';

export { IDE_TRUST_PROJECT_ENV, ideProjectTrusted } from './ideStatusModel.ts';

const DOTNET = { needs: 'load this project’s MSBuild project, which runs its build targets', runs: 'its MSBuild targets' };
const TRUST_LANGS: Record<string, { name: string; needs: string; runs: string }> = {
  zig: { name: 'Zig', needs: 'run this project’s build.zig', runs: 'its build.zig' },
  kotlin: { name: 'Kotlin', needs: 'import this project’s Gradle or Maven build', runs: 'its Gradle or Maven build scripts' },
  vue: { name: 'Vue', needs: 'load the plugins this project’s tsconfig.json names', runs: 'the plugins its tsconfig.json names' },
  csharp: { name: 'C#', ...DOTNET },
  fsharp: { name: 'F#', ...DOTNET },
};

/** The Project environment switch: the project-wide key only (per-language keys stay rows). */
export function ideProjectTrustOn(vars: readonly IdeEnvVar[]): boolean {
  return vars.some((v) => !v.secret && v.key === IDE_TRUST_PROJECT_ENV && (v.value ?? '').trim() === '1');
}

/** `vars` with the project-wide trust variable set to "1" (in place, or appended) or removed;
 *  every other entry — secrets included — is passed through untouched and in order. */
export function withProjectTrust(vars: readonly IdeEnvVar[], trusted: boolean): IdeEnvVar[] {
  if (!trusted) return vars.filter((v) => v.key !== IDE_TRUST_PROJECT_ENV);
  const entry: IdeEnvVar = { key: IDE_TRUST_PROJECT_ENV, value: '1', secret: false };
  const i = vars.findIndex((v) => v.key === IDE_TRUST_PROJECT_ENV);
  return i < 0 ? [...vars, entry] : vars.map((v, j) => (j === i ? entry : v));
}

export interface IdeTrustPlace { kind: 'local' | 'ssh'; host?: string | null }

/** The one-line notice above the editor, or null when the server wasn't refused for trust. */
export function ideLspTrustNotice(lang: string, rec: IdeLspRecord | null, trusted: boolean | null): string | null {
  const t = TRUST_LANGS[lang];
  if (!t || rec?.phase !== 'unavailable' || rec.reason !== 'trust_or_unavailable' || trusted !== false) return null;
  return `Code completion for ${t.name} is off: ${ideLspServerName(lang)} needs to ${t.needs}.`;
}

/** What "Trust project" grants, for the confirm. */
export function ideLspTrustConfirm(lang: string, place: IdeTrustPlace): string {
  const runs = TRUST_LANGS[lang]?.runs ?? 'its build code';
  const where = place.kind === 'ssh' ? `on ${place.host?.trim() || 'the SSH host'}` : 'on this Mac';
  return `Language servers may run this project’s own build code (${runs}) ${where}. Only do this for code you trust. You can turn it off in Project environment.`;
}

/** Why the trust save failed, and what to do, for the notice. */
export function ideTrustErrorText(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e ?? '');
  if (/agent session/i.test(msg)) return 'An agent session can’t trust a project. Click Trust project yourself, in Otto.';
  if (/requires root/i.test(msg)) return 'Only the root account can trust a local project. Ask its owner to click Trust project.';
  return `Couldn’t trust this project: ${loadErrorText(e)}`;
}

// "Hide" on the notice: per project, until the app restarts (memory only).
const dismissed = new Set<string>();
export function dismissIdeTrustNotice(projectKey: string): void { dismissed.add(projectKey); }
export function showIdeTrustNotice(projectKey: string): void { dismissed.delete(projectKey); }
export function ideTrustNoticeDismissed(projectKey: string): boolean { return dismissed.has(projectKey); }
