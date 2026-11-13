// "Open" for a cross-module search hit inside the main window (⌘K palette and
// the in-app floating bar). Most hits are a plain route (`hitRoute`); a Vault
// note has no deep link, so after navigating to `#/vault` this waits for the
// Vault page to bind its store (`vaultPort`, the same port the
// `otto.ui_vault_open_note` handler drives) and selects the vault + note.

import { router } from './router.svelte';
import { hitRoute, hitVaultNote } from './floatingBar';
import { vaultPort } from './uiCommands/vaultPort';
import { waitFor } from './uiCommands/pagePort';
import { toastError } from './toastError';

export function openHitInApp(hit: { kind: string; id: string; title?: string }): void {
  router.go(hitRoute(hit));
  const note = hitVaultNote(hit);
  if (note) void openVaultNote(note.vault, note.path, hit.title ?? note.path);
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
