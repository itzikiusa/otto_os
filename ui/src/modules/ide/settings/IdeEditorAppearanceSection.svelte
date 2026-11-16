<script lang="ts">
  // Appearance, Keymap, Editor (Font, Color Scheme, Rainbow Brackets, Inlay Hints,
  // Inspections), and Version Control (Git & Local History) sections.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { ide } from '../ideStore.svelte';
  import { EDITOR_SCHEMES, ideSettings } from './ideSettingsStore.svelte';
  import { IDE_SHORTCUTS, ideShortcutLabels } from '../shell/ideShortcuts';

  interface Props {
    section: string;
  }

  let { section }: Props = $props();

  // Generated from the one shortcut table the key handler also reads
  // (shell/ideShortcuts.ts), so a row can never list a key that does nothing.
  const KEYMAP_ROWS = IDE_SHORTCUTS.map((r) => ({
    id: r.id,
    action: r.title,
    category: r.category,
    jetbrains: ideShortcutLabels(r.id, 'jetbrains'),
    standard: ideShortcutLabels(r.id, 'standard'),
  }));

  interface PreviewPalette {
    bg: string; fg: string; kw: string; fn: string; ty: string; str: string; num: string; fld: string;
    rb1: string; rb2: string; rb3: string; rb4: string;
  }

  const PREVIEW_PALETTES: Record<string, PreviewPalette> = {
    'jetbrains-islands-dark': {
      bg: '#191A1C', fg: '#BCBEC4', kw: '#CF8E6D', fn: '#56A8F5', ty: '#56A8F5',
      str: '#6AAB73', num: '#2AACB8', fld: '#C77DBB',
      rb1: '#E8BA36', rb2: '#56A8F5', rb3: '#6AAB73', rb4: '#C77DBB',
    },
    'jetbrains-darcula': {
      bg: '#2B2B2B', fg: '#A9B7C6', kw: '#CC7832', fn: '#FFC66D', ty: '#A9B7C6',
      str: '#6A8759', num: '#6897BB', fld: '#9876AA',
      rb1: '#FFC66D', rb2: '#6897BB', rb3: '#6A8759', rb4: '#9876AA',
    },
    'vscode-dark-modern': {
      bg: '#1F1F1F', fg: '#CCCCCC', kw: '#569CD6', fn: '#DCDCAA', ty: '#4EC9B0',
      str: '#CE9178', num: '#B5CEA8', fld: '#9CDCFE',
      rb1: '#FFD700', rb2: '#DA70D6', rb3: '#179FFF', rb4: '#4EC9B0',
    },
    'jetbrains-islands-light': {
      bg: '#FFFFFF', fg: '#080808', kw: '#0033B3', fn: '#00627A', ty: '#0033B3',
      str: '#067D17', num: '#1750EB', fld: '#871094',
      rb1: '#B36B00', rb2: '#0057B8', rb3: '#067D17', rb4: '#871094',
    },
    'vscode-light-modern': {
      bg: '#FFFFFF', fg: '#3B3B3B', kw: '#0000FF', fn: '#795E26', ty: '#267F99',
      str: '#A31515', num: '#098658', fld: '#001080',
      rb1: '#0431FA', rb2: '#319331', rb3: '#7B3814', rb4: '#001080',
    },
  };

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
  const activePreviewPalette = $derived(PREVIEW_PALETTES[previewScheme] ?? PREVIEW_PALETTES['jetbrains-islands-dark']);

  function openProjectLocalHistory(): void {
    ide.settingsOpen = false;
    if (typeof window !== 'undefined') {
      const detail = { mode: 'local-history', root: ide.target.root, path: ide.activePath || ide.target.root };
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
      <div class="ea-field">
        <span class="ea-lbl" id="ea-km-preset">Keymap preset</span>
        <div class="segmented" role="group" aria-labelledby="ea-km-preset" data-testid="ide-keymap-preset-select">
          <button
            type="button"
            aria-pressed={ideSettings.draft.keymapPreset === 'jetbrains'}
            onclick={() => (ideSettings.draft.keymapPreset = 'jetbrains')}
            data-testid="ide-keymap-preset-jetbrains"
          >JetBrains</button>
          <button
            type="button"
            aria-pressed={ideSettings.draft.keymapPreset === 'standard'}
            onclick={() => (ideSettings.draft.keymapPreset = 'standard')}
            data-testid="ide-keymap-preset-standard"
          >VS Code / Standard</button>
        </div>
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
          {#each filteredKeymap as row (row.id)}
            <tr data-testid="ide-keymap-row" data-shortcut={row.id}>
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
      <div class="ea-field">
        <label class="ea-lbl" for="ea-max-tabs">Maximum open editor tabs ({ideSettings.draft.maxOpenTabs}):</label>
        <input id="ea-max-tabs" type="range" min="15" max="40" step="1" bind:value={ideSettings.draft.maxOpenTabs} data-testid="ide-max-open-tabs-range" />
      </div>
    </div>

    <div class="ea-card">
      <div class="ea-sw-row">
        <Switch checked={ideSettings.draft.autoSave} label="Auto-save modified files" onchange={(v) => (ideSettings.draft.autoSave = v)} />
        <span class="ea-title">Automatically save modified files on idle, tab switch, focus loss, and before Run / Debug</span>
      </div>
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

    <div
      class="code-preview"
      class:otto-force-dark={previewIsDark}
      data-ide-scheme={previewScheme}
      dir="ltr"
      style:--syn-bg={activePreviewPalette.bg}
      style:--syn-fg={activePreviewPalette.fg}
      style:--syn-keyword={activePreviewPalette.kw}
      style:--syn-func-decl={activePreviewPalette.fn}
      style:--syn-type={activePreviewPalette.ty}
      style:--syn-string={activePreviewPalette.str}
      style:--syn-number={activePreviewPalette.num}
      style:--syn-field={activePreviewPalette.fld}
      style:--syn-rb-1={activePreviewPalette.rb1}
      style:--syn-rb-2={activePreviewPalette.rb2}
      style:--syn-rb-3={activePreviewPalette.rb3}
      style:--syn-rb-4={activePreviewPalette.rb4}
      data-testid="ide-color-scheme-preview"
    >
      <div class="cp-head">
        <span>Go &amp; Java Live Token Preview ({EDITOR_SCHEMES.find((s) => s.id === ideSettings.draft.colorScheme)?.label})</span>
        <Badge tone="accent" variant="outline" label={ideSettings.draft.rainbowBrackets ? 'Rainbow On' : 'Standard Brackets'} />
      </div>
      <pre class="cp-code"><span class="kw">package</span> service

<span class="kw">func</span> <span class="fn">LoadPromotion</span><span class:rb1={ideSettings.draft.rainbowBrackets}>(</span>ctx Context, id <span class="ty">int64</span><span class:rb1={ideSettings.draft.rainbowBrackets}>)</span> <span class:rb1={ideSettings.draft.rainbowBrackets}>&#123;</span>
    cfg := map<span class:rb2={ideSettings.draft.rainbowBrackets}>[</span><span class="ty">string</span><span class:rb2={ideSettings.draft.rainbowBrackets}>]</span><span class="ty">any</span><span class:rb2={ideSettings.draft.rainbowBrackets}>&#123;</span><span class="str">"limit"</span>: <span class="num">100</span>, <span class="str">"nested"</span>: <span class:rb3={ideSettings.draft.rainbowBrackets}>[</span><span class:rb4={ideSettings.draft.rainbowBrackets}>(</span><span class="num">42</span><span class:rb4={ideSettings.draft.rainbowBrackets}>)</span><span class:rb3={ideSettings.draft.rainbowBrackets}>]</span><span class:rb2={ideSettings.draft.rainbowBrackets}>&#125;</span>
    s.<span class="fld">bonusRepo</span>.<span class="fn">InfoF</span><span class:rb2={ideSettings.draft.rainbowBrackets}>(</span>ctx, <span class="str">"loaded %v"</span>, cfg<span class:rb2={ideSettings.draft.rainbowBrackets}>)</span>
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
  .code-preview { border: 1px solid var(--border-strong); border-radius: var(--radius-s); background: var(--syn-bg); color: var(--syn-fg); overflow: hidden; }
  .cp-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 8px 12px; background: var(--surface-2); border-block-end: 1px solid var(--border); color: var(--text); font-size: var(--fs-xs); font-weight: 600; }
  .cp-code { margin: 0; padding: 12px; font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.6; overflow-x: auto; }
  .kw { color: var(--syn-keyword); font-weight: 600; }
  .fn { color: var(--syn-func-decl); }
  .ty { color: var(--syn-type); }
  .str { color: var(--syn-string); }
  .num { color: var(--syn-number); }
  .fld { color: var(--syn-field); }
  .rb1 { color: var(--syn-rb-1); font-weight: 600; }
  .rb2 { color: var(--syn-rb-2); font-weight: 600; }
  .rb3 { color: var(--syn-rb-3); font-weight: 600; }
  .rb4 { color: var(--syn-rb-4); font-weight: 600; }
</style>
