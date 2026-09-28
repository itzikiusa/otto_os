# Connection comparison verification

Run from `ui/` against an isolated test daemon:

```sh
OTTO_E2E_SLOT=comparison OTTO_E2E_PORT=7851 OTTO_E2E_PW_PORT=5251 \
  OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/absolute/path/to/ottod \
  npx playwright test e2e/desktop-db-comparison.spec.ts \
  e2e/desktop-agent-ui-control.spec.ts --project=desktop-browser --workers=1
```

Use unused ports/slot when another run is active. The harness creates temporary
state. Database profiles point to closed loopback port 9; Playwright intercepts
engine calls. Production labels exercise the UI guards without accessing any
production database. The agent suite uses the real daemon, session grant and
stdio MCP bridge, with mocked engine responses.

## Coverage

- Open two production-labeled connections; preserve distinct drafts, submitted
  statements and loaded results while switching and pinning a reference.
- Pin, change reference, focus source and clear it without additional queries.
- Show both results in light/dark; stack within a narrow workbench with no page
  overflow or overlap between the editor and reference.
- Remove reference data and an open cell viewer or context menu on permission
  revocation. Closing the source connection removes the reference too.
- Retain the pin when another connection restores a non-query view.
- Invoke Compare results from Structure through the command registry: switch
  to Query and display the loaded result without rerunning it.
- Complete a deferred query while its source is in the background: update the
  reference while leaving the active connection's draft and result untouched.
- One granted agent runs queries on two connections with `read_only: true`,
  enumerates both tab IDs, then reads the original result without rerunning it.
- Reject conflicting tab/connection IDs before changing focus, SQL or sending
  a query. Existing write cancellation, Stop/revoke and Deny remain covered.

The comparison uses existing result references and the virtualized grid; it
does not issue comparison queries or copy entire datasets. This is a view of
loaded results, not a full cross-database reconciliation test. Native detach
and return checks are documented in [detachable panes](./detachable-panes.md).

## Recorded verification — 2026-09-28

- All eight comparison cases passed, including the command-navigation regression
  which failed before its fix. Light, dark and phone screenshots were inspected.
- The combined suite passed 23 cases before the final command case was added:
  13 detachable-pane/browser split cases, seven comparison cases and three
  real-daemon agent-control cases. The final eight-case comparison rerun brings
  the distinct passing coverage to 24 cases.
- UI units passed 608/608 with serial execution. The default parallel run had
  an unchanged JSON-preview timing benchmark fail; it passed in isolation and
  in the serial suite. No threshold was changed.
