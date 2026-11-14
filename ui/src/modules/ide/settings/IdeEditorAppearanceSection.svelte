<script lang="ts">
  // Appearance, Keymap, Editor (Font, Color Scheme, Rainbow Brackets, Inlay Hints,
  // Inspections), and Version Control (Git & Local History) sections.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { ide } from '../ideStore.svelte';
  import { EDITOR_SCHEMES, ideSettings } from './ideSettingsStore.svelte';

  interface Props {
    section: string;
  }

  let { section }: Props = $props();

  interface KeymapRow {
    action: string;
    category: string;
    jetbrains: string;
    standard: string;
  }

  const KEYMAP_ROWS: readonly KeymapRow[] = [
    { action: 'Settings & Project Structure', category: 'IDE', jetbrains: '⌘,', standard: '⌘,' },
    { action: 'Search Everywhere', category: 'Navigate', jetbrains: '⇧⇧', standard: '⌘P' },
    { action: 'Recent Files', category: 'Navigate', jetbrains: '⌘E', standard: '⌃R' },
    { action: 'Go to Definition / Usages', category: 'Code', jetbrains: '⌘B', standard: 'F12' },
    { action: 'Find Usages', category: 'Code', jetbrains: '⌥F7', standard: '⇧F12' },
    { action: 'Rename Symbol', category: 'Refactor', jetbrains: '⇧F6', standard: 'F2' },
    { action: 'Quick Fix / Show Intentions', category: 'Code', jetbrains: '⌥⏎', standard: '⌘.' },
    { action: 'Reformat Code', category: 'Code', jetbrains: '⌥⌘L', standard: '⇧⌥F' },
    { action: 'Toggle Line Comment', category: 'Editor', jetbrains: '⌘/', standard: '⌘/' },
    { action: 'Duplicate Line / Selection', category: 'Editor', jetbrains: '⌘D', standard: '⇧⌥↓' },
    { action: 'Delete Line', category: 'Editor', jetbrains: '⌘⌫', standard: '⇧⌘K' },
    { action: 'File Structure Popup', category: 'Navigate', jetbrains: '⌘F12', standard: '⇧⌘O' },
    { action: 'Go to Line : Column', category: 'Navigate', jetbrains: '⌘G / ⌃G', standard: '⌃G' },
    { action: 'Navigate Back / Forward', category: 'Navigate', jetbrains: '⌘[ / ⌘]', standard: '⌃- / ⌃⇧-' },
    { action: 'Close Active Editor Tab', category: 'Tabs', jetbrains: '⌘W', standard: '⌘W' },
    { action: 'Reopen Closed Editor Tab', category: 'Tabs', jetbrains: '⌘⇧T', standard: '⌘⇧T' },
    { action: 'Toggle Project Tool Window', category: 'Windows', jetbrains: '⌘1', standard: '⌘B' },
    { action: 'Run / Debug / Problems / Git Dock', category: 'Windows', jetbrains: '⌘4 / ⌘5 / ⌘6 / ⌘9', standard: '⌘J' },
    { action: 'Terminal Tool Window', category: 'Windows', jetbrains: '⌥F12', standard: '⌃`' },
    { action: 'Run / Debug Selected Target', category: 'Run', jetbrains: '⌃R / ⌃D', standard: 'F5' },
    { action: 'Stop Active Run or Debug', category: 'Run', jetbrains: '⌘F2', standard: '⇧F5' },
    { action: 'Toggle Line Breakpoint', category: 'Debugger', jetbrains: '⌘F8', standard: 'F9' },
    { action: 'Step Over / Step Into / Step Out', category: 'Debugger', jetbrains: 'F8 / F7 / ⇧F8', standard: 'F10 / F11 / ⇧F11' },
    { action: 'Resume Program / Run to Cursor', category: 'Debugger', jetbrains: 'F9 / ⌥F9', standard: 'F5 / ⌥F9' },
  ];

  let keymapQuery = $state('');

  const filteredKeymap = $derived.by(() => {
    const q = keymapQuery.trim().toLowerCase();
    if (!q) return KEYMAP_ROWS;
    return KEYMAP_ROWS.filter(
      (r) =>
        r.action.toLowerCase().includes(q) ||
        r.category.toLowerCase().includes(q) ||
        r.jetbrains.toLowerCase().includes(q) ||
        r.standard.toLowerCase().includes(q),
    );
  });

  const previewScheme = $derived(
    ideSettings.draft.colorScheme === 'auto'
      ? ui.resolvedScheme === 'light'
        ? ideSettings.draft.preferredLightScheme
        : ideSettings.draft.preferredDarkScheme
      : ideSettings.draft.colorScheme,
  );

  const previewIsDark = $derived(
    previewScheme === 'jetbrains-islands-dark' || previewScheme === 'jetbrains-darcula' || previewScheme === 'vscode-dark-modern',
  );

  function openProjectLocalHistory(): void {
    ide.settingsOpen = false;
    if (typeof window !== 'undefined') {
      const detail = { mode: 'local-history', root: ide.target.root, path: ide.activePath || ide.target.root };
      window.dispatchEvent(new CustomEvent('otto:ide-open-local-history', { detail }));
      window.dispatchEvent(new CustomEvent('otto:ide-open-compare', { detail }));
    }
  }
