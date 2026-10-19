# Settings integration UX closure

Root took narrow ownership of Channels.svelte, EmailSenderSetup.svelte and
ActiveShareLinks.svelte after coordinating with Ronaldo; no backend permissions
or API contracts changed.

- Slack directory failures now show shared LoadState with Retry in the editor.
  Manual member/channel IDs remain editable. Refresh preserves existing names
  on failure; latestOnly and workspace guards prevent stale completion/error/
  finalizer writes from replacing the current directory. Webhook caller copy
  explicitly states that the secret key is still required.
- The disabled saved Gmail address has a visible, aria-describedby explanation
  pointing to Replace, rather than relying only on a tooltip.
- Active share links become labelled stacked rows on phones, retain native table
  headings and Revoke controls, and allow long titles/labels to wrap. The session
  link has a 36px minimum target.

Named isolated browser suite desktop-settings-closure.spec.ts: **3 passed,
zero failed, 5.2s**. Slack503→visibleRetry→successfulnames, editablemanualID,
light/dark390px shares, disabledsenderexplanation, visibleRevoke and no row/page
overflow are asserted. Only fixture GET routes are mocked; no messages, email,
revokes or real account edits are sent. Log /tmp/otto-root-settings-closure.log.
Full `npm run check` passed: guards, Svelte0errors/0warnings and all TypeScript configurations. Source LOC ratchet also passed (otto-server130470/130939).
