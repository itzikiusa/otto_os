# Round 1 — Content modules (reviewer 4/10)

Scope: Product, Vault, Canvas, Design Hall, Browser. Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`. No commits, shared-component edits, or original checkout edits.

## Scores after repairs

| Dimension | /10 | Basis |
|---|---:|---|
| Layout | 9.0 | Consistent, readable primary layouts; repaired phone onboarding and assistant sizing. Canvas no-workspace state lacks a direct action. |
| Interaction | 8.8 | Verified meaningful Browser, Vault, Design Hall and Canvas flows. External Jira/agent generation/native live browser not exercised. |
| Accessibility | 8.2 | Product onboarding activated with keyboard; Canvas composer/provider now labeled; URL direction corrected. Source inspection still shows keyboard gaps in marking and Design Hall tabs. |
| States | 8.6 | Empty states viewed; list error/retry checks included. Vault note-open failure retains only a transient toast (source-level finding). |
| Responsive | 9.0 | Repaired and verified 375px Canvas/Product; verified 1024px RTL address editing, desktop loaded flows and long menus. Full loaded phone editor combinations remain for next round. |
| **Mean** | **8.72** | **Not eligible for 9.5: known remaining gaps and unverified external flows.** |

## Confirmed repairs

1. **Product empty phone onboarding was hidden.** `ProductPage.svelte:912,1446`: the zero-story list occupied the screen while the actionable empty state lived in the collapsed content section. Browser regression measured Import story bottom at **1049.55px in an 812px-high viewport**. Hide empty accordion chrome and give the empty-state pane the available height. After repair the CTA is immediately visible and keyboard Enter opens the import dialog.
2. **Canvas phone assistant overflowed horizontally.** `CanvasPage.svelte:329`: `width:380px` remained active on a 375px viewport; the pre-existing `overlay` class had no sizing rule. Browser regression measured right edge **380px against a 375px viewport** in both schemes. Phone assistant now occupies the available width; editor remains mounted while hidden and returns when the panel closes.
3. **Canvas light assistant empty state had unreadable hint contrast.** `ConversationPanel.svelte:175,191`: hardcoded dark shell behind theme-dependent dim text. Actually viewed light screenshot demonstrated the issue. Replaced shell/lead colors with semantic surface/text tokens; terminal itself still uses its own `forceDark` component styling. Also gave composer and provider explicit accessible labels. After light/dark screenshots actually viewed.
4. **Browser URL editing inherited RTL.** `BrowserView.svelte:661`: browser assertion measured `direction:rtl` for URL input. Added `dir="ltr"`; 1024px RTL check now passes with URL visibly ordered correctly.

## Verification

- `npm run check`: **passed**, svelte-check **0 errors / 0 warnings**, all TypeScript gates passed. Log `/tmp/otto-ux-content-check.log`.
- Broad isolated E2E run: **19/19 passed** using assigned slot `uxcontent`, daemon port 7815, Vite 5315, original prebuilt daemon.
- New durable spec: `ui/e2e/desktop-ux-content.spec.ts`.
- Broad existing specs passed: `desktop-browser-reader`, `desktop-design-hall`, `desktop-vault-docs`.
- Verified main flows: Browser page → mark → annotation rail → send to isolated shell session, save to real isolated vault; Design Hall draft → edit → save version → compare → confirmed restore, links/Used in, references, 40-project clamped menu; Vault real files → tree/read → wikilinks/backlinks → edit/autosave on disk → search/tags/quick switcher → OpenAPI reference resolution/multiple D2/minified JSON → OKF; Canvas seeded Mermaid → phone assistant → close → diagram retained; Product empty onboarding → keyboard-activated import dialog.
- Added 3 focused phone list-error → Retry → empty-state recovery cases (Product/Vault/Canvas). **Final rerun: 3/3 passed**; artifacts `/tmp/otto-ux-content-states`.
- After adding state checks, `npx tsc -p tsconfig.e2e.json` passed. A first Vault failure used a malformed test API error `{error:...}`; corrected to the contract `{code,message}`, not counted as a product finding.

## Visual evidence actually viewed

Initial: phone-light Product/Vault/Canvas/Design/Browser; native-light Product; native-dark Vault/Canvas; warm-dark Design; tablet-rtl Browser.

After: `/tmp/otto-ux-content-product-phone.png`, `/tmp/otto-ux-content-canvas-light.png`, `/tmp/otto-ux-content-canvas-dark.png`, `/tmp/otto-ux-content-browser-rtl.png`.

Canvas before: `/tmp/otto-ux-content-canvas-before/desktop-ux-content-Canvas--7076a-nt-fits-and-closes-in-light-desktop-browser/test-failed-1.png` (actually viewed).

## Concrete remaining issues for fresh review

- **Browser keyboard marking — source-confirmed, browser repair/repro pending (P2).** `ReaderView.svelte:152,165–176`: Mark passage tells users to click, and marking only hooks article `onclick`; generated headings/paragraphs are not keyboard controls. Define a keyboard selection/marking path and verify it without mouse use.
- **Vault note-open errors — source-confirmed, injected-error browser repro pending (P2).** `vault.svelte.ts:514–540`: failed note reads only call a toast and return false; no persistent note error or Retry. Keep last good content but show the failed target and retry action inline.
- **Design Hall tabs — source-confirmed, browser key repro pending (P2).** `Lobby.svelte:263–271`, `ArtifactView.svelte:818–828`, `CompareModal.svelte:123`: role=tablist/tab groups lack arrow/Home/End handlers and roving tab stops. LearnedPage and VersionStrip do implement keyboard behavior; align the remaining groups.
- **Canvas no-workspace onboarding — screenshot-confirmed (P2).** `CanvasPage.svelte:174–176`: "Pick or create one" has no action; on a phone no workspace control is visible on the page. Add a direct workspace chooser/create affordance using the existing shared workspace workflow.

Limits: no live Jira/Confluence account, paid agent invocation, native Tauri/webview or external publishing; no VoiceOver manual pass. Warm dark/RTL screenshots inspected at initial states, but deep loaded editor flows in every theme/device combination need subsequent review. Native light/dark main page chrome was visually assessed; screenshots are browser renderings, not a running macOS native app.
