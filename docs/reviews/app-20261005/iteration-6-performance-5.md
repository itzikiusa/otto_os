# Iteration 6 — performance partition 5

**Verdict: Approve at the bounded repair scope; performance acceptance incomplete.** Blocker 0, major 0, minor 0, nit 0. **Provisional score: 9.4/10**, up from the preserved iteration-5 score of 8.9 through new measurement evidence only. No new source finding or claim of a memory plateau.

This is an evidence-delta review under the performance-review skill. Source checkpoint supplied by coordinator: `c425aa82` plus Audit draft. Read iteration-5-performance-5.md, current PERFORMANCE.md and VERIFICATION.md; the unchanged recap cost source retains the earlier review. No broad scan, tests, builds, browser runs, benchmarks, source changes, git mutations or agents were performed. Only this report was added.

## Evidence delta

- The isolated synthetic N=0/1/3/5 scale retry completed in **375.4 seconds, 66 samples, exit 0, zero teardown leftovers**, under unchanged limits. At N=5 daemon CPU was 2.3–2.6% and renderer CPU 12.7–14.8%, depending on view; GPU and other process groups are additional cost. Post-GC baseline→N5 daemon RSS was 162.7→272.4 MiB, renderer RSS 530.1→712.4 MiB and JS heap 17.2→32.0 MiB. Recovery returned CPU near idle but retained elevated memory. These are useful controlled observations, not proof that retained RSS is a leak or harmless.
- Measurements used **64a850e6**, fresh debug daemon and frozen headed Chromium/WebGL UI, synthetic terminal/transcript workloads and tiled/focus/Home/Git views. They directly improve Home/shared-shell coverage within partition 5. They are not measurements of all partition-5 products or the later final source. Subsequent History/Audit and version/tour changes must remain distinguished in provenance.
- The requested **15-minute** N=3 sustained run stopped at **413.9 seconds** when host load reached **12.03**, above the unchanged cap of 12. Teardown left zero processes; 77 partial samples did not reach final recovery/GC. This is an aborted run, not acceptance. The RSS trend remains unresolved.
- Read-only installed-app observation adds 25 samples over 120 seconds: desktop RSS 100.80→100.81 MiB; aggregate matching daemon/bridge RSS 258.34→256.77 MiB. The installed revision differs and native WebKit/GPU helpers were not attributed. These samples cannot substitute for candidate/native sustained acceptance.
- Earlier recap counters remain the direct repair evidence: across 15 unchanged checks and four P=1/100 × 2/20 KiB payload fixtures, zero event opens/draft reads/index builds; revision transfer totaled 10,545–10,665 bytes. Thus the prior O(K×(B+D)) repeated body work is still replaced by initial body work plus O(K) metadata checks. No new CPU/RSS speedup is inferred from counters.
- The new combined recap revision-failure → Retry → held A response → B identity-switch WebKit case is **1/1 green**, recorded in `/tmp/otto-review06-recap.log`. Three PersonalAgents mounted ABA cases passed within the **18/18** acceptance run. These strengthen lifecycle/recovery behavior but do not measure departed-request memory retention. Coordinator additionally reports final UI **0 errors/0 warnings and 1,335 unit passes**; those counts are not performance measurements.

## Fixed PLAN dimensions

| Dimension | /2 | Evidence and explicit deduction |
|---|---:|---|
| Query/network work | 2.0 | Preserve R5: recap metadata-only counters, owner-scoped HTTP and bounded page/revision protocol directly cover the repaired recurring cost. No new material gap in that bounded claim. |
| Algorithmic/serialization cost | 2.0 | Preserve R5: no transcript-body scan in revision checks; bounded page and two-attempt race recovery. Existing byte counters directly support serialization reduction. |
| Retained memory/lifecycle | 1.9 | Preserve the 0.1 deduction: mounted Retry/identity and ABA evidence improves publication ownership, but repeated archive switches with held manual requests and post-settlement memory remain unmeasured. The new test is not an abort or reclamation test. |
| Scheduling/responsiveness | 2.0 | Preserve R5 bounded result: actual poller backoff/recovery and mounted identity controls pass; the combined error/Retry/switch case strengthens this evidence. |
| Representative CPU/RAM/latency measurements | 1.5 | Increase from 1.0: named controlled N=0/1/3/5 passes now cover substantial shared/Home work, plus live observation. Deduct 0.5 for incomplete sustained recovery/plateau evidence and missing direct recap CPU/RSS/latency measurement; candidate/native attribution also remains limited. This matches PLAN's substantial executed matrix with meaningful gaps. |
| **Total** | **9.4/10** | Below 9.8; no arbitrary score cap and no conversion of an aborted workload into a pass. |

Finite remaining acceptance: complete the prescribed 15-minute N=3 run under unchanged safety limits with final recovery/GC and explain the renderer RSS trend; measure stable/stopped recap and repeated archive switching at the already defined four page/payload fixtures, including post-request-settlement memory and navigation latency; clearly attribute native helpers in any candidate-versus-installed comparison. Physical room media, credential-dependent cloud and plugin-process workloads remain outside this bounded score. Root should preserve those scope exclusions rather than imply whole-partition runtime certification.
