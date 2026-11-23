<script lang="ts">
  // Modal form to create and start a Docker container (`POST /containers/run`).
  // Validates image, port mappings and KEY=VALUE environment lines inline.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { containersApi } from '../../lib/api/containers';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import type { RunContainerResp } from '../../lib/api/types/containers';
  import { parseCommandArgs, parseEnvLines, parsePortSpecs } from './containers-util';
  import {
    fillEmptyFields,
    imageRepoPath,
    lookupImageHint,
    portsFromExposed,
    suggestionSummary,
    type RunSuggestion,
  } from './image-hints';

  interface Props {
    initialImage?: string;
    onclose: () => void;
    oncreated: (resp: RunContainerResp) => void;
  }
  let { initialImage = '', onclose, oncreated }: Props = $props();

  let image = $state(untrack(() => initialImage));
  let seededInitialImage = untrack(() => initialImage);
  let name = $state('');
  let portsText = $state('');
  let envText = $state('');
  let commandText = $state('');
  let showAdvanced = $state(false);
  let volumesText = $state('');
  let workingDir = $state('');
  let restartPolicy = $state('no');
  let autoRemove = $state(false);
  let pullIfMissing = $state(true);
  let submitting = $state(false);
  let fieldErrors = $state<Record<string, string>>({});
  let suggestion = $state<RunSuggestion | null>(null);
  const canUse = $derived(
    !!suggestion &&
      ((!portsText.trim() && suggestion.ports.length > 0) ||
        (!envText.trim() && !!suggestion.env?.length) ||
        (!commandText.trim() && !!suggestion.cmd)),
  );

  // A listed image suggests at once; anything else falls back (debounced) to
  // the ports the local image declares. Not local yet: no suggestion.
  $effect(() => {
    const ref = image.trim();
    const hit = ref ? lookupImageHint(ref) : null;
    suggestion = hit;
    if (!ref || hit) return;
    const ctl = new AbortController();
    const timer = setTimeout(
      () => {
        containersApi.inspectImage(ref, ctl.signal).then(
          (r) => {
            const ports = portsFromExposed(r.exposed_ports);
            if (ports.length) suggestion = { name: imageRepoPath(ref), ports };
          },
          () => {},
        );
      },
      ref === seededInitialImage ? 0 : 400,
    );
    return () => {
      clearTimeout(timer);
      ctl.abort();
    };
  });

  function useSuggestion(): void {
    if (!suggestion) return;
    const next = fillEmptyFields({ ports: portsText, env: envText, cmd: commandText }, suggestion);
    portsText = next.ports;
    envText = next.env;
    commandText = next.cmd;
    clearError('ports');
    clearError('env');
  }

  function syncInitialImage(next: string): void {
    if (next !== seededInitialImage) {
      seededInitialImage = next;
      if (next) image = next;
    }
  }

  $effect(() => {
    syncInitialImage(initialImage);
  });

  function clearError(field: string): void {
    if (fieldErrors[field]) {
      const { [field]: _drop, ...rest } = fieldErrors;
      fieldErrors = rest;
    }
  }

  async function submit(): Promise<void> {
    if (submitting) return;
    const errors: Record<string, string> = {};
    const img = image.trim();
    if (!img) {
      errors.image = 'Enter an image reference, such as nginx:alpine or postgres:16.';
    }
    const parsedPorts = parsePortSpecs(portsText);
    if (parsedPorts.error) errors.ports = parsedPorts.error;

    const parsedEnv = parseEnvLines(envText);
    if (parsedEnv.error) errors.env = parsedEnv.error;

    const trimmedName = name.trim();
    if (trimmedName && !/^[a-zA-Z0-9][a-zA-Z0-9_.-]*$/.test(trimmedName)) {
      errors.name = 'Name must start with a letter or number and contain only letters, numbers, _, ., or -.';
    }

    if (Object.keys(errors).length > 0) {
      fieldErrors = errors;
      return;
    }

    const volumes = volumesText
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean);
    const cmd = parseCommandArgs(commandText);

    submitting = true;
    try {
      const resp = await containersApi.run({
        image: img,
        name: trimmedName || null,
        cmd: cmd.length ? cmd : undefined,
        env: parsedEnv.env.length ? parsedEnv.env : undefined,
        ports: parsedPorts.ports.length ? parsedPorts.ports : undefined,
        volumes: volumes.length ? volumes : undefined,
        working_dir: workingDir.trim() || null,
        restart_policy: autoRemove ? 'no' : restartPolicy,
        auto_remove: autoRemove,
        pull_if_missing: pullIfMissing,
      });
      toasts.success('Container started', trimmedName || resp.short_id);
      if (resp.warnings.length > 0) {
        toasts.warn('Container started with warnings', resp.warnings.join(' · '));
      }
      oncreated(resp);
    } catch (e) {
      toastError('Couldn’t start container', e);
    } finally {
      submitting = false;
    }
  }
