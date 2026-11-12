<script lang="ts">
  // Edit a NATIVE MCP server (the agent CLI launches it from `.mcp.json`):
  // name, command, arguments, env, secret env and whether it is written for
  // new sessions. What Settings ▸ MCP servers did, through the same
  // `/mcp-servers/{id}` route, so disabling or renaming still sweeps the
  // launcher env files that hold its resolved secrets. Secret values go to
  // the macOS Keychain; a bare `KEY=` keeps the stored value.
  import { untrack } from 'svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toastError } from '../../lib/toastError';
  import { announce } from '../../lib/announce.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type { McpServerDetail, UpdateMcpServerReq } from '../../lib/api/types';
  import { NOT_GOVERNED_NOTE, nonEmptyLines, parseEnvLines, secretEnvBody } from './runMode';

  interface Props {
    server: McpServerDetail;
    onclose: () => void;
    onsaved: () => void;
  }
  let { server, onclose, onsaved }: Props = $props();

  // A snapshot of the row when the form opens (later list reloads don't
  // overwrite what the person is typing).
  const initial = untrack(() => server);
  let name = $state(initial.name);
  let command = $state(initial.command);
  let argsText = $state(initial.args.join('\n'));
  let envText = $state(Object.entries(initial.env).map(([k, v]) => `${k}=${v}`).join('\n'));
  let secretEnvText = $state(initial.secret_env_keys.map((k) => `${k}=`).join('\n'));
  let enabled = $state(initial.enabled);
  let showSecrets = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);

  const SHELL_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

  async function save(): Promise<void> {
    const env = parseEnvLines(envText);
    const secret_env = secretEnvBody(secretEnvText, initial.secret_env_keys, true);
    const bad = [...Object.keys(env), ...Object.keys(secret_env)].find((k) => !SHELL_NAME.test(k));
    error = !name.trim()
      ? 'Give the server a name.'
      : auth.isRoot && !command.trim()
        ? 'A native server needs a command.'
        : bad
          ? `“${bad}” can’t be an env key. Use letters, digits and _, not starting with a digit.`
          : null;
    if (error) {
      announce(error, { assertive: true });
      return;
    }
    // Only the owner changes what runs on this Mac; others rename / toggle.
    const body: UpdateMcpServerReq = auth.isRoot
      ? { name: name.trim(), command: command.trim(), args: nonEmptyLines(argsText), env, secret_env, enabled }
      : { name: name.trim(), enabled };
    saving = true;
    try {
      await mcpApi.update(initial.id, body);
      toasts.success('MCP server updated', name.trim());
      onsaved();
      onclose();
    } catch (e) {
      toastError('Couldn’t save the MCP server', e);
    } finally {
      saving = false;
    }
  }
</script>

<Modal title="Edit native MCP server" width={540} {onclose}>
  <div class="form">
    <p class="note"><Icon name="warning" size={12} /> {NOT_GOVERNED_NOTE}</p>
    <label class="field">
      <span>Name</span>
      <input dir="ltr" class="mono" bind:value={name} spellcheck="false" autocomplete="off" placeholder="linear" />
      <span class="hint">The key under <code>mcpServers</code> in <code>.mcp.json</code> (unique per workspace).</span>
    </label>
    {#if !auth.isRoot}<p class="hint">The owner manages credentials and the server command.</p>{/if}
    <label class="field">
      <span>Command</span>
      <input dir="ltr" class="mono" bind:value={command} disabled={!auth.isRoot} spellcheck="false" autocomplete="off" placeholder="npx" />
    </label>
    <label class="field">
      <span>Arguments <em>(one per line)</em></span>
      <textarea dir="ltr" class="mono" rows="3" bind:value={argsText} disabled={!auth.isRoot} spellcheck="false" placeholder={'-y\n@linear/mcp'}></textarea>
    </label>
    <label class="field">
      <span>Environment <em>(KEY=value, one per line)</em></span>
      <textarea dir="ltr" class="mono" rows="3" bind:value={envText} disabled={!auth.isRoot} spellcheck="false" placeholder="LOG_LEVEL=info"></textarea>
      <span class="hint">Non-secret values only, stored in Otto’s database. Put tokens and keys below.</span>
    </label>
    <div class="field">
      <div class="secret-row">
        <label for="native-secret-env"><Icon name="lock" size={12} /> Secret environment <em>(KEY=value)</em></label>
        <button class="btn small ghost" type="button" onclick={() => (showSecrets = !showSecrets)}>{showSecrets ? 'Hide' : 'Show'}</button>
      </div>
      <textarea id="native-secret-env" dir="ltr" class="mono" class:masked={!showSecrets} rows="3" bind:value={secretEnvText} disabled={!auth.isRoot} spellcheck="false" placeholder="API_TOKEN=…"></textarea>
      <span class="hint">Stored in the macOS Keychain, never in Otto’s database. A bare <code>KEY=</code> keeps the stored value; a removed line deletes it.</span>
    </div>
    <label class="check">
      <input type="checkbox" bind:checked={enabled} />
      <span>Enabled: write it to <code>.mcp.json</code> for new sessions</span>
    </label>
    {#if error}<p class="field-error">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose} disabled={saving}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={saving}>{saving ? 'Saving…' : 'Save changes'}</button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field > span,
  .secret-row label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .field em {
    font-style: normal;
    opacity: 0.7;
  }
  .field > .hint,
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin: 0;
  }
  .secret-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .masked {
    -webkit-text-security: disc;
  }
  .note {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    font-size: var(--fs-s);
  }
  .note > :global(svg) {
    color: var(--warning);
    flex-shrink: 0;
    margin-block-start: 2px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
  }
  .field-error {
    color: var(--danger);
    font-size: var(--fs-s);
    margin: 0;
  }
  .mono,
  code {
    font-family: var(--font-mono);
  }
</style>
