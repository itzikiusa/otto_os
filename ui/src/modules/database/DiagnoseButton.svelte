<script lang="ts">
  // The one "Diagnose with an agent…" action (ended SSH tab, DB connect
  // error, the form's failed Test). It only opens the consent dialog — nothing
  // runs until the user presses Start there (diagnose.svelte.ts).
  import Icon from '../../lib/components/Icon.svelte';
  import type { Connection } from '../../lib/api/types';
  import { connDiagnose } from './diagnose.svelte';
  import type { DiagnoseProbe } from './diagnose-prompt';

  interface Props {
    conn: Connection;
    probe: DiagnoseProbe | null;
    /** Runs first (the connection form closes itself so the thread is visible). */
    before?: () => void;
  }
  let { conn, probe, before }: Props = $props();
</script>

<button
  class="btn small"
  onclick={() => {
    before?.();
    connDiagnose.request(conn, probe);
  }}
  title="An agent checks, on this Mac, why {conn.name} does not connect — you approve every command"
  data-testid="conn-diagnose"
>
  <Icon name="assistant" size={12} />Diagnose with an agent…
</button>