</script>

{#if section === 'appearance.ui'}
  <div class="ea-sec" data-testid="ide-settings-appearance">
    <div class="ea-card">
      <label class="ea-lbl" for="ea-scheme-sel">Editor &amp; IDE Color Theme:</label>
      <select id="ea-scheme-sel" class="input" bind:value={ideSettings.draft.colorScheme} data-testid="ide-color-scheme-select">
        {#each EDITOR_SCHEMES as sc (sc.id)}
          <option value={sc.id}>{sc.label}</option>
        {/each}
      </select>
      <p class="ea-sub">{EDITOR_SCHEMES.find((s) => s.id === ideSettings.draft.colorScheme)?.desc}</p>

      {#if ideSettings.draft.colorScheme === 'auto'}
        <div class="ea-grid-2">
          <div class="ea-field">
            <label class="ea-lbl" for="ea-pref-dark">Preferred Dark Scheme:</label>
            <select id="ea-pref-dark" class="input" bind:value={ideSettings.draft.preferredDarkScheme}>
              <option value="jetbrains-islands-dark">JetBrains Islands Dark</option>
              <option value="jetbrains-darcula">JetBrains Darcula</option>
              <option value="vscode-dark-modern">VS Code Dark Modern</option>
            </select>
          </div>
          <div class="ea-field">
            <label class="ea-lbl" for="ea-pref-light">Preferred Light Scheme:</label>
            <select id="ea-pref-light" class="input" bind:value={ideSettings.draft.preferredLightScheme}>
              <option value="jetbrains-islands-light">JetBrains Islands Light</option>
              <option value="vscode-light-modern">VS Code Light Modern</option>
            </select>
          </div>
        </div>
      {/if}
    </div>

    <div class="ea-grid-2">
      <div class="ea-card">
        <span class="ea-title">UI Density</span>
        <div class="segmented" role="group" aria-label="IDE UI density">
          <button
            type="button"
            aria-pressed={ideSettings.draft.uiDensity === 'comfortable'}
            onclick={() => (ideSettings.draft.uiDensity = 'comfortable')}
          >
            Comfortable
          </button>
          <button
            type="button"
            aria-pressed={ideSettings.draft.uiDensity === 'compact'}
            onclick={() => (ideSettings.draft.uiDensity = 'compact')}
          >
            Compact
          </button>
        </div>
      </div>

      <div class="ea-card">
        <span class="ea-title">Debugger Default Layout</span>
        <div class="segmented" role="group" aria-label="Default debugger layout">
          <button
            type="button"
            aria-pressed={ideSettings.draft.debuggerLayout === 'tabbed'}
            onclick={() => (ideSettings.draft.debuggerLayout = 'tabbed')}
          >
            Tabbed (Variables | Console)
          </button>
          <button
            type="button"
            aria-pressed={ideSettings.draft.debuggerLayout === 'split'}
            onclick={() => (ideSettings.draft.debuggerLayout = 'split')}
          >
            Split Side-by-Side
          </button>
        </div>
      </div>
    </div>
  </div>

{:else if section === 'keymap'}
  <div class="ea-sec" data-testid="ide-settings-keymap">
    <div class="ea-toolbar">
      <div class="ea-field ea-grow">
        <label class="ea-lbl" for="ea-km-preset">Keymap Preset:</label>
        <select id="ea-km-preset" class="input" bind:value={ideSettings.draft.keymapPreset} data-testid="ide-keymap-preset-select">
          <option value="jetbrains">JetBrains / IntelliJ IDEA / GoLand</option>
          <option value="standard">VS Code / Standard macOS</option>
        </select>
      </div>
      <div class="ea-field ea-grow">
        <label class="ea-lbl" for="ea-km-filter">Filter shortcuts:</label>
        <input
          id="ea-km-filter"
          class="input"
          type="search"
          dir="ltr"
          placeholder="Search action or key (e.g. F8, rename, git)…"
          bind:value={keymapQuery}
          data-testid="ide-keymap-search"
        />
      </div>
    </div>

    <div class="km-table-wrap">
      <table class="km-table">
        <thead>
          <tr>
            <th>Action</th>
            <th>Category</th>
            <th>JetBrains / GoLand</th>
            <th>VS Code / Standard</th>
          </tr>
        </thead>
        <tbody>
          {#each filteredKeymap as row (row.action)}
            <tr>
              <td class="km-act">{row.action}</td>
              <td><Badge tone="neutral" variant="outline" label={row.category} /></td>
              <td><code class="km-kbd" class:is-selected={ideSettings.draft.keymapPreset === 'jetbrains'}>{row.jetbrains}</code></td>
              <td><code class="km-kbd" class:is-selected={ideSettings.draft.keymapPreset === 'standard'}>{row.standard}</code></td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>

{:else if section === 'editor.font'}
  <div class="ea-sec" data-testid="ide-settings-font">
    <div class="ea-card ea-grid-2">
      <div class="ea-field">
        <label class="ea-lbl" for="ea-font-size">Editor font size ({ideSettings.draft.fontSize} px):</label>
        <input id="ea-font-size" type="range" min="11" max="22" step="1" bind:value={ideSettings.draft.fontSize} />
      </div>
      <div class="ea-field">
        <label class="ea-lbl" for="ea-line-h">Line spacing ({ideSettings.draft.lineHeight.toFixed(2)}):</label>
        <input id="ea-line-h" type="range" min="1.3" max="1.9" step="0.05" bind:value={ideSettings.draft.lineHeight} />
      </div>
    </div>

    <div class="ea-card">
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.fontLigatures} label="Enable font ligatures" onchange={(v) => (ideSettings.draft.fontLigatures = v)} />
        <span class="ea-title">Enable typographic font ligatures (<code>!=</code>, <code>:=</code>, <code>-&gt;</code>)</span>
      </div>
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.formatOnSave} label="Reformat file on save" onchange={(v) => (ideSettings.draft.formatOnSave = v)} />
        <span class="ea-title">Format code on save (<code>gofmt</code> / LSP formatter)</span>
      </div>
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.autoFoldImports} label="Auto-fold import blocks" onchange={(v) => (ideSettings.draft.autoFoldImports = v)} />
        <span class="ea-title">Collapse multi-line Go &amp; Java <code>import</code> blocks on open</span>
      </div>
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.breadcrumbs} label="Show symbol breadcrumbs" onchange={(v) => (ideSettings.draft.breadcrumbs = v)} />
        <span class="ea-title">Show path &amp; enclosing symbol breadcrumbs bar</span>
      </div>
    </div>
  </div>

