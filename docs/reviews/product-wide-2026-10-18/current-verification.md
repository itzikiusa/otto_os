# Current verification matrix

This file is the current summary. Original findings, failed runs and provisional
scores remain in the iteration reports and verification-history-before-current.md.
Commands/log hashes and their snapshot boundaries are in verification-evidence.json.
No full-product release or all-lenses9.8 claim is established.

**Latest correction results:** backend committed18a21bfd0. Policy coverage3/3,
ClickHouse parity with explicit ports12/12, affected clippy, workspace doctests,
nextest filter audit and LOC ratchet all passed. LOC including staged new files
is130677/130939. Team Performance492passed/1skipped. Full UI rerun2062/2062,
zero failures/skips. Final UI check/build/bundle passed and named browser closure is recorded;
no push, merge or installation yet. The initial failure details below are retained.

## Final UI gate (2026-10-19)

The frozen UI passed npm run check (zero errors/warnings), all 2062 unit tests
(zero failures/skips), production build, and bundle-budget (zero overages).
No bundle baseline was raised. Scheduled report markdown now shows an inline
load failure with Retry and retains Plain text access; the fault-injection and
normal report browser cases passed 2/2 on an isolated daemon. Logs are recorded
in verification-evidence.json. PR publication, remote CI, merge, and installation
remain pending. Historical failures below are retained, not current blockers.

## Latest combined gate (2026-10-19, r4)

