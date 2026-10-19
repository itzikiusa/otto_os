# D06 — Vault Docs, Design Hall, Canvas & Skills Lab UX

- **Reviewer Subagent ID:** `5e1b4878-1e07-4dcc-a108-e4be33d40cf3` (`D06 Vault Design Hall & Skills UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`) — Cross-review handoff to `Pele` (`grok`) for `vault/`, `design-hall/`, `canvas/`, `skills-lab/`, `skills-eval/`
- **Scope:** `ui/src/modules/{vault,design-hall,canvas,skills-lab,skills-eval}/*`
- **Iteration 1 Baseline Score:** **8.3 / 10**
- **Cross-Review Status:** Verified findings handed off to `Pele` (`grok`) who owns direct edits in these modules.

---

## 1. Confirmed Findings & Cross-Lane Handoff

### D06-01 (High) — `CanvasPage.svelte` opens on an empty "Select a canvas" pane even when scenes exist
- **Location:** `ui/src/modules/canvas/CanvasPage.svelte:205-225`
- **Repro:** Open Canvas in a workspace with existing scenes; `refreshScenes()` populates `scenes` without auto-selecting `scenes[0]`.
- **Fix Direction (Coordinated with Pele):** Auto-select `scenes[0].id` when `!activeId && scenes.length > 0`.

### D06-02 (High) — `CanvasPage.svelte`, `VaultPage.svelte`, and `DesignHallPage.svelte` initial load failures show toast only with no inline Retry state
- **Location:** `ui/src/modules/canvas/CanvasPage.svelte:220`, `ui/src/modules/vault/VaultPage.svelte:236`, `ui/src/modules/design-hall/DesignHallPage.svelte:98`
- **Fix Direction (Coordinated with Pele):** Track `loadErr = loadErrorText(e)` and render an inline `.error-banner` with a Retry button.

### D06-03 (High) — Global `.row`, `.card`, and `.chip` class collisions in `SkillsEvalPage.svelte`, `SkillsLabPage.svelte`, and `VaultPage.svelte`
- **Location:** `ui/src/modules/skills-eval/SkillsEvalPage.svelte:286, 369`, `ui/src/modules/skills-lab/SkillsLabPage.svelte:231`, `ui/src/modules/vault/VaultPage.svelte:2063`
- **Fix Direction (Coordinated with Pele):** Prefix module classes (`.se-row`, `.se-card`, `.sl-chip`, `.vault-card`).

### D06-04 (Medium) — Split-pane resizer handles in `VaultPage.svelte`, `CanvasPage.svelte`, and `DesignArtifactView.svelte` lack keyboard support and RTL delta inversion
- **Location:** `ui/src/modules/vault/VaultPage.svelte:1058, 1070`, `ui/src/modules/canvas/CanvasPage.svelte:190`, `ui/src/modules/design-hall/DesignArtifactView.svelte:320`
- **Fix Direction (Coordinated with Pele):** Add `role="separator"`, `tabindex="0"`, `onkeydown` arrow handlers, and RTL sign inversion (`document.documentElement.dir === 'rtl' ? -1 : 1`).
