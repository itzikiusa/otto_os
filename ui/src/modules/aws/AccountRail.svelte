<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // Left rail inside the AWS module: accounts → services. A service is greyed
  // (still navigable, so the AccessDenied is visible) when the account's IAM
  // probe said `denied`, and hidden outright when the user lacks the feature's
  // View grant. Right-click / ⋯ on an account → edit / refresh perms / delete.
  import { aws, AWS_SERVICES } from '../../lib/stores/aws.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import { pollWhileVisible } from '../../lib/poll';
  import { logsRoute } from './util';
  import type { AwsAccount, AwsService, Feature } from '../../lib/api/types';

  interface Props {
    activeId: string | null;
    activeService: AwsService | null;
    onedit: (a: AwsAccount) => void;
    ondelete: (a: AwsAccount) => void;
    /** Opens the `aws sso login` sheet — offered ahead of a sign-in expiry. */
    onsignin?: (a: AwsAccount) => void;
    /** The CloudWatch Logs view is open for `activeId`. */
    logsActive?: boolean;
  }
  let { activeId, activeService, onedit, ondelete, onsignin, logsActive = false }: Props = $props();

  // A-5: warn BEFORE the sign-in lapses instead of failing mid-action. The
  // expiry is absolute, so a 30 s client clock is enough (no polling).
  const WARN_MS = 15 * 60_000;
  let now = $state(Date.now());
  $effect(() => {
    const p = pollWhileVisible(() => { now = Date.now(); }, { ms: 30_000, immediate: false });
    return () => p.stop();
  });
  function sessionNote(a: AwsAccount): { text: string; expired: boolean } | null {
    const s = a.session;
    if (!s) return null;
    const left = Date.parse(s.expires_at) - now;
    if (Number.isNaN(left)) return null;
    // Refreshable sessions renew silently; only a lapsed one needs the user.
    if (left <= 0 && !s.refreshable) return { text: 'Sign-in expired', expired: true };
    if (!s.refreshable && left < WARN_MS) {
      const min = Math.max(1, Math.round(left / 60_000));
      return { text: `Sign-in ends in ${min} min`, expired: false };
    }
    return null;
  }

  let collapsed: Record<string, boolean> = $state({});

  function featureOf(svc: AwsService): Feature {
    return `aws_${svc}` as Feature;
  }

  $effect(() => { for (const a of aws.accounts) void resourceAccess.load('aws_account', a.id); });
  function services(a: AwsAccount) {
    return AWS_SERVICES.filter((s) => resourceAccess.can('aws_account', a.id, s.id === 's3' ? 'discover' : `${s.id}_view`, featureOf(s.id), 'view')).map((s) => ({
      ...s,
      denied: !aws.serviceAllowed(a.id, s.id),
    }));
  }

  function menu(e: MouseEvent | KeyboardEvent, a: AwsAccount): void {
    ctxMenu.show(e, [
      { label: 'Overview', icon: 'grid', action: () => router.go('aws') },
      {
        label: 'Re-check permissions',
        disabled: !resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'view'),
        icon: 'refresh',
        action: () => void aws.loadPermissions(a.id, true),
      },
      ...(resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'admin')
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
  </div>
  {#each aws.accounts as a (a.id)}
    {@const open = !collapsed[a.id]}
    {@const note = sessionNote(a)}
    <div class="acct" class:active={a.id === activeId}>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="acct-row" oncontextmenu={(e) => menu(e, a)}>
        <button
          class="acct-toggle"
          aria-expanded={open}
          title={a.identity?.arn ? `${a.name}\n${a.identity.arn}` : a.profile ? `${a.name} · profile ${a.profile}` : a.name}
          onclick={() => (collapsed = { ...collapsed, [a.id]: open })}
        >
          <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
          <span class="dot" style="background:{a.color || 'var(--text-dim)'}"></span>
          <span class="name">{a.name}</span>
        </button>
        <EnvBadge env={a.environment} />
        <button
          class="icon-btn more"
          onclick={(e) => menu(e, a)}
          aria-label={`Actions for ${a.name}`}
          title="Actions"
        ><Icon name="more" size={14} /></button>
      </div>
      {#if note}
        <div class="sess" class:expired={note.expired} role="status">
          <Icon name="warning" size={12} />
          <span>{note.text}</span>
          {#if onsignin && a.auth_mode === 'profile'}
            <button class="sess-btn" onclick={() => onsignin?.(a)} title={`Run aws sso login for ${a.name}`}>Sign in again</button>
          {/if}
        </div>
      {/if}
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
          {#if resourceAccess.can('aws_account', a.id, 'metrics', 'aws', 'view')}
            {@const on = a.id === activeId && logsActive}
            <li>
              <a href={`#/${logsRoute(a.id)}`} class:active={on} title="CloudWatch Logs" aria-current={on ? 'page' : undefined}>
                <Icon name="text" size={13} />
                <span>CloudWatch Logs</span>
              </a>
            </li>
          {/if}
        </ul>
      {/if}
    </div>
  {:else}
    <p class="empty">No accounts yet.</p>
  {/each}
</nav>

<style>
  .sess {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 2px 10px 4px;
    padding: 4px 8px;
    border-radius: 6px;
    background: var(--warning-soft);
    font-size: var(--fs-xs);
  }
  .sess.expired {
    background: var(--danger-soft);
  }
  .sess :global(svg) {
    color: var(--warning);
    flex-shrink: 0;
  }
  .sess.expired :global(svg) {
    color: var(--danger);
  }
  .sess-btn {
    margin-inline-start: auto;
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .rail {
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    padding: 6px 0;
    font-size: var(--fs-m);
  }
  .rail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 10px 6px;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .acct-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 3px;
    padding-inline: 8px 6px;
    color: var(--text);
    border-inline-start: 2px solid transparent;
  }
  .acct.active .acct-row {
    border-inline-start-color: var(--accent);
  }
  .acct-toggle {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: start;
    cursor: pointer;
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
    flex-shrink: 0;
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
    padding-block: 4px;
    padding-inline: 32px 10px;
    color: var(--text);
    text-decoration: none;
    border-inline-start: 2px solid transparent;
  }
  .svcs a:hover {
    background: var(--surface-2);
  }
  .svcs a.active {
    background: var(--accent-soft);
    border-inline-start-color: var(--accent);
  }
  .svcs a.denied {
    color: var(--text-dim);
    opacity: 0.7;
  }
  .deny {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .empty {
    margin: 8px 12px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
