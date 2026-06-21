<script lang="ts">
  // Settings → Language Servers: shows LSP server availability and lets users
  // install missing servers via a spawned shell session.
  import { api } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { LspCapabilities, LspServerStatus } from '../../lib/api/types';
  import Skeleton from '../../lib/components/Skeleton.svelte';
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
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // ── Install helpers ────────────────────────────────────────────────────────

  const wsId = $derived(ws.currentId);

  async function installLang(lang: string): Promise<void> {
    if (!wsId) { toasts.error('No workspace selected'); return; }
    installing = new Set([...installing, lang]);
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/lsp/install`, { langs: [lang] });
      ws.addSession(session); // navigates to the install session
      toasts.info('Installing…', 'Watch the terminal session for progress.');
    } catch (e) {
      toasts.error('Install failed', e instanceof Error ? e.message : String(e));
    } finally {
      installing = new Set([...installing].filter((l) => l !== lang));
    }
  }

  async function installAll(): Promise<void> {
    if (!wsId) { toasts.error('No workspace selected'); return; }
    installingAll = true;
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/lsp/install`, {});
      ws.addSession(session); // navigates to the install session
      toasts.info('Installing all missing…', 'Watch the terminal session for progress.');
    } catch (e) {
      toasts.error('Install failed', e instanceof Error ? e.message : String(e));
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
</script>

<div class="page">
  <div class="page-header">
    <div>
      <div class="page-title">Language Servers</div>
      <div class="page-subtitle">
        Servers are detected on your PATH. The daemon promotes your shell PATH so
        tools installed via <code>mise</code>, <code>asdf</code>, or shell rc files are found.
      </div>
    </div>
    {#if missingWithInstall.length > 0}
      <button
        class="btn btn-primary"
        disabled={installingAll}
        onclick={installAll}
      >
        {installingAll ? 'Installing…' : `Install all missing (${missingWithInstall.length})`}
      </button>
    {/if}
  </div>

  {#if loading}
    <div class="skeleton-list">
      {#each [1,2,3,4,5] as _}
        <Skeleton height={44} />
      {/each}
    </div>
  {:else if error}
    <div class="error-msg">{error}</div>
  {:else if caps}
    <table class="ls-table">
      <thead>
        <tr>
          <th>Language</th>
          <th>Server command</th>
          <th>Status</th>
          <th>Install command</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each caps.servers as server (server.lang)}
          <tr>
            <td class="lang-name">{langLabel(server.lang)}</td>
            <td class="mono dim">{server.command}</td>
            <td>
              {#if server.available}
                <span class="badge ok">Installed ✓</span>
              {:else}
                <span class="badge missing">Missing ✗</span>
              {/if}
            </td>
            <td class="mono dim install-cmd">
              {server.install_command ?? '—'}
            </td>
            <td class="action-cell">
              {#if !server.available && server.install_command}
                <button
                  class="btn btn-sm"
                  disabled={installing.has(server.lang)}
                  onclick={() => installLang(server.lang)}
                >
                  {installing.has(server.lang) ? 'Installing…' : 'Install'}
                </button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>

    {#if caps.servers.length === 0}
      <div class="empty-note dim">
        No language servers are configured. The backend detected no supported languages.
      </div>
    {/if}
  {/if}
</div>

<style>
  .page {
    padding: 24px;
    max-width: min(800px, 92vw);
  }

  .page-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 20px;
  }

  .page-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
    margin-bottom: 4px;
  }

  .page-subtitle {
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
    max-width: 560px;
  }

  .page-subtitle code {
    font-family: var(--font-mono);
    font-size: 11px;
    background: var(--surface-2);
    padding: 1px 4px;
    border-radius: 3px;
  }

  .ls-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }

  .ls-table th {
    text-align: start;
    padding: 6px 10px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }

  .ls-table td {
    padding: 8px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
    vertical-align: middle;
  }

  .ls-table tr:last-child td {
    border-bottom: none;
  }

  .ls-table tr:hover td {
    background: var(--surface-2);
  }

  .lang-name {
    font-weight: 500;
    color: var(--text);
    white-space: nowrap;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }

  .install-cmd {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .action-cell {
    white-space: nowrap;
    text-align: end;
  }

  .badge {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 999px;
    font-weight: 500;
    white-space: nowrap;
  }

  .badge.ok {
    background: color-mix(in srgb, var(--status-working, #34c759) 15%, transparent);
    color: var(--status-working, #34c759);
  }

  .badge.missing {
    background: color-mix(in srgb, var(--status-exited, #ff3b30) 12%, transparent);
    color: var(--status-exited, #ff453a);
  }

  .btn {
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    font-size: 12px;
    cursor: pointer;
    transition: background 120ms ease-out;
    white-space: nowrap;
  }
  .btn:hover:not(:disabled) {
    background: var(--surface);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .btn-primary {
    background: color-mix(in srgb, var(--accent) 80%, transparent);
    color: white;
    border-color: transparent;
  }
  .btn-primary:hover:not(:disabled) {
    background: var(--accent);
  }

  .btn-sm {
    height: 24px;
    padding: 0 10px;
    font-size: 11px;
  }

  .skeleton-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .error-msg {
    padding: 12px;
    font-size: 12px;
    color: var(--danger, #e05252);
  }

  .empty-note {
    padding: 16px;
    font-size: 12px;
    text-align: center;
  }
</style>
