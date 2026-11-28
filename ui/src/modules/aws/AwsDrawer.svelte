<script lang="ts">
  // The Detail pane of the AWS service views (EC2, RDS): the shared DetailPane
  // (inline column beside the table while it keeps 480 px, else a right sheet;
  // resizable, Esc, focus return) with the detail tabs AS its header, and the
  // kind / name / id / state on the first line of the body.
  import type { BadgeTone } from '../../lib/status';
  import { sentenceCase } from '../../lib/labels';
  import Badge from '../../lib/components/Badge.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import DetailPane from '../../lib/components/side/DetailPane.svelte';
  import type { Snippet } from 'svelte';

  interface Props {
    /** Small uppercase kind label ("instance", "db instance"). */
    kind: string;
    name: string;
    /** Secondary id shown after the name (instance id, endpoint…). */
    id?: string;
    /** Status text (the raw AWS state; shown sentence-cased) + its Badge tone. */
    status?: string;
    statusTone?: BadgeTone;
    tabs: { id: string; label: string }[];
    tab: string;
    ontab: (id: string) => void;
    onclose: () => void;
    children: Snippet;
  }
  let { kind, name, id = '', status = '', statusTone = 'neutral', tabs, tab, ontab, onclose, children }: Props =
    $props();
</script>

<DetailPane open module="aws" pane={kind.replace(/\s+/g, '-')} title="{kind} details" {onclose} testid="aws-drawer">
  {#snippet views()}
    <Tabs size="s" label="Detail tabs" idBase="aws-dr" {tabs} value={tab} onchange={ontab} />
  {/snippet}
  <div class="dr-title">
    <span class="dr-kind">{kind}</span>
    <span class="dr-name" title={name}>{name}</span>
    {#if id && id !== name}<span class="dr-id mono" title={id}>{id}</span>{/if}
    {#if status}<Badge tone={statusTone} label={sentenceCase(status)} testid="aws-drawer-status" />{/if}
  </div>
  <div class="dr-body" id="aws-dr-panel-{tab}" role="tabpanel" aria-labelledby="aws-dr-tab-{tab}">
    {@render children()}
  </div>
</DetailPane>

<style>
  .dr-title {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 10px 16px 0;
    font-size: var(--fs-m);
  }
  .dr-kind {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .dr-name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .dr-id {
    color: var(--text-dim);
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .dr-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
