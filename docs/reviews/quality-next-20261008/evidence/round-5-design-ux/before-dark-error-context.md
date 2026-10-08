# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: desktop-quality-round5-evidence-ux.spec.ts >> pending score never presents old Approval as current evidence dark
- Location: e2e/desktop-quality-round5-evidence-ux.spec.ts:8:3

# Error details

```
Error: expect(locator).toHaveCount(expected) failed

Locator:  locator('.scorecard .artifact').filter({ hasText: 'Human rating' })
Expected: 0
Received: 1
Timeout:  10000ms

Call log:
  - Expect "toHaveCount" locator('.scorecard .artifact').filter({ hasText: 'Human rating' }) with timeout 10000ms
  - waiting for locator('.scorecard .artifact').filter({ hasText: 'Human rating' })
    24 × locator resolved to 1 element
       - unexpected value "1"

```

# Page snapshot

```yaml
- generic [ref=e2]:
  - generic [ref=e3]:
    - generic [ref=e4]:
      - navigation "Modules" [ref=e6]:
        - button "Expand sidebar" [ref=e7] [cursor=pointer]
        - button "Go back" [ref=e10] [cursor=pointer]
        - button "Go forward" [disabled] [ref=e13]
        - button "Notifications, 2 unread" [ref=e17] [cursor=pointer]:
          - generic [aria-hidden] [ref=e20]: "2"
        - generic [ref=e21]:
          - button "Home" [ref=e22] [cursor=pointer]
          - button "Assistant" [ref=e25] [cursor=pointer]
          - button "Agents" [ref=e28] [cursor=pointer]
          - button "Rooms" [ref=e31] [cursor=pointer]
          - button "History" [ref=e34] [cursor=pointer]
          - button "Run with Otto" [ref=e37] [cursor=pointer]
          - button "Mission Control" [ref=e40] [cursor=pointer]
          - separator "Automate" [ref=e43]
          - button "Swarm" [ref=e44] [cursor=pointer]
          - button "Goal Loops" [ref=e47] [cursor=pointer]
          - button "Workflows" [ref=e50] [cursor=pointer]
          - button "Scheduled Tasks" [ref=e53] [cursor=pointer]
          - button "Personal Agents" [ref=e56] [cursor=pointer]
          - separator "Build" [ref=e59]
          - button "Git" [ref=e60] [cursor=pointer]
          - button "Proof" [ref=e63] [cursor=pointer]
          - button "Product" [ref=e66] [cursor=pointer]
          - button "Vault" [ref=e69] [cursor=pointer]
          - button "Design Hall" [ref=e72] [cursor=pointer]
          - button "Workbench" [ref=e75] [cursor=pointer]
          - button "Skills Lab" [ref=e78] [cursor=pointer]
          - separator "Infrastructure" [ref=e81]
          - button "Connections" [ref=e82] [cursor=pointer]
          - button "AWS" [ref=e85] [cursor=pointer]
          - button "Kubernetes" [ref=e88] [cursor=pointer]
          - button "API" [ref=e91] [cursor=pointer]
          - button "Browser" [ref=e94] [cursor=pointer]
          - button "MCP Control Plane" [ref=e97] [cursor=pointer]
          - separator "Insight" [ref=e100]
          - button "Insights" [ref=e101] [cursor=pointer]
          - button "Usage" [ref=e104] [cursor=pointer]
        - generic [ref=e107]:
          - button "Help" [ref=e108] [cursor=pointer]
          - button "Settings" [ref=e111] [cursor=pointer]
          - button "Account" [ref=e114] [cursor=pointer]:
            - generic [ref=e115]: E
      - generic [ref=e119]:
        - generic [ref=e121]:
          - banner [ref=e122]:
            - generic [ref=e123]:
              - button "Hide evaluations list" [expanded] [ref=e125] [cursor=pointer]
              - heading "Skills Lab" [level=1] [ref=e131]
              - button "Evaluator defaults" [ref=e134] [cursor=pointer]
            - generic [ref=e138]:
              - tablist "Skills Lab section" [ref=e139]:
                - tab "Skills" [ref=e140] [cursor=pointer]
                - tab "Review" [ref=e141] [cursor=pointer]
                - tab "Evaluator" [selected] [ref=e142] [cursor=pointer]
              - tablist "Evaluator view" [ref=e146]:
                - tab "Runs" [selected] [ref=e147] [cursor=pointer]
                - tab "Golden tasks" [ref=e150] [cursor=pointer]
                - tab "Matrix" [ref=e153] [cursor=pointer]
          - tabpanel "Runs" [ref=e160]:
            - generic [ref=e161]:
              - complementary [ref=e162]:
                - generic [ref=e163]:
                  - generic [ref=e164]: Evaluations
                  - button "Compare runs (needs at least two)" [disabled] [ref=e165]
                  - button "New evaluation" [ref=e168] [cursor=pointer]
                - button "pending-evidence Succeeded Preserve truthful proof presentation. Score only 10s ago" [ref=e172] [cursor=pointer]:
                  - generic [ref=e173]:
                    - generic "pending-evidence" [ref=e174]
                    - img "Succeeded" [ref=e175]
                  - generic "Preserve truthful proof presentation." [ref=e177]
                  - generic [ref=e178]:
                    - generic [ref=e179]: Score only
                    - generic "10/8/2026, 11:31:08 AM" [ref=e180]: 10s ago
              - separator "Resize the evaluations list" [ref=e181]
              - main [ref=e182]:
                - generic [ref=e183]:
                  - generic [ref=e184]:
                    - generic [ref=e185]:
                      - heading "pending-evidence" [level=2] [ref=e188]
                      - generic [ref=e189]: Succeeded
                      - button "Delete this evaluation" [ref=e192] [cursor=pointer]
                    - paragraph [ref=e195]: Preserve truthful proof presentation.
                    - generic [ref=e196]:
                      - generic [ref=e197]: 1 iteration
                      - generic [ref=e198]: "Iteration 1: score pending"
                  - generic [ref=e200]:
                    - generic [ref=e201]:
                      - generic [ref=e202]: Iteration 1
                      - generic [ref=e203]: score-only
                      - generic [ref=e206]: Score update pending
                      - generic [ref=e208]: Succeeded
                    - generic [ref=e211]:
                      - generic [ref=e212]:
                        - generic [ref=e213]: Implementation
                        - generic [ref=e214]: score-only
                        - button "View code diff" [ref=e215] [cursor=pointer]
                      - paragraph [ref=e216]: score-only run (no agent)
                      - paragraph [ref=e217]: /var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-dirty-01Ie9E
                    - generic [ref=e218]:
                      - generic [ref=e219]:
                        - generic [ref=e220]:
                          - generic [ref=e221]: "98"
                          - generic [ref=e222]: Provisional composite
                        - generic [ref=e223]:
                          - generic "Proof pack status" [ref=e224]: "Proof: Pending"
                          - generic "How much of the done contract (tests, lint, proof) this iteration met" [ref=e225]: Done contract 0/100
                      - generic [ref=e226]:
                        - generic [ref=e227]:
                          - generic [ref=e228]: Tests
                          - generic [ref=e231]: "100"
                          - 'generic "`true # cargo test` → passed" [ref=e232]'
                        - generic [ref=e233]:
                          - generic [ref=e234]: Lint
                          - generic [ref=e237]: "100"
                          - 'generic "`true # cargo clippy` → passed" [ref=e238]'
                        - generic [ref=e239]:
                          - generic [ref=e240]: Diff
                          - generic [ref=e243]: "90"
                          - generic "2 file(s), +2/-0, risk 10" [ref=e244]
                        - generic [ref=e245]:
                          - generic [ref=e246]: Review
                          - generic [ref=e247]: Not run
                          - generic [ref=e248]: —
                        - generic [ref=e249]:
                          - generic [ref=e250]: Human
                          - generic [ref=e253]: "20"
                          - generic "Published rating" [ref=e254]
                      - button "Hide proof pack" [expanded] [ref=e255] [cursor=pointer]
                      - generic [ref=e258]:
                        - generic [ref=e259]:
                          - generic [ref=e260]:
                            - generic [ref=e262]: Diff
                            - generic [ref=e263]: ·
                            - generic [ref=e264]: Working tree diff
                            - generic [ref=e265]: ·
                            - generic [ref=e266]: Info
                          - generic [ref=e267]: diff --git a/tracked_0.txt b/tracked_0.txt index a4bdd23..d121c62 100644 --- a/tracked_0.txt +++ b/tracked_0.txt @@ -1 +1,2 @@ original 0 +MODIFIED on the working tree
                        - generic [ref=e269]:
                          - generic [ref=e271]: Command
                          - generic [ref=e272]: ·
                          - generic [ref=e273]: "true # cargo test"
                          - generic [ref=e274]: ·
                          - generic [ref=e275]: Passed
                        - generic [ref=e277]:
                          - generic [ref=e279]: Command
                          - generic [ref=e280]: ·
                          - generic [ref=e281]: "true # cargo clippy"
                          - generic [ref=e282]: ·
                          - generic [ref=e283]: Passed
                        - generic [ref=e284]:
                          - generic [ref=e285]:
                            - generic [ref=e287]: Approval
                            - generic [ref=e288]: ·
                            - generic [ref=e289]: Human rating
                            - generic [ref=e290]: ·
                            - generic [ref=e291]: Passed
                          - generic [ref=e292]: "rating: 5/5 Published rating"
                    - alert [ref=e294]:
                      - generic [ref=e295]: Couldn’t load the updated score. Your rating is saved. Retry to finish updating the score before promoting this skill.
                      - button "Retry" [ref=e298] [cursor=pointer]
                    - generic [ref=e299]:
                      - generic [ref=e300]: Your rating
                      - group "Your rating" [ref=e301]:
                        - button "Rate 1 of 5" [pressed] [ref=e302] [cursor=pointer]
                        - button "Rate 2 of 5" [ref=e305] [cursor=pointer]
                        - button "Rate 3 of 5" [ref=e308] [cursor=pointer]
                        - button "Rate 4 of 5" [ref=e311] [cursor=pointer]
                        - button "Rate 5 of 5" [ref=e314] [cursor=pointer]
                      - generic [ref=e317]: 1/5
                      - button "Save as regression case" [ref=e318] [cursor=pointer]
                    - generic [ref=e321]:
                      - generic [ref=e322]: Skill score-only
                      - button "Copy" [ref=e323] [cursor=pointer]
                      - button "Download" [ref=e324] [cursor=pointer]
                      - button "Promote" [ref=e325] [cursor=pointer]
        - button "Open the Otto bar" [ref=e326] [cursor=pointer]:
          - generic [ref=e329]: Ask Otto
          - generic [ref=e330]: ⌘K
    - contentinfo [ref=e331]:
      - generic [ref=e332]:
        - button "0 working" [ref=e333] [cursor=pointer]
        - status [ref=e335]
        - 'generic "Event stream: live" [ref=e336]': live
      - generic [ref=e338]:
        - button "main" [ref=e339] [cursor=pointer]
        - generic "Network listener" [ref=e343]: loopback
        - generic [ref=e346]: 11:31 AM
  - alert [ref=e348]:
    - generic [ref=e353]:
      - generic [ref=e354]: Couldn’t finish updating your rating
      - generic [ref=e355]: Fixture proof publication failed
    - button "Dismiss" [ref=e356] [cursor=pointer]