Full workspace formatting and all-target clippy passed. Standard Cargo ran
the workspace tests (nextest was absent from that command's PATH):5990passed,
1failed,125ignored across106 result summaries. The sole failure is the policy
snapshot missing ten entries for the two new Product Stop routes; all write
entries require Product Edit and no prior entry changed. Snapshot regeneration
and its focused rerun, ClickHouse CI fixture ports, doctests and the nextest
filter guard are in progress. Do not report this initial run as entirely green.

The combined UI check passed; units2060passed/2failed due missing names in two
extracted-component harnesses (Pele owns repairs). Production build passed;
unchanged bundle limits initially failed Product/PersonalAgents/Usage. Product
and Usage now measure197.5/203.3KB and32.2/36.1KB after deferred component loads.
PersonalAgents, ScheduledTasks and MissionControl budget closure remains with
Pele. Root's extended Usage report-tab browser cases passed2/2.

Ronaldo's real Product Stop UI is implemented and frozen: exact turn IDs,
pending acknowledgement, truthful stopped:false, preserved accepted messages.
Its ten ownership units and seven mounted browser cases passed; see
iteration-02-product-stop.md. This supersedes the older pending-UI row below.

## Completed integration evidence

- Rust workspace r3: formatting, all-target clippy, **5978 nextest passed /124
  skipped**, and doctests passed. The final filter guard initially failed because
  system Python lacked tomllib; its Python3.13 rerun **passed** and compiled/listed
  current test binaries. The later reservation-refund source was not executed by
  that workspace test run.
- UI: **2056 unit tests passed, zero failed/skipped**, full npm check and production
  build passed. The Plugin bundle initially exceeded tolerance; Ronaldo's later
  reduction passed the unchanged budget (8.4KB vs8.3KB, inside existing tolerance).
  Root Settings/Insights follow-ups passed their subsequent npm check/guards.
- Root named browser closure: Proof4, Design/Canvas5, API/browser3, Insights2
  together **14/14 passed** against rebuilt isolated debug ottod. Later Insights
  phone-loading/filter suite **3/3**, Settings Slack/sharing suite **3/3** including
  light/dark390px, and lazy browser credentials **1/1** passed.
- Active held-shell restart smoke: same shell PID survives daemon SIGTERM/restart,
  input works after reattachment, and explicit fixture cleanup completes. This
  is not a global detached-job drain or installed-app deployment test.
- LOC ratchet passed: otto-server130470/130939. No budget baseline increase.

## Engineering closure and remaining work

| Area | Evidence already available | Still required |
|---|---|---|
| E01 sessions | Capture/grid/kill-retirement regression runs; sessions/server1586 and shared consumer clippy; independent report | Final changed-tree consumer gate after reservation API changes |
| E02 workflows | Workflow62 and shared actor/env/auth, replay, trigger cancellation cases | Final integration snapshot; workflow UI transferred to Ronaldo |
| E03 Git |408 lib/1ignored, native stash identity/recovery regressions, clippy and independent review | Final integration snapshot |
| E04 schedules | Automation78; supervisor admission checks | Pele's remaining schedule UI and final integration |
| E05 loops | Automation78; stopped-row/runtime admission tests | Goal-loop UI transferred to Ronaldo |
| E06 assistant | Assistant74; proactive first fixture corrected1/1 | Four focused refund regressions and assistant check passed; assistant UI remains |
| E07 swarm | Swarm65 after initial reservation changes | Refund/retention negative controls passed; swarm UI and final integration remain |
| E08 API | API engine25, server API46/grpc3, compiler-backed banner, current UI/browser cases | Final integration snapshot |
| E09 connections | SSH37, connections82+integration, brokers69 and clippy | Final integration snapshot |
| E10 Vault | Vault91+32engine, memory8+7integration and clippy | Ronaldo's current UI browser closure |
| E11 Design/Canvas | Design145/1ignored, assist13, Canvas30, stateCanvas12/1ignored; scene ownership and browser cases | Final integration snapshot; documented old-import/publish boundaries retained |
| E12 Product | Product157/1ignored, issues125, startup lifecycle regressions | Real conversational Stop backend/UI follow-up and browser closure; immutable old-file retention remains explicit |
| E13 proof/review | Coreproof25, statereviews16, server publication/retry/provisioning and browser4 | Final integration snapshot |
| E14 state | Atomic/settings/pre13/db regressions and clippy | New reservation/retention changes and shared consumers |
| E15 runtime | Supervisor cancellation regressions, server/daemon clippy, active held-shell restart | Preserve bounded-shutdown boundary; score remains9.7, not global-drain certification |
| E16 credentials | Real Git challenge fixtures, Git/server/review gates and validated publication caller | Final integration snapshot |
| E17 channels | Channels139, typed provider transcript/retry/order fixtures, consumer checks | Final integration snapshot; memory-only queue boundary retained |
| E18 accounting | Strict budget unit, rebuilt live ClickHouse replay zeroSKIP, usage/transcript and server budget gates | Final integration snapshot; legacy non-durable record buffer boundary retained |
| E19 extensions | MCP110, skills20, context64, browser133, pluginserver6 and clippy | Final integration snapshot |
| E20 UI contracts | State settings7, server atomic1, Activity ownership units and mounted browser3 | Final integration snapshot |

Counts above belong to their actual runs, not a freshly rerun single snapshot.
Do not repeat unchanged package suites merely because Cargo becomes idle.

## Active independent findings

The three reservation-refund findings in iteration-02-automation-independent.md
are now repaired and exercised by four focused state tests: exact stopped-marker
refund across retention, NULL/other-error negative controls, proactive previous
stamp restoration, and preservation of a newer stamp with daily-cap accounting.
Pele reports all four passed plus assistant lib check. Root independently ran the
`release` filter (three tests passed, including two refund cases) in
/tmp/otto-root-reservation-release-tests.log. Root's affected state/assistant/swarm all-target clippy passed after replacing
a test-only get/is_none assertion with contains_key. Scoped rustfmt and
workspace diff whitespace checks passed; final consumer integration remains pending.

## UI ownership

Only existing original sessions are used; no new reviewers/sessions.

- Ronaldo: current Product/Vault/Help/Skills/Plugin browser verification, then
  workflows and goal-loop UI remaining findings. Prior Canvas store/panel and
  compiler-backed API banner repairs are complete.
- Pele: scheduled tasks, assistant/personal agents, swarm/mission control UI,
  alongside reservation backend repairs.
- Root: Proof/review/Run-with-Otto, Design/Canvas components, API/browser,
  Insights and narrow Settings closures; final integration/release.

Product Stop waiting remains distinct from cancellation. A new exact-turn Stop
backend is implemented with six registry tests, real-PTY cleanup and permission
regressions, server type-check and all-target clippy passing. Ronaldo's UI
integration and mounted browser test are pending. See
iteration-02-product-stop.md. Refinement still produces a non-destructive
suggested draft. No closure score is raised before the new flow is verified.

## Database and release boundaries

The DB ledger has24 lenses through ten passes. Mongo/MySQL/Redis improvements
and explicit Mongo shell fallback are exercised; ClickHouse native
JSON/Dynamic/Variant and several transport-matrix lenses remain below9.8.
See ../db-parity-2026-10-18/final-verification.md. No universal parity claim.

PR111 remains open at published head c009da20. Published-head non-advisory
checks passed, including live DB/Kafka; advisory shards/ratchet failed and desktop
packaging was skipped. Expanded work is still uncommitted, so those checks do
not validate it. Required next: finish open fixes, final affected checks,
update the same PR, pass expanded-head required checks, merge AND push main,
then rebuild/reinstall and verify deployment receipt/health/revision.
No merge or deployment has occurred. completion-audit.md tracks the full request.

## Latest Usage UI follow-up

Root chart/phone/empty-state follow-up passed two spacing regressions and two
light/dark390px named browser cases; Svelte0errors/0warnings and E2E tsc passed.
See iteration-02-usage-ux.md for exact evidence and native-select automation
boundary. Full combined UI guard gate awaits concurrent Swarm fixes.

## ClickHouse strict-read-only startup follow-up

The prior readonly1 native startup limitation is repaired in both dependency and
driver setup. New live regression red-to-green; full live parity12/12, focused
units40/40(oneoptionalignored), dbviewer all-target clippy passed. No weakening
of server policy, no user-query replay, and no claim of native JSON support.
The dependency patch requires the fresh workspace integration gate now running.

Root-owned historical OPEN UX rows are reconciled in root-ux-closure-crosscheck.md
with source/test boundaries. The earlier aggregate165-open tally is historical,
not a current release count. Pele’s named browser run for scheduled tasks,
personal agents, rooms, assistant, swarm, and mission control is 64/64 on slot 18
(daemon 7816, UI 5214). Finding ids and which of them that run actually asserted
are in iteration-01-automation-grok.md. Owned UI is frozen. Workflows and loops
were not repeated.
