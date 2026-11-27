// "Trust project…" (the editor's trust notice and the status strip's "needs trust" item):
// confirm what is granted, add OTTO_IDE_TRUST_PROJECT_EXECUTION=1 to the project's saved
// environment through the Project environment editor's PUT /ide/environments (re-read
// first, every other entry kept as it is), then re-attach the language server. A refused
// save (local trust needs the root account; an agent credential can't) shows in the notice.
import { api } from '../../../lib/api/client';
import { IDE_RECONNECT_LSP_EVENT } from '../../../lib/components/cm-ide-lsp-reconnect';
import { confirmer } from '../../../lib/confirm.svelte';
import { ide } from '../ideStore.svelte';
import { ideTargetKey, type IdeProjectEnvironment } from '../store/ideTypes';
import { ideLspTrustConfirm, ideTrustErrorText, showIdeTrustNotice, withProjectTrust } from './ideLspTrust';

class IdeLspTrustAction {
  /** Bumped when a notice is hidden or shown again (the dismissals live in ideLspTrust.ts). */
  rev = $state(0);
  busy = $state(false);
  error = $state<{ project: string; text: string } | null>(null);

  async trust(lang: string, rootPath: string): Promise<void> {
    if (this.busy) return;
    const project = ideTargetKey(ide.target);
    const message = ideLspTrustConfirm(lang, { kind: ide.target.kind, host: ide.target.connection_name });
    if (!(await confirmer.ask(message, { title: 'Trust project', confirmLabel: 'Trust project', danger: false }))) return;
    showIdeTrustNotice(project);
    this.rev++;
    this.busy = true;
    this.error = null;
    try {
      await ide.loadEnvironments();
      const env = ide.projectEnv;
      if (ideTargetKey(ide.target) !== project) return;
      if (!env || ide.envError) throw new Error(ide.envError ?? 'The project environment isn’t loaded.');
      const resp = await api.put<{ env: IdeProjectEnvironment }>('/ide/environments', {
        target: ide.targetPayload,
        inherit_shell_env: true,
        ...env,
        env_vars: withProjectTrust(env.env_vars ?? [], true),
      });
      if (ideTargetKey(ide.target) === project) ide.projectEnv = resp.env;
      window.dispatchEvent(new CustomEvent(IDE_RECONNECT_LSP_EVENT, { detail: { rootPath } }));
    } catch (e) {
      this.error = { project, text: ideTrustErrorText(e) };
    } finally {
      this.busy = false;
    }
  }
}

export const ideLspTrust = new IdeLspTrustAction();
