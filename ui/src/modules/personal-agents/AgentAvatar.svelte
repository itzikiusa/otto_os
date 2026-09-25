<script lang="ts">
  // One avatar tile for a personal agent, used by the cards, the agent page
  // header and room messages. The avatar is user data (the sheet's "Avatar"
  // field, usually an emoji); with none set it falls back to a monogram. The
  // tile is neutral on purpose: accent means *selected*, never agent identity.
  interface Props {
    /** The agent's saved avatar ('' = none). */
    avatar?: string | null;
    /** Name for the monogram fallback. */
    name: string;
    /** Tile edge in px. */
    size?: number;
    round?: boolean;
  }
  let { avatar = '', name, size = 32, round = false }: Props = $props();

  const glyph = $derived.by(() => {
    const a = (avatar ?? '').trim();
    if (a) return a;
    const m = name.trim().match(/[\p{L}\p{N}]/u);
    return m ? m[0].toUpperCase() : '?';
  });
  const mono = $derived(!(avatar ?? '').trim());
</script>

<span
  class="pa-avatar"
  class:round
  class:mono
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.max(11, Math.round(size * (mono ? 0.46 : 0.55)))}px"
  aria-hidden="true">{glyph}</span>

<style>
  .pa-avatar {
    flex: 0 0 auto;
    display: inline-grid;
    place-items: center;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
    line-height: 1;
    overflow: hidden;
    box-sizing: border-box;
  }
  .pa-avatar.round {
    border-radius: 50%;
  }
  .pa-avatar.mono {
    font-weight: 600;
    color: var(--text-dim);
  }
</style>
