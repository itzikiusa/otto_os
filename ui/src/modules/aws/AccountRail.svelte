<script lang="ts">
  import { announce } from '../../lib/announce.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // The AWS list pane's body (inside the page's ListPane): accounts → services,
  // a tree on the shared rows. A service is greyed
  // (still navigable, so the AccessDenied is visible) when the account's IAM
  // probe said `denied`, and hidden outright when the user lacks the feature's
  // View grant. Right-click / ⋯ on an account → edit / refresh perms / delete.
  import { aws, AWS_SERVICES } from '../../lib/stores/aws.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import SideRow from '../../lib/components/side/SideRow.svelte';
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
    /** The accounts overview (all accounts as cards). */
    onoverview: () => void;
    /** A service / Logs was chosen (a phone pushes its content). */
    onopen: () => void;
  }
  let { activeId, activeService, onedit, ondelete, onsignin, logsActive = false, onoverview, onopen }: Props = $props();

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

  function go(path: string): void {
    router.go(path);
    onopen();
  }

  function menu(e: MouseEvent | KeyboardEvent, a: AwsAccount): void {
    ctxMenu.show(e, accountItems(a));
  }
  function accountItems(a: AwsAccount): MenuItem[] {
    return [
      { label: 'Overview', icon: 'grid', action: onoverview },
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
            { label: 'Delete account…', icon: 'trash', danger: true, action: () => ondelete(a) },
          ]
        : []),
    ];
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  // Keyed by account and expiry state (not the ticking minutes), so the
  // countdown doesn't re-announce every minute.
  const sessionNotes = $derived(aws.accounts.map((a) => { const n = sessionNote(a); return n ? `${a.name}: ${n.expired ? 'sign-in expired' : 'sign-in ends soon'}` : ''; }).filter(Boolean).join('. '));
  $effect(() => { if (sessionNotes) announce(`${sessionNotes}.`, { assertive: sessionNotes.includes('expired') }); });
</script>

<div class="rail" role="tree" aria-label="AWS accounts">
  {#each aws.accounts as a (a.id)}
    {@const open = !collapsed[a.id]}
    {@const note = sessionNote(a)}
    <div class="side-row" data-testid="aws-account-{a.id}">
      <button
        use:rowMenu
        class="side-row-main"
        data-side-row
        data-side-toggle
        role="treeitem"
        aria-level={1}
        aria-selected="false"
        aria-expanded={open}
        title={a.identity?.arn ? `${a.name}\n${a.identity.arn}` : a.profile ? `${a.name} · profile ${a.profile}` : a.name}
        onclick={() => (collapsed = { ...collapsed, [a.id]: open })}
        oncontextmenu={(e) => menu(e, a)}
      >
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
        <span class="dot" style="background:{a.color || 'var(--text-dim)'}"></span>
        <span class="side-row-title acct-name">{a.name}</span>
        <span class="side-row-trailing"><EnvBadge env={a.environment} /></span>
      </button>
      <button class="icon-btn side-row-more" tabindex="-1" onclick={(e) => menu(e, a)} aria-label={`Actions for ${a.name}`} title="Actions">
        <Icon name="more" size={14} />
      </button>
    </div>
    {#if note}
      <div class="sess" class:expired={note.expired}>
        <Icon name="warning" size={12} />
        <span>{note.text}</span>
        {#if onsignin && a.auth_mode === 'profile'}
          <button class="sess-btn" onclick={() => onsignin?.(a)} title={`Run aws sso login for ${a.name}`}>Sign in again</button>
        {/if}
      </div>
    {/if}
    {#if open}
      <div role="group" aria-label={a.name}>
        {#each services(a) as s (s.id)}
          <SideRow
            tree
            depth={1}
            icon={s.icon}
            title={s.label}
            selected={a.id === activeId && s.id === activeService}
            tip={s.denied ? `${s.label}: access denied for this account` : s.label}
            testid="aws-svc-{a.id}-{s.id}"
            onclick={() => go(`aws/${a.id}/${s.id}`)}
          >
            {#snippet trailing()}{#if s.denied}<span class="deny">denied</span>{/if}{/snippet}
          </SideRow>
        {/each}
        {#if resourceAccess.can('aws_account', a.id, 'metrics', 'aws', 'view')}
          <SideRow tree depth={1} icon="text" title="CloudWatch Logs" selected={a.id === activeId && logsActive} testid="aws-svc-{a.id}-logs" onclick={() => go(logsRoute(a.id))} />
        {/if}
      </div>
    {/if}
  {/each}
</div>

<style>
  .sess {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-block: 2px 4px;
    margin-inline: 8px 0;
    padding: 4px 8px;
    border-radius: var(--radius-s);
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
  .dot {
    inline-size: 8px;
    block-size: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .acct-name {
    font-weight: 500;
  }
  /* A denied service stays navigable (so the AccessDenied is visible), greyed with the word. */
  .deny {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
</style>
