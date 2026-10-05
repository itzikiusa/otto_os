<script lang="ts" module>
  // The Assistant's state pill — a thin wrapper over the shared Badge. Tone
  // comes from the domain helpers in ../model.ts, never picked at the call site.
  import type { BadgeTone } from '../../../lib/status';
  export type Tone = Exclude<BadgeTone, 'accent'>;
</script>

<script lang="ts">
  import Badge from '../../../lib/components/Badge.svelte';

  interface Props {
    tone: Tone;
    label: string;
    /** Adds a pulsing dot — only for live state (running now). */
    live?: boolean;
    title?: string;
  }
  let { tone, label, live = false, title }: Props = $props();
</script>

<Badge {tone} {label} {live} {title} />