```

# Test source

```ts
  1  | import { test, expect } from '@playwright/test';
  2  | import { apiCtx, seedWorkspace, seedDirtyRepo } from './seed';
  3  | import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
  4  | 
  5  | test.use({ serviceWorkers: 'block' });
  6  | 
  7  | for (const scheme of ['light', 'dark'] as const) {
  8  |   test(`pending score never presents old Approval as current evidence ${scheme}`, async ({ page }, info) => {
  9  |     const { ctx, base } = await apiCtx();
  10 |     const workspace = await seedWorkspace(ctx, base);
  11 |     const repo = await seedDirtyRepo(ctx, base, workspace);
  12 |     const post = async (path: string, data: unknown) => {
  13 |       const response = await ctx.post(`${base}/api/v1${path}`, { data });
  14 |       expect(response.ok(), await response.text()).toBe(true);
  15 |       return response.json();
  16 |     };
  17 |     try {
  18 |       const golden = await post(`/workspaces/${workspace}/golden-tasks`, {
  19 |         name: 'Round 5 pending evidence', prompt: 'Preserve truthful proof presentation.', skill: 'pending-evidence',
  20 |         test_cmd: 'true # cargo test', lint_cmd: 'true # cargo clippy',
  21 |       });
  22 |       const created = await post(`/workspaces/${workspace}/skill-evaluations`, {
  23 |         source: { kind: 'library', reference: '' }, task: '', impl_cli: '', validations: [], iterations: 1,
  24 |         mode: 'score_only', golden_task_id: golden.id, target: { kind: 'path', path: repo.dir },
  25 |       });
  26 |       let run: any;
  27 |       await expect.poll(async () => {
  28 |         run = await (await ctx.get(`${base}/api/v1/skill-evaluations/${created.id}`)).json();
  29 |         return run.status;
  30 |       }, { timeout: 30_000 }).not.toBe('running');
  31 |       const iteration = run.iterations[0].id;
  32 |       run = await post(`/skill-evaluations/${run.id}/iterations/${iteration}/rate`, { rating: 5, note: 'Published rating' });
  33 |       let pending: any = null;
  34 |       const ratePath = `**/skill-evaluations/${run.id}/iterations/${iteration}/rate`;
  35 |       await page.route(ratePath, async route => {
  36 |         pending = structuredClone(run);
  37 |         pending.best_iteration = null;
  38 |         pending.best_score = null;
  39 |         Object.assign(pending.iterations[0], { human_rating: 1, score: 0 });
  40 |         Object.assign(pending.iterations[0].scoring, { proof_status: 'pending', done_score: 0 });
  41 |         Object.assign(pending.iterations[0].scoring.human, { rating: 1, score: 20 });
  42 |         await route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Fixture proof publication failed' } });
  43 |       });
  44 |       await page.route(`**/skill-evaluations/${run.id}`, route => pending ? route.fulfill({ json: pending }) : route.continue());
  45 |       await page.addInitScript(({ workspace, scheme }) => {
  46 |         localStorage.setItem('otto_workspace', workspace);
  47 |         localStorage.setItem('otto_scheme', scheme);
  48 |         localStorage.setItem('otto_theme', 'native');
  49 |         localStorage.setItem('otto_firstrun_dismissed', '1');
  50 |         localStorage.setItem('otto_rail_expanded', '0');
  51 |       }, { workspace, scheme });
  52 |       await page.setViewportSize({ width: 1440, height: 900 });
  53 |       await page.goto('/#/skills-eval');
  54 |       await page.getByTestId('tab-evaluator').click();
  55 |       await page.getByTestId('scorecard-proofpack-btn').click();
  56 |       const approval = page.locator('.scorecard .artifact').filter({ hasText: 'Human rating' });
  57 |       await expect(approval).toContainText('rating: 5/5');
  58 |       await page.getByRole('button', { name: 'Rate 1 of 5', exact: true }).click();
  59 |       await expect(page.locator('.rated')).toHaveText('1/5');
  60 |       await expect(page.getByTestId('scorecard-proof')).toContainText('Pending');
> 61 |       await expect(approval).toHaveCount(0);
     |                              ^ Error: expect(locator).toHaveCount(expected) failed
  62 |       await expect(page.getByTestId('proof-pending-evidence')).toBeVisible();
  63 |       await page.screenshot({ path: info.outputPath(`pending-evidence-desktop-${scheme}.png`) });
  64 |       await page.setViewportSize({ width: 390, height: 844 });
  65 |       await page.getByTestId('proof-pending-evidence').scrollIntoViewIfNeeded();
  66 |       await expectNoHorizontalOverflow(page);
  67 |       await expectFullyInViewport(page, page.getByTestId('proof-pending-evidence'));
  68 |       await page.screenshot({ path: info.outputPath(`pending-evidence-phone-${scheme}.png`) });
  69 |       await page.unroute(ratePath);
  70 |       pending = null;
  71 |       await page.locator('.score-pending').getByRole('button', { name: 'Retry', exact: true }).press('Enter');
  72 |       await expect(page.getByTestId('scorecard-proof')).toContainText('Passed');
  73 |       await expect(approval).toContainText('rating: 1/5');
  74 |       await expect(approval).not.toContainText('rating: 5/5');
  75 |       await expect(page.getByTestId('proof-pending-evidence')).toHaveCount(0);
  76 |     } finally { await ctx.dispose(); }
  77 |   });
  78 | }
  79 | 
```