// "Open" for a cross-module search hit inside the main window (⌘K palette and
// the in-app floating bar). Most hits are a plain route (`hitRoute`); a Vault
// note has no deep link, so after navigating to `#/vault` this waits for the
// Vault page to bind its store (`vaultPort`, the same port the
// `otto.ui_vault_open_note` handler drives) and selects the vault + note.

import { api } from './api/client';
import type { WorkflowRun } from './api/types';
import { confirmer } from './confirm.svelte';
import { router } from './router.svelte';
import { hitRoute, hitVaultNote } from './floatingBar';
import { apiClient } from './stores/apiClient.svelte';
import { toasts } from './toast.svelte';
import { vaultPort } from './uiCommands/vaultPort';
import { waitFor } from './uiCommands/pagePort';
import { toastError } from './toastError';

export function openHitInApp(hit: { kind: string; id: string; title?: string }): void {
  router.go(hitRoute(hit));
  const note = hitVaultNote(hit);
  if (note) void openVaultNote(note.vault, note.path, hit.title ?? note.path);
}

/** Run a server-advertised secondary power verb on a cross-module search hit.
 *  Outward verbs (`rerun` workflow or saved API request) confirm first. */
export async function runHitPowerAction(
  hit: { kind: string; id: string; title: string; subtitle?: string | null },
  action: 'rerun' | 'review',
): Promise<void> {
  if (action === 'review' && hit.kind === 'repo') {
    router.go(`git/${encodeURIComponent(hit.id)}/review`);
    return;
  }
  if (action === 'rerun' && hit.kind === 'workflow') {
    const ok = await confirmer.ask(
      `Run workflow “${hit.title}” now? Its steps may touch repositories or external services.`,
      { title: 'Run workflow', confirmLabel: 'Run workflow' },
    );
    if (!ok) return;
    try {
      const run = await api.post<WorkflowRun>(`/workflows/${encodeURIComponent(hit.id)}/run`, {});
      router.go(
        run?.id
          ? `workflows/${encodeURIComponent(hit.id)}/runs/${encodeURIComponent(run.id)}`
          : `workflows/${encodeURIComponent(hit.id)}`,
      );
      toasts.success('Workflow started', hit.title);
    } catch (e) {
      toastError(`Couldn’t run “${hit.title}”`, e);
    }
    return;
  }
  if (action === 'rerun' && hit.kind === 'api_request') {
    router.go(`api/${encodeURIComponent(hit.id)}`);
    try {
      await apiClient.ensureLoaded();
      const opened = await apiClient.openRequest(hit.id);
      if (!opened) {
        toasts.warn('Request not found', `“${hit.title}” is no longer in this workspace.`);
        return;
      }
      const target = hit.subtitle ? ` (${hit.subtitle})` : '';
      const ok = await confirmer.ask(`Send saved request “${hit.title}”${target} now?`, {
        title: 'Send request',
        confirmLabel: 'Send',
      });
      if (!ok) return;
      if (apiClient.draft.kind === 'grpc') await apiClient.executeGrpc();
      else await apiClient.execute();
    } catch (e) {
      toastError(`Couldn’t send “${hit.title}”`, e);
    }
  }
}

async function openVaultNote(vaultId: string, path: string, title: string): Promise<void> {
  // `parts` moves only once the Vault chunk is in (and after any leave
  // guard), so wait for the arrival; leaving again — or never getting there
  // (a guard kept the user where they were) — ends the wait quietly.
  let arrived = false;
  const left = (): boolean => {
    if (router.parts[0] === 'vault') arrived = true;
    return arrived && router.parts[0] !== 'vault';
  };
  const signal = new AbortController().signal;
  try {
    const vault = await waitFor(() => (left() ? 'left' : arrived && vaultPort.peek()), signal, 15_000, 'the Vault page');
    if (vault === 'left') return;
    if (vault.vaults.length === 0 && !vault.loading) await vault.load();
    await waitFor(() => left() || !vault.loading, signal, 15_000, 'the vault list');
    if (left()) return;
    const v = vault.vaults.find((x) => String(x.id) === vaultId);
    if (!v) throw new Error('That vault is no longer registered.');
    if (vault.current?.id !== v.id) await vault.select(v.id);
    // An unsaved note can refuse the switch; the page already says why.
    if (vault.current?.id !== v.id || left()) return;
    await vault.open(path);
    if (vault.notePath !== path && !left()) throw new Error(`“${path}” could not be opened.`);
  } catch (e) {
    if (arrived && !left()) toastError(`Couldn’t open “${title}”`, e);
  }
}