{:else if section === 'editor.colors' || section === 'editor.rainbow'}
  <div class="ea-sec" data-testid="ide-settings-colors" data-preview-scheme={previewScheme}>
    <div class="ea-card">
      <label class="ea-lbl" for="ea-color-scheme">Scheme:</label>
      <select id="ea-color-scheme" class="input" bind:value={ideSettings.draft.colorScheme}>
        {#each EDITOR_SCHEMES as sc (sc.id)}
          <option value={sc.id}>{sc.label}</option>
        {/each}
      </select>

      <div class="ea-sw-row">
        <Switch
          checked={ideSettings.draft.rainbowBrackets}
          label="Enable Rainbow Brackets"
          onchange={(v) => (ideSettings.draft.rainbowBrackets = v)}
        />
        <span class="ea-title">Enable Rainbow Brackets — cycle 4 bracket colors by nesting depth for <code>()</code>, <code>[]</code>, and <code>&#123;&#125;</code></span>
      </div>
    </div>

    <div class="code-preview" class:otto-force-dark={previewIsDark} data-ide-scheme={previewScheme} dir="ltr" data-testid="ide-color-scheme-preview">
      <div class="cp-head">
        <span>Go &amp; Java Live Token Preview ({EDITOR_SCHEMES.find((s) => s.id === ideSettings.draft.colorScheme)?.label})</span>
        <Badge tone="accent" variant="outline" label={ideSettings.draft.rainbowBrackets ? 'Rainbow On' : 'Standard Brackets'} />
      </div>
      <pre class="cp-code"><span class="kw">package</span> service

<span class="kw">func</span> <span class="fn">LoadPromotion</span><span class:rb1={ideSettings.draft.rainbowBrackets}>(</span>ctx Context, id <span class="ty">int64</span><span class:rb1={ideSettings.draft.rainbowBrackets}>)</span> <span class:rb1={ideSettings.draft.rainbowBrackets}>&#123;</span>
    cfg := map<span class:rb2={ideSettings.draft.rainbowBrackets}>[</span><span class="ty">string</span><span class:rb2={ideSettings.draft.rainbowBrackets}>]</span><span class="ty">any</span><span class:rb2={ideSettings.draft.rainbowBrackets}>&#123;</span><span class="str">"limit"</span>: <span class="num">100</span>, <span class="str">"nested"</span>: <span class:rb3={ideSettings.draft.rainbowBrackets}>[</span><span class:rb4={ideSettings.draft.rainbowBrackets}>(</span><span class="num">42</span><span class:rb4={ideSettings.draft.rainbowBrackets}>)</span><span class:rb3={ideSettings.draft.rainbowBrackets}>]</span><span class:rb2={ideSettings.draft.rainbowBrackets}>&#125;</span>
    logger.<span class="fn">InfoF</span><span class:rb2={ideSettings.draft.rainbowBrackets}>(</span>ctx, <span class="str">"loaded %v"</span>, cfg<span class:rb2={ideSettings.draft.rainbowBrackets}>)</span>
<span class:rb1={ideSettings.draft.rainbowBrackets}>&#125;</span></pre>
    </div>
  </div>

{:else if section === 'editor.hints'}
  <div class="ea-sec" data-testid="ide-settings-hints">
    <div class="ea-card">
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.inlayHints} label="Show parameter name hints" onchange={(v) => (ideSettings.draft.inlayHints = v)} />
        <span class="ea-title">Show inline parameter name &amp; inferred type hints</span>
      </div>
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.codeVision} label="Show CodeVision usages" onchange={(v) => (ideSettings.draft.codeVision = v)} />
        <span class="ea-title">Show CodeVision symbol reference counts above functions &amp; types</span>
      </div>
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.compactHover} label="Compact Quick Documentation on hover" onchange={(v) => (ideSettings.draft.compactHover = v)} />
        <span class="ea-title">Compact Quick Documentation card on symbol hover</span>
      </div>
    </div>
  </div>

