<script lang="ts">
  // Left vertical tool rail. Picking an insert tool arms the editor (the next
  // pane click drops that node). Order leads with the high-frequency tools
  // (Select, Sticky, Text, Shape, Connector), then a divider, then the power
  // blocks (Mermaid, Code, JSON, Image, Frame, Freehand). Each button shows its
  // single-key shortcut in the tooltip.
  import Icon from '../../lib/components/Icon.svelte';
  import { SHAPE_VARIANTS, type Tool } from './tools';

  interface Props {
    activeTool: Tool;
    onpick: (t: Tool) => void;
  }
  let { activeTool, onpick }: Props = $props();

  let shapeMenu = $state(false);

  interface ToolBtn {
    tool: Tool;
    icon: string;
    label: string;
    key: string;
  }
  const top: ToolBtn[] = [
    { tool: 'select', icon: 'command', label: 'Select', key: 'V' },
    { tool: 'sticky', icon: 'note', label: 'Sticky note', key: 'S' },
    { tool: 'text', icon: 'edit', label: 'Text', key: 'T' },
  ];
  const power: ToolBtn[] = [
    { tool: 'mermaid', icon: 'share', label: 'Diagram (Mermaid)', key: 'M' },
    { tool: 'code', icon: 'terminal', label: 'Code block', key: 'C' },
    { tool: 'json', icon: 'file', label: 'JSON block', key: 'J' },
    { tool: 'image', icon: 'square', label: 'Image', key: 'I' },
    { tool: 'frame', icon: 'panel', label: 'Frame / slide', key: 'F' },
    { tool: 'freehand', icon: 'edit', label: 'Freehand (beta)', key: 'P' },
  ];

  const isShapeActive = $derived(activeTool.startsWith('shape:'));
</script>

<div class="rail">
  {#each top as b (b.tool)}
    <button
      class="tool"
      class:active={activeTool === b.tool}
      title={`${b.label} (${b.key})`}
      aria-label={b.label}
      onclick={() => onpick(b.tool)}
    >
      <Icon name={b.icon} />
    </button>
  {/each}

  <!-- Shape: a button that opens a variant menu -->
  <div class="shape-wrap">
    <button
      class="tool"
      class:active={isShapeActive}
      title="Shape (R)"
      aria-label="Shape"
      onclick={() => (shapeMenu = !shapeMenu)}
    >
      <Icon name="square" />
    </button>
    {#if shapeMenu}
      <div class="shape-menu" role="menu">
        {#each SHAPE_VARIANTS as v (v.variant)}
          <button
            role="menuitem"
            class:active={activeTool === `shape:${v.variant}`}
            onclick={() => {
              onpick(`shape:${v.variant}`);
              shapeMenu = false;
            }}
          >
            {v.label}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <button
    class="tool"
    class:active={activeTool === 'connector'}
    title="Connector (X)"
    aria-label="Connector"
    onclick={() => onpick('connector')}
  >
    <Icon name="branch" />
  </button>

  <span class="divider"></span>

  {#each power as b (b.tool)}
    <button
      class="tool"
      class:active={activeTool === b.tool}
      title={`${b.label} (${b.key})`}
      aria-label={b.label}
      onclick={() => onpick(b.tool)}
    >
      <Icon name={b.icon} />
    </button>
  {/each}
</div>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 6px 4px;
    background: var(--surface);
    border-right: 1px solid var(--border);
    width: 42px;
    flex: 0 0 42px;
    z-index: 4;
  }
  .tool {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: none;
    background: none;
    color: var(--text);
    border-radius: var(--radius-s, 6px);
    cursor: pointer;
  }
  .tool:hover {
    background: var(--surface-2);
  }
  .tool.active {
    background: var(--accent);
    color: #fff;
  }
  .divider {
    width: 22px;
    height: 1px;
    background: var(--border);
    margin: 5px 0;
  }
  .shape-wrap {
    position: relative;
  }
  .shape-menu {
    position: absolute;
    left: 38px;
    top: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    padding: 4px;
    z-index: 10;
    min-width: 130px;
  }
  .shape-menu button {
    text-align: left;
    padding: 6px 8px;
    border: none;
    background: none;
    color: var(--text);
    border-radius: var(--radius-s, 6px);
    font-size: 13px;
    cursor: pointer;
    white-space: nowrap;
  }
  .shape-menu button:hover,
  .shape-menu button.active {
    background: var(--surface-2);
  }
</style>
