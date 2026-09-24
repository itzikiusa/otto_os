<script lang="ts">
  // "New skill" sheet with a small template picker:
  //   • Blank           — a SKILL.md scaffold (frontmatter + When to use / Method)
  //   • From bundled    — start from a bundled skill's SKILL.md under a new name
  //   • Import a file   — a skill package .zip (the daemon unpacks it)
  // Importing from a URL isn't offered: the daemon has no fetch-a-package
  // endpoint, and fetching arbitrary URLs from the webview would bypass the
  // SSRF guard.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { skillLabApi } from '../../lib/api/skillLab';
  import type { BundledSkillView, LibrarySkill } from '../../lib/api/types';

  type Template = 'blank' | 'bundled' | 'import';

  interface Props {
    bundled: BundledSkillView[];
    categories: string[];
    /** Existing names, to catch a clash before the daemon's 409. */
    taken: Set<string>;
    initial?: Template;
    onclose: () => void;
    oncreated: (s: LibrarySkill) => void;
  }
  let { bundled, categories, taken, initial = 'blank', onclose, oncreated }: Props = $props();

  // The sheet is re-created per open, so the initial template is read once.
  // svelte-ignore state_referenced_locally
  let template = $state<Template>(initial);
  let name = $state('');
  let category = $state('development');
  let description = $state('');
  let fromBundled = $state('');
  let file = $state<File | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let touched = $state(false);

  const NAME_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
  const nameError = $derived.by(() => {
    if (template === 'import') return null;
    const n = name.trim();
    if (!n) return touched ? 'Give the skill a name.' : null;
    if (!NAME_RE.test(n)) return 'Use lowercase letters, digits and dashes (kebab-case), up to 64 characters.';
    if (taken.has(n)) return `A skill named "${n}" already exists in the library.`;
    return null;
  });
  const valid = $derived(
    template === 'import' ? !!file : !!name.trim() && !nameError && (template !== 'bundled' || !!fromBundled),
  );

  function pickBundled(n: string): void {
    fromBundled = n;
    const b = bundled.find((x) => x.name === n);
    if (b) {
      if (!name.trim()) name = `${b.name}-custom`;
      category = b.category || category;
      if (!description.trim()) description = b.description;
    }
  }

  function blankBody(): string {
    const n = name.trim();
    const d = description.trim() || 'What this skill does and when an agent should use it.';
    return `---\nname: ${n}\ndescription: ${d}\ncategory: ${category.trim() || 'development'}\nversion: 1\n---\n\n# ${n}\n\n## When to use\n\n${d}\n\n## Method\n\n1. \n2. \n3. \n\n## Output\n\nWhat the agent hands back, and in what shape.\n`;
  }

  async function create(): Promise<void> {
    touched = true;
    if (!valid || busy) return;
    busy = true;
    error = null;
    try {
      let s: LibrarySkill;
      if (template === 'import' && file) {
        s = await skillLabApi.importZip(file);
      } else if (template === 'bundled') {
        const src = await skillLabApi.getBundled(fromBundled);
        // Keep the method, rename it: rewrite `name:`/`description:` when present.
        let body = src.body.replace(/^name:.*$/m, `name: ${name.trim()}`);
        if (description.trim()) body = body.replace(/^description:.*$/m, `description: ${description.trim().replace(/\n/g, ' ')}`);
        s = await skillLabApi.create({ name: name.trim(), category: category.trim(), description: description.trim(), body });
      } else {
        s = await skillLabApi.create({ name: name.trim(), category: category.trim(), description: description.trim(), body: blankBody() });
      }
      oncreated(s);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      error = /409|exists|conflict/i.test(msg) ? `A skill with that name already exists. Choose another name.` : msg;
    } finally {
      busy = false;
    }
  }

  const TEMPLATES: { id: Template; icon: 'file' | 'box' | 'download'; title: string; body: string }[] = [
    { id: 'blank', icon: 'file', title: 'Blank', body: 'A SKILL.md scaffold: frontmatter, when to use, method, output.' },
    { id: 'bundled', icon: 'box', title: 'From a bundled skill', body: 'Copy one of Otto’s skills under a new name and adapt it.' },
    { id: 'import', icon: 'download', title: 'Import a file', body: 'A skill package (.zip) with SKILL.md at its root.' },
  ];
