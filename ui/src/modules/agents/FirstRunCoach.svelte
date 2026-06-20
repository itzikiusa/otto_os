<script lang="ts">
  // First-run coach: a guided checklist shown on the Agents page when the
  // workspace has no agent sessions yet. It walks a fresh user from "just
  // finished setup" to "first agent running" by (1) confirming an agent CLI is
  // installed, (2) ensuring a workspace exists, (3) optionally installing a
  // couple of recommended bundled skills, and (4) launching a first session
  // seeded with a friendly starter prompt. Every step reuses an existing API —
  // no new backend is invented here. Dismissable; the dismissal is remembered
  // per-machine so it doesn't nag after the user has found their footing.
  import { contextApi } from '../../lib/api/context';
  import type { BundledSkill } from '../../lib/api/types';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';

  interface Props {
    /** Called when the user dismisses the coach (falls back to the bare empty state). */
    ondismiss: () => void;
  }
  let { ondismiss }: Props = $props();

  // --- Step 1: agent CLIs ----------------------------------------------------
  // `providers` is the STATIC provider registry — it always lists claude/codex/agy
  // regardless of what's installed, so it can't gate the launch. Real detection
  // lives in /meta.tools: a coding agent is usable only when its tool is `found`.
  const agentProviders = $derived(
    (auth.meta?.providers ?? []).filter((p) => p !== 'shell'),
  );
  // Tool rows for claude/codex from /meta.tools (shows version when found).
  const agentTools = $derived(
    (auth.meta?.tools ?? []).filter((t) => t.name === 'claude' || t.name === 'codex'),
  );
  // The gate: at least one agent CLI is actually installed on PATH.
  const hasAgentCli = $derived(agentTools.some((t) => t.found));
  // The provider names whose CLI is actually present.
  const foundProviders = $derived(agentTools.filter((t) => t.found).map((t) => t.name));

  // --- Step 2: workspace -----------------------------------------------------
  const hasWorkspace = $derived(ws.workspaces.length > 0 && ws.current !== null);
  let wsName = $state('');
  let wsPath = $state('~/');
  let wsBusy = $state(false);
  let pickerOpen = $state(false);
  // Suggest a name from the trailing path segment until the user types one.
  let wsNameTouched = $state(false);
  $effect(() => {
    if (wsNameTouched) return;
    const seg = wsPath.replace(/\/+$/, '').split('/').pop() ?? '';
    if (seg !== '' && seg !== '~') wsName = seg;
  });
  const canCreateWs = $derived(wsName.trim() !== '' && wsPath.trim() !== '' && !wsBusy);

  async function createWorkspace(): Promise<void> {
    if (!canCreateWs) return;
    wsBusy = true;
    try {
      const w = await ws.createWorkspace(wsName, wsPath);
      toasts.success('Workspace created', w.name);
    } catch (e) {
      toasts.error('Could not create workspace', e instanceof Error ? e.message : String(e));
    } finally {
      wsBusy = false;
    }
  }

  // --- Step 3: recommended skills (optional) ---------------------------------
  // A short, opinionated set of bundled skills that pay off on a first run.
  // We only surface the ones Otto actually ships (matched against /library/bundled),
  // so the list never advertises a skill that isn't installable.
  const RECOMMENDED = ['grill', 'correctness-review', 'security-review', 'insights'];
  let bundled: BundledSkill[] = $state([]);
  let skillsLoaded = $state(false);
  let skillBusy: Set<string> = $state(new Set());
  const recommendedSkills = $derived(
    RECOMMENDED.map((n) => bundled.find((b) => b.name === n)).filter(
      (b): b is BundledSkill => b !== undefined,
    ),
  );

  $effect(() => {
    if (skillsLoaded || !auth.isRoot) return;
    void loadSkills();
  });

  async function loadSkills(): Promise<void> {
    try {
      bundled = await contextApi.listBundled();
    } catch {
      // Non-fatal: the skills step just stays empty.
      bundled = [];
    } finally {
      skillsLoaded = true;
    }
  }

  function setSkillBusy(name: string, on: boolean): void {
    const next = new Set(skillBusy);
    if (on) next.add(name);
    else next.delete(name);
    skillBusy = next;
  }

  async function installSkill(s: BundledSkill): Promise<void> {
    setSkillBusy(s.name, true);
    try {
      await contextApi.installBundled(s.name);
      toasts.success('Skill installed', s.name);
      await loadSkills();
    } catch (e) {
      toasts.error('Install failed', e instanceof Error ? e.message : String(e));
    } finally {
      setSkillBusy(s.name, false);
    }
  }

  // --- Step 4: launch first session ------------------------------------------
  // Friendly starter prompt: orients the agent at the workspace and asks for a
  // grounded first task that produces immediate, visible value.
  const STARTER_PROMPT =
    'You are starting fresh in this repository. Give me a short, friendly orientation: ' +
    "summarise what this project is, the main folders, and how to build/run/test it. " +
    "Then suggest 3 small first tasks I could ask you to do. Keep it concise.";

  let launchBusy = $state(false);

  // First useful provider: prefer one whose CLI is actually installed — the
  // global default if it's installed, else claude, else the first found agent.
  // Falls back to the registry only if detection is somehow empty.
  const launchProvider = $derived.by(() => {
    const def = auth.meta?.default_provider;
    if (def && foundProviders.includes(def)) return def;
    if (foundProviders.includes('claude')) return 'claude';
    if (foundProviders.length > 0) return foundProviders[0];
    return agentProviders.includes('claude') ? 'claude' : (agentProviders[0] ?? '');
  });

  async function launchFirstSession(): Promise<void> {
    if (launchBusy || !hasWorkspace || !hasAgentCli) return;
    launchBusy = true;
    try {
      const s = await ws.createSession({
        kind: 'agent',
        provider: launchProvider,
        title: 'First session',
        cwd: ws.current?.root_path ?? null,
        meta: { source: 'onboarding' },
      });
      // createSession → addSession → navigateToSession handles the route.
      // Seed the starter prompt once the PTY has had a moment to spawn the CLI.
      // Best-effort: a failed seed still leaves a usable, opened session.
      setTimeout(() => {
        void ws.sendInput(s.id, STARTER_PROMPT).catch(() => {
          /* agent not ready / no permission — the empty session is still fine */
        });
      }, 1500);
      remember();
      ondismiss();
    } catch (e) {
      toasts.error('Could not start session', e instanceof Error ? e.message : String(e));
    } finally {
      launchBusy = false;
    }
  }

  // Persist dismissal so the coach doesn't reappear on every empty Agents view.
  const LS_DISMISSED = 'otto_firstrun_dismissed';
  function remember(): void {
    try {
      localStorage.setItem(LS_DISMISSED, '1');
    } catch {
      /* private mode — coach just shows again next time */
    }
  }
  function dismiss(): void {
    remember();
    ondismiss();
  }
