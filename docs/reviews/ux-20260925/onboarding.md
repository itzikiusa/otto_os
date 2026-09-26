# First-run phone setup

The cross-page screenshot showed a 480px coach card inside a 375px viewport. Its grid's intrinsic column size defeated max-width and clipped the workspace folder browser and recommended-skill actions.

A browser regression failed in both LTR and RTL (480px > 375px). A minmax(0,1fr) grid column now constrains the card, long provider chips wrap, and workspace name/folder fields have accessible names. The folder remains LTR in RTL chrome. Both regressions pass, including control visibility and internal overflow checks (`desktop-ux-onboarding.spec.ts`, 2 passed).

Fresh shared-control review also found multiline provider badges overflowing their inherited fixed height. A regression measured 7px overflow in LTR and RTL, then passed six repeated cases after scoped automatic-height repair. The bounds assertion now retries across the normal bootstrap loading transition instead of dereferencing a transient null. Both corrected phone screenshots were visually inspected. Evidence: `/tmp/otto-ux-coach-chips-red.log`, `/tmp/otto-ux-coach-green.log`, `/tmp/otto-ux-coach-green/`.