</script>

<Modal title="New skill" width={560} {onclose}>
  <div class="templates" role="radiogroup" aria-label="Start from">
    {#each TEMPLATES as t (t.id)}
      <button
        class="tpl"
        class:active={template === t.id}
        role="radio"
        aria-checked={template === t.id}
        onclick={() => (template = t.id)}
        data-testid="tpl-{t.id}"
      >
        <span class="tpl-icon"><Icon name={t.icon} size={16} /></span>
        <span class="tpl-title">{t.title}</span>
        <span class="tpl-body">{t.body}</span>
      </button>
    {/each}
  </div>

  {#if template === 'import'}
    <div class="field">
      <label for="ns-file">Skill package</label>
      <input id="ns-file" class="input file" type="file" accept=".zip" onchange={(e) => (file = (e.currentTarget as HTMLInputElement).files?.[0] ?? null)} />
      <span class="hint">The skill's name comes from the package. Existing skills are never overwritten.</span>
    </div>
  {:else}
    {#if template === 'bundled'}
      <div class="field">
        <label for="ns-src">Bundled skill</label>
        <select id="ns-src" class="input" value={fromBundled} onchange={(e) => pickBundled((e.currentTarget as HTMLSelectElement).value)}>
          <option value="" disabled>Choose a skill…</option>
          {#each bundled as b (b.name)}<option value={b.name}>{b.name} · {b.category}</option>{/each}
        </select>
      </div>
    {/if}
    <div class="field">
      <label for="ns-name">Name</label>
      <input
        id="ns-name"
        class="input mono"
        bind:value={name}
        onblur={() => (touched = true)}
        placeholder="release-notes-writer"
        aria-invalid={!!nameError}
        aria-describedby="ns-name-hint"
        data-testid="new-skill-name"
      />
      {#if nameError}
        <span class="err" id="ns-name-hint">{nameError}</span>
      {:else}
        <span class="hint" id="ns-name-hint">Kebab-case. Agents load it by this name.</span>
      {/if}
    </div>
    <div class="row2">
      <div class="field">
        <label for="ns-cat">Category</label>
        <input id="ns-cat" class="input" list="ns-cats" bind:value={category} placeholder="review" />
        <datalist id="ns-cats">{#each categories as c (c)}<option value={c}></option>{/each}</datalist>
      </div>
    </div>
    <div class="field">
      <label for="ns-desc">Description</label>
      <textarea id="ns-desc" class="input" rows="2" bind:value={description} placeholder="Draft release notes from the PRs merged since the last tag."></textarea>
      <span class="hint">Agents read this to decide when to use the skill — say what it does and when.</span>
    </div>
  {/if}
  {#if error}<p class="err" role="alert">Couldn't create the skill. {error}</p>{/if}

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={create} disabled={!valid || busy} data-testid="create-skill">
      {busy ? (template === 'import' ? 'Importing…' : 'Creating…') : template === 'import' ? 'Import skill' : 'Create skill'}
    </button>
  {/snippet}
</Modal>

<style>
  .templates {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
    margin-bottom: 16px;
  }
  .tpl {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    text-align: start;
    cursor: pointer;
    font: inherit;
  }
  .tpl:hover {
    background: var(--hover);
  }
  .tpl.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .tpl-icon {
    display: inline-flex;
    color: var(--text-dim);
  }
  .tpl.active .tpl-icon {
    color: var(--accent-text);
  }
  .tpl-title {
    font-weight: 600;
    font-size: var(--fs-m);
  }
  .tpl-body {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.4;
  }
  .field {
    margin-bottom: 12px;
  }
  .row2 {
    display: grid;
    grid-template-columns: 1fr;
  }
  textarea.input {
    height: auto;
    padding-block: 6px;
    resize: vertical;
    font: inherit;
  }
  .input.file {
    height: auto;
    padding-block: 4px;
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-s);
    margin: 0;
  }
  @media (max-width: 640px) {
    .templates {
      grid-template-columns: 1fr;
    }
  }
</style>
