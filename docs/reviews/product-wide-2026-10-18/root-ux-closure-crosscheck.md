# Current root-owned UX closure cross-check

The OPEN labels for root-owned surfaces in design-grok-closure-crosscheck.md
refer to an earlier source snapshot. This table supersedes those labels only
for the listed root-owned findings. Original reports remain historical evidence;
no global count or 9.8 score is inferred from this table.

| Finding IDs | Current source evidence | Verification boundary |
|---|---|---|
|08-01/14-01,14-02,14-03,08-03/14-04,14-05|ProofPage hides phone empty detail, suppresses partial-page totals, labels done/risk, wraps titles, adds36px hits and hides archive without workspace; ProofStatusChip labels risk.|Phone light/dark browser plus proofOwnership/reviewClosure; iteration-02-proof-review-ux.md.|
|08-02/14-06,08-04/14-07,08-05|RunWithOttoPage phone Back hides rather than destroys launcher; RunLauncher/RunDetail use loadErrorText and36px hits.|Browser Back preserves draft; focused actual-handler tests.|
|08-06/14-09,08-09/08-10,08-12/14-10,08-13,08-14|ReviewPanel normal-flow configuration, wrapped findings, readiness error/Retry with response ownership, readable CI labels; LocalReviewPanel text outside checkbox label; both register palette commands.|Source rechecked; modal/waive browser uses real isolated finding API; related12focused tests recorded.|
|08-08/14-11|FindingActions prompts for audit reason and aborts on Cancel/changed finding; ProofPackView error Retry, disabled Export without loaded pack, readable statuses.|Mounted browser confirms cancel no mutation and supplied reason audit; proof retry recovers.|
|06-01/12-Canvas-01,06-03/12-Canvas-02,12-Canvas-03/18-09,06-04/18-15|Canvas phone editor remains editable, diagram controls36px, assistant wraps with Close first-row, automatic-direction filters and focus styling.|Real isolated phone source save in both schemes; assistant viewport/hits; pointer numerical unit tests.|
|12-DH-01,12-DH-02,12-DH-03|Studio3D exposes Scene controls; SiteCanvas toolbar measured/clamped/wrapped; SiteStudio panel navigation reveals checks instead of vertical inspector trapping.|Named browser Scene controls at1024px, site navigation and toolbar bounds.|
|06-14/12-DH-04,12-DH-05,12-DH-06,06-05–13/15–16/12-DH-07|Publish secondary, variant controls36px, visible spatial captions; Lobby primary hierarchy, touch selects; Artifact phone badges; VersionStrip RTL/scroll/hits; Brand/Learned Back and readable errors.|Source rechecked; Brand Back browser; design closure5/5. Real-device multitouch remains untested.|
|09-01/15-02,09-03/15-03/04/18-11,09-04/15-07|API response reveal, named close/status tabs, matching36px virtual rows; Git pull failure retains dialog/Retry.|Named API browser cases and actual store handlers.|
|09-02/15-05,15-01/06/08|Persistent TLS-off explanation and automation destination/effects summary; one primary; timeout button names actual settings.|Alternative resolution preserving user's minimal-friction requirement; no extra modal approval claim.|
|09-05/15-12/18-10,09-06/15-13/14,09-07/09/15-09/10/11|Browser touch controls, mark deletion/send preview, h2 article, summary provenance, container-responsive URL and reachable lazy credentials modal.|Named browser + handler tests; credentials repeat after lazy bundle fix1/1.|
|09-10,09-11,09-12|Channels Slack directory LoadState Retry and manual-ID fallback, correct webhook secret guidance; sender replacement explanation and stacked phone sharing rows.|Settings3/3 including light/dark390px.|
|10-Insights-01/02,16-Insights remaining|Wall-clock/visibility elapsed time, retained report with failed-reload Retry, phone loading detail hidden and touch controls.|Insights3/3 plus source review; iteration-02-insights-ux.md.|
|10-Usage-01–05,16-Usage-01–03|Measured chart width/spacing, HTML day details, visible costs, transparent hits, consistent stacks, zero-state, primary hierarchy and preserved sparkline aspect.|Two red→green spacing tests; phone light/dark browser2/2; iteration-02-usage-ux.md.|
|16-Usage-04/05|Existing budget wrapping and OttoUsage LoadState/loadErrorText; follow-up gives last_error a fixed explanatory sentence before technical detail.|Current source confirmed; broad telemetry behavior remains separate from UI copy.|

The root named combined browser run recorded14/14 against rebuilt isolated
debug ottod, plus later Settings3/3, Insights3/3, lazy credentials1/1 and Usage2/2.
These are separate actual runs, not a claim all cases were rerun on one frozen
snapshot. Current combined UI/Rust/release gates remain required.

Product real Stop is not included as closed: backend exists and passed its
focused tests; Ronaldo is implementing the corresponding UI. Pele's new
scheduled/assistant/swarm/mission-control work requires his named browser evidence.
