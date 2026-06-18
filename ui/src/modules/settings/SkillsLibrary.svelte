<script lang="ts">
  // Settings → Skills: the catalogue of skills that ship with Otto ("bundled"),
  // shown grouped by category with their state relative to the installed library
  // copy. Installing/updating writes into the Otto library; the backend always
  // backs up an existing copy before overwriting (backup=true), so nothing is
  // ever destroyed silently. Install/update/install-all are root-only on the
  // server; this page is gated to root in Settings.svelte (like Context Library).
  import { contextApi } from '../../lib/api/context';
  import type { BundledSkill, BundledSkillState } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let skills: BundledSkill[] = $state([]);
  let loading = $state(true);
  // Names with an in-flight install/update/remove (disables that row's buttons).
  let busy: Set<string> = $state(new Set());
  // Category currently running "Install all" (disables that header button).
  let busyCategory: string | null = $state(null);

  // Preferred category ordering; anything else falls in afterwards, alphabetically.
  const CATEGORY_ORDER = [
    'product',
    'project',
    'development',
    'review',
    'design',
    'insights',
  ];

  // Skills grouped by category, in CATEGORY_ORDER then alphabetical for the rest.
  const groups = $derived.by(() => {
    const byCat = new Map<string, BundledSkill[]>();
    for (const s of skills) {
      const list = byCat.get(s.category) ?? [];
      list.push(s);
      byCat.set(s.category, list);
    }
    const cats = [...byCat.keys()].sort((a, b) => {
      const ia = CATEGORY_ORDER.indexOf(a);
      const ib = CATEGORY_ORDER.indexOf(b);
      if (ia !== -1 && ib !== -1) return ia - ib;
      if (ia !== -1) return -1;
      if (ib !== -1) return 1;
      return a.localeCompare(b);
    });
    return cats.map((cat) => ({
      category: cat,
      skills: (byCat.get(cat) ?? []).sort((a, b) => a.name.localeCompare(b.name)),
    }));
  });

  // ---------------------------------------------------------------------------
  // Load
  // ---------------------------------------------------------------------------

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      skills = await contextApi.listBundled();
    } catch (e) {
      toasts.error('Could not load skills', e instanceof Error ? e.message : String(e));
      skills = [];
    } finally {
      loading = false;
    }
  }

  function setBusy(name: string, on: boolean): void {
    const next = new Set(busy);
    if (on) next.add(name);
    else next.delete(name);
    busy = next;
  }

  // ---------------------------------------------------------------------------
  // Badge / action labels per state
  // ---------------------------------------------------------------------------

  function badge(s: BundledSkill): { text: string; cls: string } {
    switch (s.state) {
      case 'not_installed':
        return { text: 'Not installed', cls: '' };
      case 'up_to_date':
        return { text: `Installed v${s.installed_version}`, cls: 'ok' };
      case 'update_available':
        return {
          text: `Update available v${s.installed_version}→v${s.version}`,
          cls: 'accent',
        };
      case 'ahead':
        return { text: 'Edited (ahead)', cls: 'bad' };
      default:
        return { text: s.state, cls: '' };
    }
  }

  // ---------------------------------------------------------------------------
  // Install / update one skill
  // ---------------------------------------------------------------------------

  async function install(s: BundledSkill): Promise<void> {
    // "ahead" means the installed copy was hand-edited and is newer than the
    // bundled one — updating discards those edits (after a backup). Make the
    // keep-old-vs-sync choice explicit; doing nothing keeps the edited copy.
    if (s.state === 'ahead') {
      const ok = await confirmer.ask(
        `Your installed copy of “${s.name}” was edited and is ahead of the bundled version. ` +
          `Updating backs it up first, then replaces it with the bundled skill. ` +
          `Do nothing to keep your edited copy.`,
        { title: 'Replace edited skill?', confirmLabel: 'Back up & replace', danger: true },
      );
      if (!ok) return;
    }

    setBusy(s.name, true);
    try {
      const resp = await contextApi.installBundled(s.name);
      if (resp.backed_up) {
        toasts.success(
          'Backed up & updated',
          resp.backup_path
            ? `${s.name} — previous copy saved to ${resp.backup_path}`
            : `${s.name} — previous copy backed up`,
        );
      } else {
        toasts.success('Installed', s.name);
      }
      await load();
    } catch (e) {
      toasts.error('Install failed', e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(s.name, false);
    }
  }

  // ---------------------------------------------------------------------------
  // Remove an installed skill
  // ---------------------------------------------------------------------------

  async function remove(s: BundledSkill): Promise<void> {
    if (
      !(await confirmer.ask(`Remove the installed skill “${s.name}” from the library?`, {
        title: 'Remove skill',
        confirmLabel: 'Remove',
      }))
    )
      return;
    setBusy(s.name, true);
    try {
      await contextApi.deleteSkill(s.name);
      toasts.info('Removed', s.name);
      await load();
    } catch (e) {
      toasts.error('Remove failed', e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(s.name, false);
    }
  }

  // ---------------------------------------------------------------------------
  // Install all in a category
  // ---------------------------------------------------------------------------

  async function installAll(category: string): Promise<void> {
    busyCategory = category;
    try {
      const resp = await contextApi.installAllBundled(category);
      const n = resp.installed.length;
      const b = resp.backed_up.length;
      if (n === 0) {
        toasts.info('Nothing to install', `All ${category} skills are already up to date.`);
      } else if (b > 0) {
        toasts.success(
          `Installed ${n} ${category} skill${n === 1 ? '' : 's'}`,
          `${b} existing cop${b === 1 ? 'y was' : 'ies were'} backed up first.`,
        );
      } else {
        toasts.success(`Installed ${n} ${category} skill${n === 1 ? '' : 's'}`);
      }
      await load();
    } catch (e) {
      toasts.error('Install all failed', e instanceof Error ? e.message : String(e));
    } finally {
      busyCategory = null;
    }
  }

  // True when a category has at least one skill that would change on "Install all".
  function categoryHasWork(skillsInCat: BundledSkill[]): boolean {
    return skillsInCat.some((s) => s.state !== 'up_to_date');
  }

  // Per-state action button label.
  function actionLabel(state: BundledSkillState, name: string): string {
    if (busy.has(name)) return '…';
    return state === 'not_installed' ? 'Install' : 'Update';
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Skills</h1>
      <div class="sub">
        Skills that ship with Otto. Install or update them into your library — your edited copies are
        always backed up before being replaced.
      </div>
    </div>
  </div>

  {#if loading}
    <Skeleton rows={4} height={52} />
  {:else if skills.length === 0}
    <EmptyState
      icon="box"
      title="No bundled skills"
      body="Otto did not ship any skills, or the catalogue could not be loaded."
    />
  {:else}
    {#each groups as g (g.category)}
      <section class="cat">
        <div class="cat-head">
          <span class="cat-title">{g.category}</span>
          <button
            class="btn small"
            disabled={busyCategory === g.category || !categoryHasWork(g.skills)}
            onclick={() => installAll(g.category)}
          >
            {busyCategory === g.category ? 'Installing…' : 'Install all in category'}
          </button>
        </div>

        <div class="skill-list">
          {#each g.skills as s (s.name)}
            {@const b = badge(s)}
            <div class="skill card">
              <div class="grow">
                <div class="skill-name">
                  <span class="mono">{s.name}</span>
                  <span class="chip {b.cls}">{b.text}</span>
                </div>
                {#if s.description}
                  <div class="skill-desc dim">{s.description}</div>
                {/if}
                {#if s.state === 'update_available'}
                  <div class="note dim">Your installed copy is backed up first.</div>
                {:else if s.state === 'ahead'}
                  <div class="note warn">
                    Your copy was edited — updating backs it up, then replaces it. Doing nothing keeps
                    your edited copy.
                  </div>
                {/if}
              </div>

              <div class="skill-actions">
                {#if s.state === 'up_to_date'}
                  <button
                    class="btn small"
                    disabled={busy.has(s.name)}
                    onclick={() => remove(s)}
                  >
                    {busy.has(s.name) ? '…' : 'Remove'}
                  </button>
                {:else}
                  <button
                    class="btn small primary"
                    class:warn-btn={s.state === 'ahead'}
                    disabled={busy.has(s.name)}
                    onclick={() => install(s)}
                  >
                    {actionLabel(s.state, s.name)}
                  </button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/each}
  {/if}
</div>

<style>
  .cat {
    max-width: 720px;
    margin-bottom: 22px;
  }
  .cat-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 8px;
  }
  .cat-title {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .skill-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .skill {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 14px;
  }
  .skill-name {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 600;
    flex-wrap: wrap;
  }
  .skill-desc {
    font-size: 11.5px;
    margin-top: 3px;
    line-height: 1.4;
  }
  .note {
    font-size: 11px;
    margin-top: 4px;
    line-height: 1.4;
  }
  .note.warn {
    color: var(--status-exited);
  }

  .skill-actions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  /* "ahead" update is a replace-after-backup — tint it like a warning. */
  .btn.warn-btn {
    background: var(--status-exited);
  }
  .btn.warn-btn:hover {
    background: color-mix(in srgb, var(--status-exited) 88%, black);
  }
</style>
