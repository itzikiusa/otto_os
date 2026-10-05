<script lang="ts">
  // Left vertical tool rail. Picking an insert tool arms the editor (the next
  // pane click drops that node). Order leads with the high-frequency tools
  // (Select, Sticky, Text, Shape, Connector), then a divider, then the power
  // blocks (Mermaid, Code, JSON, Image, Frame, Freehand). Each button shows its
  // single-key shortcut in the tooltip.
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { SHAPE_VARIANTS, type Tool } from './tools';

  interface Props {
    activeTool: Tool;
    onpick: (t: Tool) => void;
  }
  let { activeTool, onpick }: Props = $props();


  interface ToolBtn {
    tool: Tool;
    icon: IconName;
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

  // The shape variants open in the shared ctxMenu: it clamps into the viewport,
  // closes on Esc / outside click, and returns focus to the Shape button.
  function openShapeMenu(e: MouseEvent): void {
    ctxMenu.showAt(
      e.currentTarget as HTMLElement,
      SHAPE_VARIANTS.map((v) => ({
        label: v.label,
        checked: activeTool === `shape:${v.variant}`,
        action: () => onpick(`shape:${v.variant}`),
      })),
    );
  }
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
  <button
    class="tool"
    class:active={isShapeActive}
    title="Shape (R)"
    aria-label="Shape"
    aria-haspopup="menu"
    onclick={openShapeMenu}
  >
    <Icon name="square" />
  </button>

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
    border-inline-end: 1px solid var(--border);
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
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .tool:hover {
    background: var(--hover);
  }
  .tool.active {
    background: var(--accent-solid);
    color: var(--accent-contrast);
  }
  .divider {
    width: 22px;
    height: 1px;
    background: var(--border);
    margin: 4px 0;
  }
</style>