{:else if section === 'editor.inspections'}
  <div class="ea-sec" data-testid="ide-settings-inspections">
    <div class="ea-card">
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.javaUnresolvedImports} label="Inspect Java unresolved imports" onchange={(v) => (ideSettings.draft.javaUnresolvedImports = v)} />
        <span class="ea-title">Highlight unresolved Java imports &amp; offer Quick Fix import actions</span>
      </div>
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.lombokSupport} label="Enable Lombok annotation awareness" onchange={(v) => (ideSettings.draft.lombokSupport = v)} />
        <span class="ea-title">Lombok awareness (<code>@Slf4j</code>, <code>@Data</code>, <code>@Builder</code>, <code>@RequiredArgsConstructor</code>)</span>
      </div>
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.unusedImports} label="Highlight unused imports" onchange={(v) => (ideSettings.draft.unusedImports = v)} />
        <span class="ea-title">Dim unused imports across Go, Java, TypeScript, Rust, and Python</span>
      </div>
    </div>
  </div>

{:else if section === 'vcs.git'}
  <div class="ea-sec" data-testid="ide-settings-git">
    <div class="ea-card">
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.gitTreeBadges} label="Show Git status badges in Project tree" onchange={(v) => (ideSettings.draft.gitTreeBadges = v)} />
        <span class="ea-title">Color modified, staged, and untracked files in Project Explorer</span>
      </div>
      <div>
        <button
          type="button"
          class="btn small"
          onclick={() => { ide.settingsOpen = false; ide.openBottomTab('git'); }}
        >
          <Icon name="branch" size={12} />
          Open IDE Git Tool Window (⌘9)
        </button>
      </div>
    </div>
  </div>

