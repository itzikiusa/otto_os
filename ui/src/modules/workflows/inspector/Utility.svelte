<script lang="ts">
  // The small utility kinds: delay, transform, game_engine, channel_notify,
  // budget_gate, human_approval and condition.
  import type { WorkflowNode } from '../../../lib/api/types';
  import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
  import JsonLabel from './JsonLabel.svelte';
  import { jsonValue, paramJson, paramNum, paramStr, type SetParam } from './util';

  let { node, set, onzoom }: { node: WorkflowNode; set: SetParam; onzoom: (field: string, label: string) => void } = $props();
</script>

{#if node.kind === 'delay'}
  <label for="np-ms">Wait (ms)</label>
  <input
    id="np-ms"
    type="number"
    min="0"
    max="10000"
    step="100"
    value={paramNum(node, 'ms', 0)}
    oninput={(e) => set('ms', Number(e.currentTarget.value))}
  />
{:else if node.kind === 'transform'}
  <JsonLabel forId="np-json" label="Merge JSON (object)" onzoom={() => onzoom('json', 'Merge JSON (object)')} />
  <textarea dir="ltr"
    id="np-json"
    rows="4"
    placeholder="&#123;&#125;"
    value={paramJson(node, 'json')}
    oninput={(e) => set('json', jsonValue(e.currentTarget.value))}
  ></textarea>
{:else if node.kind === 'game_engine'}
  <label for="np-game">Game type</label>
  <select
    id="np-game"
    value={paramStr(node, 'game') || 'slots'}
    onchange={(e) => set('game', e.currentTarget.value)}
  >
    <option value="slots">Slots (5×3)</option>
    <option value="crash">Crash (Aviator-style)</option>
    <option value="scratch">Scratch card</option>
  </select>
{:else if node.kind === 'channel_notify'}
  <label for="np-msg">Message</label>
  <textarea dir="auto"
    id="np-msg"
    rows="3"
    placeholder="Workflow step completed: &#123;reply&#125;"
    value={paramStr(node, 'message')}
    oninput={(e) => set('message', e.currentTarget.value)}
  ></textarea>
  <label for="np-ch">Channel (optional)</label>
  <select
    id="np-ch"
    value={paramStr(node, 'channel') || ''}
    onchange={(e) => set('channel', e.currentTarget.value || undefined)}
  >
    <option value="">Any enabled</option>
    <option value="slack">Slack</option>
    <option value="telegram">Telegram</option>
  </select>
{:else if node.kind === 'budget_gate'}
  <label for="np-provider">Provider (whose usage budget to check)</label>
  <select
    id="np-provider"
    value={paramStr(node, 'provider') || defaultAgentProvider()}
    onchange={(e) => set('provider', e.currentTarget.value)}
  >
    {#each agentProviders() as pv (pv)}<option value={pv}>{pv}</option>{/each}
  </select>
  <p class="node-hint">Errors the run if the provider budget is exceeded and enforcement is on.</p>
{:else if node.kind === 'human_approval'}
  <label for="np-aprompt">Approval prompt</label>
  <input dir="auto"
    id="np-aprompt"
    type="text"
    placeholder="Please review and approve to continue"
    value={paramStr(node, 'prompt')}
    oninput={(e) => set('prompt', e.currentTarget.value)}
  />
  <p class="node-hint">Pauses the run until an operator calls the resume endpoint or clicks Approve above.</p>
{:else if node.kind === 'condition'}
  <label for="np-expr">Expression</label>
  <input dir="ltr"
    id="np-expr"
    type="text"
    placeholder="e.g. score >= 80"
    value={paramStr(node, 'expr')}
    oninput={(e) => set('expr', e.currentTarget.value)}
  />
  <p class="node-hint">Truthy → downstream nodes run; falsy → they’re skipped.</p>
{/if}