</script>

<Modal title="Run container" width={520} dismissable={!submitting} {onclose}>
  <form
    class="ct-run-form"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <div class="field">
      <label for="ct-run-image">Image</label>
      <input
        id="ct-run-image"
        class="input"
        dir="ltr"
        data-autofocus
        bind:value={image}
        oninput={() => clearError('image')}
        placeholder="nginx:alpine"
        aria-invalid={fieldErrors.image ? 'true' : undefined}
        aria-describedby={fieldErrors.image ? 'ct-run-image-err' : suggestion ? 'ct-run-suggest' : undefined}
        disabled={submitting}
      />
      {#if fieldErrors.image}
        <p class="ct-field-err" id="ct-run-image-err">{fieldErrors.image}</p>
      {:else if suggestion}
        <div class="ct-suggest" data-testid="run-suggestion">
          <p class="hint" id="ct-run-suggest" dir="ltr" title={suggestion.note}>
            Suggested for {suggestion.name}: {suggestionSummary(suggestion)}{suggestion.note
              ? ` — ${suggestion.note}`
              : ''}
          </p>
          <button
            type="button"
            class="btn small"
            onclick={useSuggestion}
            disabled={submitting || !canUse}
            aria-label="Use suggested settings for {suggestion.name}"
            title={canUse ? 'Fill the empty fields with these' : 'The fields are already filled in'}
          >
            Use
          </button>
        </div>
      {/if}
    </div>

    <div class="ct-two-col">
      <div class="field">
        <label for="ct-run-name">Container name</label>
        <input
          id="ct-run-name"
          class="input"
          dir="ltr"
          bind:value={name}
          oninput={() => clearError('name')}
          placeholder="Optional"
          aria-invalid={fieldErrors.name ? 'true' : undefined}
          aria-describedby={fieldErrors.name ? 'ct-run-name-err' : undefined}
          disabled={submitting}
        />
        {#if fieldErrors.name}
          <p class="ct-field-err" id="ct-run-name-err">{fieldErrors.name}</p>
        {/if}
      </div>

      <div class="field">
        <label for="ct-run-ports">Ports</label>
        <input
          id="ct-run-ports"
          class="input"
          dir="ltr"
          bind:value={portsText}
          oninput={() => clearError('ports')}
          placeholder={suggestion?.ports.join(', ') || '8080:80'}
          aria-invalid={fieldErrors.ports ? 'true' : undefined}
          aria-describedby={fieldErrors.ports ? 'ct-run-ports-err' : undefined}
          disabled={submitting}
        />
        {#if fieldErrors.ports}
          <p class="ct-field-err" id="ct-run-ports-err">{fieldErrors.ports}</p>
        {:else}
          <span class="hint">Comma-separated host:container mappings.</span>
        {/if}
      </div>
    </div>

    <div class="field">
      <label for="ct-run-cmd">Command</label>
      <input
        id="ct-run-cmd"
        class="input"
        dir="ltr"
        bind:value={commandText}
        placeholder={suggestion?.cmd || 'Optional command override'}
        disabled={submitting}
      />
    </div>

    <div class="field">
      <label for="ct-run-env">Environment variables</label>
      <textarea
        id="ct-run-env"
        class="input ct-mono"
        dir="ltr"
        rows={3}
        bind:value={envText}
        oninput={() => clearError('env')}
        placeholder={suggestion?.env?.join('\n') || 'KEY=value'}
        aria-invalid={fieldErrors.env ? 'true' : undefined}
        aria-describedby={fieldErrors.env ? 'ct-run-env-err' : undefined}
        disabled={submitting}
      ></textarea>
      {#if fieldErrors.env}
        <p class="ct-field-err" id="ct-run-env-err">{fieldErrors.env}</p>
      {:else}
        <span class="hint">One KEY=VALUE entry per line.</span>
      {/if}
    </div>

    <div class="ct-checks">
      <label class="checkbox-row">
        <input type="checkbox" bind:checked={pullIfMissing} disabled={submitting} />
        Pull image if not present locally
      </label>
      <label class="checkbox-row">
        <input type="checkbox" bind:checked={autoRemove} disabled={submitting} />
        Remove container automatically when it stops
      </label>
    </div>

    <button
      type="button"
      class="btn small ghost ct-adv-toggle"
      aria-expanded={showAdvanced}
      onclick={() => (showAdvanced = !showAdvanced)}
    >
      <Icon name={showAdvanced ? 'chevronDown' : 'chevronRight'} size={12} />
      Advanced options
    </button>

    {#if showAdvanced}
      <div class="ct-adv-panel">
        <div class="ct-two-col">
          <div class="field">
            <label for="ct-run-workdir">Working directory</label>
            <input
              id="ct-run-workdir"
              class="input"
              dir="ltr"
              bind:value={workingDir}
              placeholder="/app"
              disabled={submitting}
            />
          </div>

          <div class="field">
            <label for="ct-run-restart">Restart policy</label>
            <select
              id="ct-run-restart"
              class="input"
              bind:value={restartPolicy}
              disabled={submitting || autoRemove}
              title={autoRemove ? 'Disabled while auto-remove is checked' : undefined}
            >
              <option value="no">No</option>
              <option value="unless-stopped">Unless stopped</option>
              <option value="on-failure">On failure</option>
              <option value="always">Always</option>
            </select>
          </div>
        </div>

        <div class="field">
          <label for="ct-run-volumes">Mounts / Volumes</label>
          <textarea
            id="ct-run-volumes"
            class="input ct-mono"
            dir="ltr"
            rows={2}
            bind:value={volumesText}
            placeholder={'/host/path:/container/path\nmy-volume:/data'}
            disabled={submitting}
          ></textarea>
          <span class="hint">One source:target[:ro] bind or volume mount per line.</span>
        </div>
      </div>
    {/if}
  </form>

  {#snippet footer()}
    <button type="button" class="btn" onclick={onclose} disabled={submitting}>Cancel</button>
    <button
      type="button"
      class="btn primary"
      onclick={() => void submit()}
      disabled={submitting || !image.trim()}
      title={!image.trim() ? 'Enter an image name to run' : undefined}
    >
      <Icon name="play" size={12} />
      {submitting ? 'Starting…' : 'Run container'}
    </button>
  {/snippet}
</Modal>

<style>
  .ct-run-form {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ct-two-col {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }
  .ct-mono {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .ct-suggest {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block-start: 4px;
  }
  .ct-suggest .hint {
    flex: 1;
    min-inline-size: 0;
    margin: 0;
    overflow-wrap: anywhere;
  }
  .ct-field-err {
    margin: 2px 0 0;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .ct-checks {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-block-end: 10px;
  }
  .ct-adv-toggle {
    align-self: flex-start;
    margin-block-end: 8px;
  }
  .ct-adv-panel {
    padding-block-start: 8px;
    border-block-start: 1px solid var(--border);
  }
  @media (max-width: 640px) {
    .ct-two-col {
      grid-template-columns: minmax(0, 1fr);
      gap: 0;
    }
  }
</style>
