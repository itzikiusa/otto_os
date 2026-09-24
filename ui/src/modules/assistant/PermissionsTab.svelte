<script lang="ts">
  // The Permissions tab — a READ-ONLY preview. Editable grants (an agent
  // identity with folders, sites, channels and tools set to allow / ask /
  // deny, plan §2.5) arrive in a later phase and the daemon has no grants
  // route yet, so this page says what is true today instead of showing
  // controls that do nothing.
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import StatePill, { type Tone } from './cards/StatePill.svelte';

  const TODAY: { icon: IconName; resource: string; mode: string; tone: Tone; note: string }[] = [
    { icon: 'send', resource: 'Send, post, publish or submit', mode: 'Ask', tone: 'warn', note: 'An approval card shows where it goes, what is sent and who sees it' },
    { icon: 'hand', resource: 'Always allow a destination', mode: 'Ask', tone: 'warn', note: 'Offered per destination and tool, from the approval card itself' },
    { icon: 'lock', resource: 'Purchases', mode: 'Ask', tone: 'warn', note: 'Never “Always allow”' },
    { icon: 'db', resource: 'Prod connections', mode: 'Ask', tone: 'warn', note: 'Never “Always allow”' },
    { icon: 'key', resource: 'Site logins', mode: 'Keychain', tone: 'neutral', note: 'Site Credentials are used by name, never pasted into prompts' },
    { icon: 'eyeOff', resource: 'Incognito threads', mode: 'No memory', tone: 'neutral', note: 'Nothing is recalled or remembered; deleted 24 h after the last turn' },
  ];
</script>

<div class="perm" data-testid="assistant-permissions">
  <div class="banner" role="note">
    <Icon name="info" size={14} />
    <p>
      <strong>Read-only for now.</strong>
      Choosing what Otto may do on its own — folders, websites, channels, tools — arrives in a later phase. Until then, the rules
      below are what Otto follows.
    </p>
  </div>

  <section>
    <h2 class="h">Otto’s identity</h2>
    <p class="help">Everything Otto does is attributed to it in the thread and on the Tasks tab, with who approved what. A separate agent identity with its own audit trail comes with editable grants.</p>
  </section>

  <section>
    <h2 class="h">What Otto does today</h2>
    <p class="help">Otto starts read-only. Anything that leaves your Mac asks you first.</p>
    <div class="table-wrap">
      <table>
        <thead>
          <tr><th scope="col">Action</th><th scope="col">Access</th><th scope="col" class="note-col">Details</th></tr>
        </thead>
        <tbody>
          {#each TODAY as r (r.resource)}
            <tr>
              <td><span class="res"><Icon name={r.icon} size={14} />{r.resource}</span></td>
              <td><StatePill tone={r.tone} label={r.mode} /></td>
              <td class="note-col dim">{r.note}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </section>
</div>

<style>
  .perm {
    max-width: 820px;
    padding: 18px 20px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .banner {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    background: var(--info-soft);
    border: 1px solid color-mix(in srgb, var(--info) 30%, transparent);
    font-size: var(--fs-s);
  }
  .banner > :global(svg) {
    color: var(--info);
    margin-top: 2px;
  }
  .banner p {
    margin: 0;
  }
  .h {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .help {
    margin: 2px 0 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  th {
    text-align: start;
    font-weight: 600;
    color: var(--text-dim);
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
  }
  td {
    padding: 6px 12px;
    height: 32px;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }
  tr:last-child td {
    border-bottom: 0;
  }
  .res {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .res :global(svg) {
    color: var(--text-dim);
  }
  .dim {
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .perm {
      padding: 12px 12px 24px;
    }
    .note-col {
      display: none;
    }
  }
</style>
