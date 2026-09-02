<script lang="ts">
  // Left rail inside the AWS module: accounts → services. A service is greyed
  // (still navigable, so the AccessDenied is visible) when the account's IAM
  // probe said `denied`, and hidden outright when the user lacks the feature's
  // View grant. Right-click / ⋯ on an account → edit / refresh perms / delete.
  import { aws, AWS_SERVICES } from '../../lib/stores/aws.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EnvPill from './EnvPill.svelte';
  import type { AwsAccount, AwsService, Feature } from '../../lib/api/types';

  interface Props {
    activeId: string | null;
    activeService: AwsService | null;
    onedit: (a: AwsAccount) => void;
    ondelete: (a: AwsAccount) => void;
    onadd: () => void;
  }
  let { activeId, activeService, onedit, ondelete, onadd }: Props = $props();

  let collapsed: Record<string, boolean> = $state({});
  const canAdmin = $derived(auth.can('aws', 'admin'));

  function featureOf(svc: AwsService): Feature {
    return `aws_${svc}` as Feature;
  }

  function services(a: AwsAccount) {
    return AWS_SERVICES.filter((s) => auth.can(featureOf(s.id), 'view')).map((s) => ({
      ...s,
      denied: !aws.serviceAllowed(a.id, s.id),
    }));
  }

  function menu(e: MouseEvent | KeyboardEvent, a: AwsAccount): void {
    ctxMenu.show(e, [
      { label: 'Overview', icon: 'grid', action: () => router.go('aws') },
      {
        label: 'Re-check permissions',
        icon: 'refresh',
        action: () => void aws.loadPermissions(a.id, true),
      },
      ...(canAdmin
        ? [
            { label: 'Edit account…', icon: 'edit', action: () => onedit(a) },
            { separator: true },
            { label: 'Delete account', icon: 'trash', danger: true, action: () => ondelete(a) },
          ]
        : []),
    ]);
  }
</script>

<nav class="rail" aria-label="AWS accounts">
  <div class="rail-head">
    <span>Accounts</span>
    {#if canAdmin}
      <button class="mini" onclick={onadd} title="Add account" aria-label="Add account">
        <Icon name="plus" size={13} />
      </button>
    {/if}
  </div>
  {#each aws.accounts as a (a.id)}
    {@const open = !collapsed[a.id]}
    <div class="acct" class:active={a.id === activeId}>
      <div
        class="acct-row"
        role="button"
        tabindex="0"
        aria-expanded={open}
        onclick={() => (collapsed = { ...collapsed, [a.id]: open })}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            collapsed = { ...collapsed, [a.id]: open };
          }
        }}
        oncontextmenu={(e) => menu(e, a)}
      >
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
        <span class="dot" style="background:{a.color || 'var(--text-dim)'}"></span>
        <span class="name" title={a.identity?.arn ?? a.profile ?? ''}>{a.name}</span>
        <EnvPill env={a.environment} />
        <button
          class="more"
          onclick={(e) => {
            e.stopPropagation();
            menu(e, a);
          }}
          aria-label={`Actions for ${a.name}`}
          title="Actions"
        >⋯</button>
      </div>
      {#if open}
        <ul class="svcs">
          {#each services(a) as s (s.id)}
            <li>
              <a
                href={`#/aws/${a.id}/${s.id}`}
                class:active={a.id === activeId && s.id === activeService}
                class:denied={s.denied}
                title={s.denied ? `${s.label}: access denied for this account` : s.label}
                aria-current={a.id === activeId && s.id === activeService ? 'page' : undefined}
              >
                <Icon name={s.icon} size={13} />
                <span>{s.label}</span>
                {#if s.denied}<span class="deny">denied</span>{/if}
              </a>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else}
    <p class="empty">No accounts yet.</p>
  {/each}
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    padding: 6px 0;
    font-size: 12.5px;
  }
  .rail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 10px 6px;
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .mini {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .acct-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px 5px 10px;
    cursor: pointer;
    color: var(--text);
    border-left: 2px solid transparent;
  }
  .acct.active .acct-row {
    border-left-color: var(--accent);
  }
  .acct-row:hover {
    background: var(--surface-2);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .more {
    border: 0;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0 4px;
    font-size: 14px;
    line-height: 1;
    opacity: 0;
  }
  .acct-row:hover .more,
  .acct-row:focus-within .more {
    opacity: 1;
  }
  .svcs {
    list-style: none;
    margin: 0;
    padding: 0 0 4px;
  }
  .svcs a {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px 4px 32px;
    color: var(--text);
    text-decoration: none;
    border-left: 2px solid transparent;
  }
  .svcs a:hover {
    background: var(--surface-2);
  }
  .svcs a.active {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-left-color: var(--accent);
  }
  .svcs a.denied {
    color: var(--text-dim);
    opacity: 0.7;
  }
  .deny {
    margin-left: auto;
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .empty {
    margin: 8px 12px;
    color: var(--text-dim);
    font-size: 12px;
  }
</style>
