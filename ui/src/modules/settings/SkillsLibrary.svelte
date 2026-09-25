<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Settings → Skills: the catalogue of skills that ship with Otto ("bundled"),
  // shown grouped by category with their state relative to the installed library
  // copy. Installing/updating writes into the Otto library; the backend always
  // backs up an existing copy before overwriting (backup=true), so nothing is
  // ever destroyed silently. It ALSO materializes a copy into each provider CLI's
  // global skills dir (~/.claude/skills, $CODEX_HOME/skills, ~/.gemini/skills) so
  // the skill is discoverable everywhere; Remove reconciles those too (it only
  // ever touches skills Otto's per-dir manifest owns). Install/update/install-all
  // are root-only on the server; this page is gated to root in Settings.svelte
  // (like Context Library). The "Update" button (shown when state is
  // `update_available`/`ahead`) posts to the same install endpoint.
  import { contextApi } from '../../lib/api/context';
  import type { BundledSkill, BundledSkillState } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let skills: BundledSkill[] = $state([]);
  let loading = $state(true);
  // A failed load is shown inline with Retry — never as "No bundled skills".
  let loadError = $state('');
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
      loadError = '';
    } catch (e) {
      // Keep the last good list (if any); LoadState shows a stale bar over it.
      loadError = loadErrorText(e);
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
          text: `Update available · v${s.installed_version} → v${s.version}`,
          cls: 'accent',
        };
      case 'ahead':
        return { text: 'Edited locally', cls: 'warn' };
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
      toasts.error(`Couldn’t install ${s.name}`, e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(s.name, false);
    }
  }

  // ---------------------------------------------------------------------------
  // Remove an installed skill
  // ---------------------------------------------------------------------------

  async function remove(s: BundledSkill): Promise<void> {
    if (
      !(await confirmer.ask(`Remove “${s.name}” from your library and from each agent CLI's skills folder? You can install it again from this page.`, {
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
      toasts.error(`Couldn’t remove ${s.name}`, e instanceof Error ? e.message : String(e));
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
      toasts.error(`Couldn’t install the ${category} skills`, e instanceof Error ? e.message : String(e));
    } finally {
      busyCategory = null;
    }
  }

  // True when a category has at least one skill that would change on "Install all".
  function categoryHasWork(skillsInCat: BundledSkill[]): boolean {
    return skillsInCat.some((s) => s.state !== 'up_to_date');
  }

  // Per-state action button label. "Replace…" asks first (edited copy).
  function actionLabel(state: BundledSkillState, name: string): string {
    if (busy.has(name)) return state === 'not_installed' ? 'Installing…' : 'Updating…';
    if (state === 'not_installed') return 'Install';
    return state === 'ahead' ? 'Replace…' : 'Update';
  }

  function installedCount(list: BundledSkill[]): number {
    return list.filter((s) => s.state !== 'not_installed').length;
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('skills')} subtitle="Skills that ship with Otto" />
  <PageBody width="readable">
  <SectionIntro>Installing a skill adds it to your library and to each agent CLI's global skills folder, so Claude, Codex and agy can all use it. Your edited copies are always backed up before being replaced.</SectionIntro>

  <LoadState what="bundled skills" {loading} error={loadError} empty={skills.length === 0} rows={5} onretry={() => void load()}>
    {#snippet emptyView()}
      <EmptyState
        variant="page"
        icon="box"
        title="No bundled skills"
        body="This build of Otto doesn't ship any skills. Add your own in Settings → Context library."
      />
    {/snippet}
    {#each groups as g (g.category)}
      <section class="cat" aria-labelledby={`cat-${g.category}`}>
        <div class="cat-head">
          <h2 class="section-title" id={`cat-${g.category}`}>{g.category}</h2>
          <span class="cat-count">{installedCount(g.skills)} of {g.skills.length} installed</span>
          <span class="grow"></span>
          <button
            class="btn small"
            disabled={busyCategory === g.category || !categoryHasWork(g.skills)}
            title={categoryHasWork(g.skills) ? `Install or update every ${g.category} skill` : `Every ${g.category} skill is up to date`}
            onclick={() => installAll(g.category)}
          >
            {busyCategory === g.category ? 'Installing…' : 'Install all'}
          </button>
        </div>

        <div class="skill-list">
          {#each g.skills as s (s.name)}
            {@const b = badge(s)}
            <div class="skill">
              <div class="grow">
                <div class="skill-name">
                  <span class="mono">{s.name}</span>
                  <span class="chip {b.cls}">{b.text}</span>
                </div>
                {#if s.description}
                  <div class="skill-desc dim" title={s.description}>{s.description}</div>
                {/if}
                {#if s.state === 'update_available'}
                  <div class="note dim">Your installed copy is backed up first.</div>
                {:else if s.state === 'ahead'}
                  <div class="note warn">
                    Your copy was edited — replacing backs it up first. Doing nothing keeps your edited copy.
                  </div>
                {/if}
              </div>

              <div class="skill-actions">
                {#if s.state === 'up_to_date'}
                  <button
                    class="btn small ghost"
                    disabled={busy.has(s.name)}
                    onclick={() => remove(s)}
                  >
                    {busy.has(s.name) ? 'Removing…' : 'Remove…'}
                  </button>
                {:else}
                  <button
                    class="btn small"
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
  .cat {
    max-width: 880px;
    margin-bottom: 24px;
  }
  .cat-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }
  .cat-head .section-title {
    margin: 0;
  }
  .cat-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  /* One bordered list per category (hairlines between rows), not a card per
     skill — denser, and matches the other settings lists. */
  .skill-list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow: hidden;
  }
  .skill {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
  }
  .skill + .skill {
    border-top: 1px solid var(--border);
  }
  .skill-name {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
    font-weight: 600;
    flex-wrap: wrap;
  }
  /* Two lines, full text in the tooltip — some descriptions run 6+ lines. */
  .skill-desc {
    font-size: var(--fs-s);
    margin-top: 3px;
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .dim {
    color: var(--text-dim);
  }
  .note {
    font-size: var(--fs-xs);
    margin-top: 4px;
    line-height: 1.4;
  }
  .note.warn {
    color: var(--warning);
  }
  .chip.warn {
    color: var(--warning);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
    background: var(--warning-soft);
  }
  .skill-actions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
  }
</style>