</script>

<div class="coach-wrap">
  <div class="coach card">
    <button class="coach-close" title="Dismiss" onclick={dismiss}><Icon name="x" size={13} /></button>

    <div class="coach-head">
      <div class="coach-mark"><Icon name="zap" size={20} /></div>
      <div>
        <h2>Let's launch your first agent</h2>
        <p>A few quick checks, then Otto starts a coding agent in your workspace.</p>
      </div>
    </div>

    <!-- Step 1: agent CLI -->
    <div class="step" class:ok={hasAgentCli}>
      <span class="step-mark" class:ok={hasAgentCli}>
        {#if hasAgentCli}<Icon name="check" size={12} />{:else}<span class="num">1</span>{/if}
      </span>
      <div class="step-body">
        <div class="step-title">Agent CLI detected</div>
        {#if hasAgentCli}
          <div class="tool-chips">
            {#each agentTools as t (t.name)}
              <span class="chip" class:found={t.found}>
                <Icon name={t.found ? 'check' : 'x'} size={10} />
                {t.name}{t.found && t.version ? ` ${t.version}` : ''}
              </span>
            {/each}
            {#if agentTools.length === 0}
              <span class="chip found">{agentProviders.join(', ')}</span>
            {/if}
          </div>
        {:else}
          <div class="step-hint">
            No coding-agent CLI found on your <span class="mono">PATH</span>. Install one, then
            reopen Otto:
            <ul class="install-list">
              <li>
                <strong>Claude Code</strong> —
                <span class="mono">npm i -g @anthropic-ai/claude-code</span>
              </li>
              <li>
                <strong>Codex</strong> —
                <span class="mono">npm i -g @openai/codex</span>
              </li>
            </ul>
            <button class="btn small" onclick={() => auth.refreshMeta()}>
              <Icon name="refresh" size={11} /> Re-check
            </button>
          </div>
        {/if}
      </div>
    </div>

    <!-- Step 2: workspace -->
    <div class="step" class:ok={hasWorkspace}>
      <span class="step-mark" class:ok={hasWorkspace}>
        {#if hasWorkspace}<Icon name="check" size={12} />{:else}<span class="num">2</span>{/if}
      </span>
      <div class="step-body">
        <div class="step-title">Workspace ready</div>
        {#if hasWorkspace}
          <div class="step-hint">
            <span class="mono">{ws.current?.name}</span>
            <span class="dim mono">· {ws.current?.root_path}</span>
          </div>
        {:else}
          <div class="step-hint">A workspace maps to a project directory. Sessions run inside it.</div>
          <div class="ws-form">
            <input class="input" bind:value={wsName} oninput={() => (wsNameTouched = true)} placeholder="my-project" />
            <div class="path-row">
              <input class="input mono" bind:value={wsPath} spellcheck="false" placeholder="~/code/my-project" />
              <button class="btn" type="button" onclick={() => (pickerOpen = true)}>Browse…</button>
            </div>
            <button class="btn small primary" disabled={!canCreateWs} onclick={createWorkspace}>
              {wsBusy ? 'Creating…' : 'Create workspace'}
            </button>
          </div>
        {/if}
      </div>
    </div>

    <!-- Step 3: recommended skills (optional) -->
    {#if auth.isRoot && recommendedSkills.length > 0}
      <div class="step optional">
        <span class="step-mark"><span class="num">3</span></span>
        <div class="step-body">
          <div class="step-title">
            Recommended skills <span class="opt-tag">optional</span>
          </div>
          <div class="step-hint">
            Drop-in expertise your agents can use — code review and usage insights. Install now or
            later from <button class="link" onclick={() => router.go('settings/skills')}>Settings → Skills</button>.
          </div>
          <div class="skill-rows">
            {#each recommendedSkills as s (s.name)}
              {@const installed = s.state === 'up_to_date' || s.state === 'ahead'}
              <div class="skill-row">
                <span class="mono skill-name">{s.name}</span>
                {#if installed}
                  <span class="chip found"><Icon name="check" size={10} /> installed</span>
                {:else}
                  <button class="btn small" disabled={skillBusy.has(s.name)} onclick={() => installSkill(s)}>
                    {skillBusy.has(s.name) ? '…' : s.state === 'update_available' ? 'Update' : 'Install'}
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      </div>
    {/if}

    <!-- Step 4: launch -->
    <div class="coach-launch">
      <button
        class="btn primary big"
        disabled={!hasAgentCli || !hasWorkspace || launchBusy}
        onclick={launchFirstSession}
      >
        <Icon name="play" size={13} />
        {launchBusy ? 'Starting…' : 'Start your first agent'}
      </button>
      <p class="launch-hint">
        Opens a <span class="mono">{launchProvider || 'claude'}</span> session and asks it to orient
        you in the project.
      </p>
    </div>
  </div>
</div>

{#if pickerOpen}
  <FolderPicker
    title="Choose project directory"
    start={wsPath}
    onpick={(p) => {
      wsPath = p;
      wsNameTouched = false;
      pickerOpen = false;
    }}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<style>
  .coach-wrap {
    height: 100%;
    display: grid;
    place-items: center;
    overflow: auto;
    padding: 24px;
  }
  .coach {
    position: relative;
    width: 480px;
    max-width: 100%;
    padding: 22px 24px 24px;
  }
  .coach-close {
    position: absolute;
    top: 12px;
    right: 12px;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    padding: 4px;
    border-radius: var(--radius-s);
    line-height: 1;
  }
  .coach-close:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .coach-head {
    display: flex;
    gap: 13px;
    align-items: flex-start;
    margin-bottom: 18px;
  }
  .coach-mark {
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: var(--radius-l);
    display: grid;
    place-items: center;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .coach-head h2 {
    margin: 0 0 2px;
    font-size: 16px;
  }
  .coach-head p {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.45;
  }

  .step {
    display: flex;
    gap: 11px;
    padding: 12px 0;
    border-top: 1px solid var(--border);
  }
  .step.optional {
    opacity: 0.95;
  }
  .step-mark {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
    font-size: 11px;
  }
  .step-mark.ok {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    border-color: transparent;
    color: var(--status-working);
  }
  .step-mark .num {
    font-weight: 600;
  }
  .step-body {
    flex: 1;
    min-width: 0;
  }
  .step-title {
    font-size: 13px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .opt-tag {
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .step-hint {
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.5;
    margin-top: 3px;
  }
  .install-list {
    margin: 6px 0;
    padding-left: 16px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .tool-chips,
  .skill-rows {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .chip.found {
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
    color: var(--status-working);
  }

  .ws-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }
  .path-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .path-row .input {
    flex: 1;
    min-width: 0;
  }
  .ws-form .btn {
    align-self: flex-start;
  }

  .skill-rows {
    flex-direction: column;
    gap: 6px;
  }
  .skill-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .skill-name {
    font-size: 12px;
  }

  .coach-launch {
    margin-top: 16px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .btn.big {
    height: 32px;
    padding: 0 18px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }
  .launch-hint {
    margin: 0;
    font-size: 11px;
    color: var(--text-dim);
    text-align: center;
  }

  .mono {
    font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: inherit;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
