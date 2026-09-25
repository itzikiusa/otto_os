<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Settings → Language Servers: shows LSP server availability and lets users
  // install missing servers via a spawned shell session.
  import { api } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { LspCapabilities, LspServerStatus } from '../../lib/api/types';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import SectionIntro from './SectionIntro.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { loadErrorText } from '../../lib/loadError';
  import type { Session } from '../../lib/api/types';

  // ── State ──────────────────────────────────────────────────────────────────

  let caps: LspCapabilities | null = $state(null);
  let loading = $state(true);
  let error = $state('');
  let installing: Set<string> = $state(new Set());
  let installingAll = $state(false);

  // ── Load ───────────────────────────────────────────────────────────────────

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    error = '';
    try {
      caps = await api.get<LspCapabilities>('/lsp/capabilities');
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // ── Install helpers ────────────────────────────────────────────────────────

  const wsId = $derived(ws.currentId);

  async function installLang(lang: string, command: string): Promise<void> {
    if (!wsId) { toasts.error("Couldn't start the install", 'Select a workspace first.'); return; }
    // It installs software on this Mac: say exactly what runs first.
    const ok = await confirmer.ask(
      `Run “${command}” in a new terminal session? It installs the ${langLabel(lang)} language server on this Mac.`,
      { title: `Install ${langLabel(lang)} server`, confirmLabel: 'Install', danger: false },
    );
    if (!ok) return;
    installing = new Set([...installing, lang]);
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/lsp/install`, { langs: [lang] });
      ws.addSession(session); // navigates to the install session
      toasts.info(`Installing the ${langLabel(lang)} server`, 'Follow it in the terminal session, then Refresh here.');
    } catch (e) {
      toasts.error("Couldn't start the install", loadErrorText(e));
    } finally {
      installing = new Set([...installing].filter((l) => l !== lang));
    }
  }

  async function installAll(): Promise<void> {
    if (!wsId) { toasts.error("Couldn't start the install", 'Select a workspace first.'); return; }
    const cmds = missingWithInstall.map((s) => s.install_command).join('\n');
    const ok = await confirmer.ask(
      `Run these in a new terminal session to install ${missingWithInstall.length} missing language server${missingWithInstall.length === 1 ? '' : 's'} on this Mac?\n\n${cmds}`,
      { title: 'Install missing servers', confirmLabel: 'Install', danger: false },
    );
    if (!ok) return;
    installingAll = true;
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/lsp/install`, {});
      ws.addSession(session); // navigates to the install session
      toasts.info('Installing the missing servers', 'Follow it in the terminal session, then Refresh here.');
    } catch (e) {
      toasts.error("Couldn't start the install", loadErrorText(e));
    } finally {
      installingAll = false;
    }
  }

  // ── Derived ────────────────────────────────────────────────────────────────

  const missingWithInstall = $derived(
    (caps as LspCapabilities | null)?.servers.filter((s: LspServerStatus) => !s.available && s.install_command) ?? [],
  );

  // Language display names
  const LANG_LABELS: Record<string, string> = {
    go: 'Go',
    python: 'Python',
    typescript: 'TypeScript',
    javascript: 'JavaScript',
    rust: 'Rust',
    json: 'JSON',
    html: 'HTML',
    css: 'CSS',
    markdown: 'Markdown',
    java: 'Java',
    typescriptreact: 'TypeScript (React)',
    javascriptreact: 'JavaScript (React)',
    scss: 'SCSS',
  };

  function langLabel(lang: string): string {
    return LANG_LABELS[lang] ?? lang;
  }

  async function copyCmd(cmd: string): Promise<void> {
    try {
      await copyTextOrThrow(cmd);
      toasts.success('Install command copied');
    } catch {
      toasts.error("Couldn't copy", 'Select the command and copy it manually.');
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('language-servers')} subtitle="Code intelligence for Otto's editors">
    {#snippet actions()}
      <button class="btn" data-icon="refresh" disabled={loading} onclick={() => void load()}>
        <Icon name="refresh" size={13} /> {loading ? 'Checking…' : 'Refresh'}
      </button>
      {#if missingWithInstall.length > 0}
        <button class="btn primary" disabled={installingAll} onclick={installAll}>
          {installingAll ? 'Starting…' : `Install missing (${missingWithInstall.length})…`}
        </button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
  <SectionIntro>Otto's editors use these for completion, hover and go-to-definition. The daemon reads your shell PATH, so servers installed via <code>mise</code>, <code>asdf</code> or your shell rc files are found.</SectionIntro>

  <LoadState what="language servers" {loading} {error} empty={!caps || caps.servers.length === 0} onretry={() => void load()} rows={5}>
    {#snippet emptyView()}
      <EmptyState icon="function" title="No language servers configured" body="This daemon doesn't list any supported languages." />
    {/snippet}
    {#if caps}
      <div class="table-wrap card">
        <table class="ls-table">
          <thead>
            <tr>
              <th>Language</th>
              <th>Status</th>
              <th>Server</th>
              <th><span class="sr-only">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {#each caps.servers as server (server.lang)}
              <tr>
                <td class="lang-name">{langLabel(server.lang)}</td>
                <td>
                  {#if server.available}
                    <span class="chip ok"><Icon name="check" size={12} /> Installed</span>
                  {:else}
                    <span class="chip">Not installed</span>
                  {/if}
                </td>
                <td class="cmd-cell">
                  {#if server.available}
                    <span class="mono path" title={server.command}>{server.command}</span>
                  {:else if server.install_command}
                    <button class="mono path copy" title="Copy: {server.install_command}" onclick={() => void copyCmd(server.install_command ?? '')}>
                      {server.install_command}
                    </button>
                  {:else}
                    <span class="mono path dim" title={server.command}>{server.command} · install it yourself</span>
                  {/if}
                </td>
                <td class="action-cell">
                  {#if !server.available && server.install_command}
                    <button
                      class="btn small"
                      disabled={installing.has(server.lang)}
                      onclick={() => installLang(server.lang, server.install_command ?? '')}
                    >
                      {installing.has(server.lang) ? 'Starting…' : 'Install…'}
                    </button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </LoadState>
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  /* Wide paths scroll inside the card, never the page. */
  .table-wrap {
    overflow-x: auto;
    max-width: var(--settings-col);
  }
  .ls-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-m);
    table-layout: fixed;
  }
  .ls-table th {
    position: sticky;
    top: 0;
    text-align: start;
    padding: 8px 12px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .ls-table th:nth-child(1) {
    width: 150px;
  }
  .ls-table th:nth-child(2) {
    width: 130px;
  }
  .ls-table th:nth-child(4) {
    width: 96px;
  }
  .ls-table td {
    height: 32px;
    padding: 4px 12px;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }
  .ls-table tr:last-child td {
    border-bottom: none;
  }
  .ls-table tbody tr:hover td {
    background: var(--hover);
  }
  .lang-name {
    font-weight: 500;
    color: var(--text);
    white-space: nowrap;
  }
  .cmd-cell {
    min-width: 0;
  }
  .path {
    display: block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-s);
    color: var(--text-dim);
    direction: ltr;
  }
  .copy {
    border: none;
    background: none;
    padding: 0;
    text-align: start;
    cursor: copy;
  }
  .copy:hover {
    color: var(--text);
  }
  .action-cell {
    white-space: nowrap;
    text-align: end;
  }
  @media (max-width: 640px) {
    .ls-table {
      min-width: 520px;
    }
  }
</style>