{:else}
  <div class="ea-sec" data-testid="ide-settings-local-history">
    <div class="ea-card">
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.localHistoryEnabled} label="Enable Local History" onchange={(v) => (ideSettings.draft.localHistoryEnabled = v)} />
        <span class="ea-title">Record automatic Local History snapshots on save, refactor, and agent edits</span>
      </div>
      <div class="ea-field">
        <label class="ea-lbl" for="ea-lh-days">Snapshot retention period ({ideSettings.draft.localHistoryRetentionDays} days):</label>
        <input id="ea-lh-days" type="range" min="1" max="60" step="1" bind:value={ideSettings.draft.localHistoryRetentionDays} />
      </div>
      <div>
        <button
          type="button"
          class="btn small primary"
          onclick={openProjectLocalHistory}
          data-testid="ide-open-local-history-btn"
        >
          <Icon name="clock" size={12} />
          Open Project Local History…
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .ea-sec { display: flex; flex-direction: column; gap: 14px; }
  .ea-card { border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  .ea-grid-2 { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; }
  .ea-toolbar { display: flex; align-items: flex-end; gap: 10px; flex-wrap: wrap; }
  .ea-grow { flex: 1; min-inline-size: 200px; }
  .ea-field { display: flex; flex-direction: column; gap: 4px; }
  .ea-lbl { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .ea-title { font-size: var(--fs-s); font-weight: 500; color: var(--text); }
  .ea-sub { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .ea-sw-row { display: flex; align-items: center; gap: 10px; }
  .km-table-wrap { max-block-size: 360px; overflow-y: auto; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); }
  .km-table { inline-size: 100%; border-collapse: collapse; font-size: var(--fs-xs); }
  .km-table th { position: sticky; top: 0; background: var(--surface-2); padding: 8px 10px; text-align: start; font-weight: 600; color: var(--text-dim); border-block-end: 1px solid var(--border); }
  .km-table td { padding: 6px 10px; border-block-end: 1px solid var(--separator); color: var(--text); }
  .km-act { font-weight: 500; }
  .km-kbd { font-family: var(--font-mono); padding: 2px 6px; border-radius: var(--radius-s); background: var(--surface-2); border: 1px solid var(--border); color: var(--text-dim); }
  .km-kbd.is-selected { background: var(--accent-soft); border-color: var(--accent-line); color: var(--accent-text); font-weight: 600; }
  .ea-sec[data-preview-scheme] { --syn-bg: var(--term-bg); --syn-fg: var(--text); --syn-keyword: var(--code-keyword); --syn-func-decl: var(--accent-text); --syn-string: var(--code-string); --syn-type: var(--info); --syn-number: var(--warning); --syn-rb-1: var(--warning); --syn-rb-2: var(--accent-text); --syn-rb-3: var(--info); --syn-rb-4: var(--success); }
  .ea-sec[data-preview-scheme='jetbrains-darcula'] { --syn-bg: var(--surface); --syn-keyword: var(--warning); --syn-func-decl: var(--cat-2); --syn-string: var(--cat-3); --syn-type: var(--cat-4); --syn-number: var(--cat-1); }
  .ea-sec[data-preview-scheme='vscode-dark-modern'] { --syn-bg: var(--term-bg); --syn-keyword: var(--cat-1); --syn-func-decl: var(--cat-2); --syn-string: var(--code-number); --syn-type: var(--cat-6); --syn-number: var(--cat-3); }
  .ea-sec[data-preview-scheme='jetbrains-islands-light'], .ea-sec[data-preview-scheme='vscode-light-modern'] { --syn-bg: var(--surface); --syn-fg: var(--text); --syn-keyword: var(--cat-4); --syn-func-decl: var(--cat-1); --syn-string: var(--cat-3); --syn-type: var(--cat-6); }
  .code-preview { border: 1px solid var(--border-strong); border-radius: var(--radius-s); background: var(--syn-bg); color: var(--syn-fg); overflow: hidden; }
  .cp-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 8px 12px; background: var(--surface-2); border-block-end: 1px solid var(--border); font-size: var(--fs-xs); font-weight: 600; }
  .cp-code { margin: 0; padding: 12px; font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.6; overflow-x: auto; }
  .kw { color: var(--syn-keyword); font-weight: 600; }
  .fn { color: var(--syn-func-decl); }
  .ty { color: var(--syn-type); }
  .str { color: var(--syn-string); }
  .num { color: var(--syn-number); }
  .rb1 { color: var(--syn-rb-1); font-weight: 600; }
  .rb2 { color: var(--syn-rb-2); font-weight: 600; }
  .rb3 { color: var(--syn-rb-3); font-weight: 600; }
  .rb4 { color: var(--syn-rb-4); font-weight: 600; }
</style>
