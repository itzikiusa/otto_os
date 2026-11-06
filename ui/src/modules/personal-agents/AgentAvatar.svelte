<script lang="ts">
  // One avatar tile for a personal agent, used by the rail, the conversation
  // header, the panel and room messages. The avatar is user data: a character
  // (`char:<id>` — the in-repo art, characters.ts), a short glyph or initials;
  // with none set it falls back to the name's monogram. The tile is neutral
  // on purpose: accent means *selected*, never agent identity. `state`
  // animates the character subtly (working = a slow pulse) — only where one
  // avatar is shown, never in lists, and never under reduced motion.
  import { characterUrl } from './characters';

  interface Props {
    /** The agent's saved avatar ('' = none). */
    avatar?: string | null;
    /** Name for the monogram fallback. */
    name: string;
    /** Tile edge in px. */
    size?: number;
    round?: boolean;
    state?: 'idle' | 'working' | 'needs_you' | null;
  }
  let { avatar = '', name, size = 32, round = false, state = null }: Props = $props();

  const img = $derived(characterUrl(avatar));
  const glyph = $derived.by(() => {
    const a = (avatar ?? '').trim();
    if (a && !img && !a.startsWith('char:')) return a;
    const m = name.trim().match(/[\p{L}\p{N}]/u);
    return m ? m[0].toUpperCase() : '?';
  });
  const mono = $derived(!img && !((avatar ?? '').trim() && !(avatar ?? '').startsWith('char:')));
</script>

{#if img}
  <span class="pa-avatar char state-{state ?? 'none'}" class:round style:width="{size}px" style:height="{size}px" aria-hidden="true">
    <img src={img} alt="" width={size} height={size} loading="lazy" decoding="async" draggable="false" />
  </span>
{:else}
  <span
    class="pa-avatar"
    class:round
    class:mono
    style:width="{size}px"
    style:height="{size}px"
    style:font-size="{Math.max(11, Math.round(size * (mono ? 0.46 : 0.55)))}px"
    aria-hidden="true">{glyph}</span>
{/if}

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
  .pa-avatar.char img {
    inline-size: 100%;
    block-size: 100%;
    object-fit: contain;
    display: block;
  }
  .pa-avatar.state-needs_you {
    border-color: var(--warning);
  }
  @media (prefers-reduced-motion: no-preference) {
    .pa-avatar.state-working img {
      animation: otto-pulse 2.4s ease-in-out infinite;
    }
  }
</style>
