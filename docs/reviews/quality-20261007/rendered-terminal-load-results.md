# Real rendered-terminal load — measured results

The corrected WebKit run passed with 1/3/5 real Terminal widgets connected to owned local fake-CLI PTYs. Every terminal rendered all 96 submitted markers, with no missed markers, no remaining parser/render queues, real credit acknowledgements, and no recorded fatal UI errors. All nine widgets used WebGL.

Evidence: [raw report](evidence/rendered-terminal-load-full.json), [independent analysis](evidence/rendered-terminal-load-analysis.json), and [live process identity snapshot](evidence/rendered-webkit-owned-processes.json). Every one of the 360 browser resource rows contains all four verified process IDs: root 80464, Networking 80469, GPU 80470, WebContent 80488. The single owned browser run, matching engine path and resource coalition establish attribution; coalition IDs alone are not unique test-run identifiers.

| Terminals | Per-terminal latency p95 | Worst marker | Frame p95 | Mean total CPU | Mean total RSS MiB | Peak total RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 20 ms | 22 ms | 18 ms | 14.33% | 1063.60 | 1213.47 |
| 3 | 23–24 ms | 28 ms | 18 ms | 15.76% | 1153.40 | 1178.41 |
| 5 | 26–28 ms | 31 ms | 18 ms | 20.92% | 1264.11 | 1432.45 |

The marker latency follows actual clipboard input through xterm, daemon transport, PTY echo, parser and render notification. It is not a compositor presentation timestamp. Each tile sent 82,656 bytes (about 861 bytes/second at the one-second paste cadence), below the 4 KiB/second cap. Across all phases the transport recorded nine credit grants, 18 acknowledgements and 864 input frames. Peak pending parser bytes were at most 1,273; peak queued bytes were zero; every tile drained to zero.

The harness requested 90 load observations and 30 recovery observations per concurrency. Sampling overhead extends the observed load timestamp span to 95.45–95.81 seconds and recovery to 31.02–31.30 seconds; this explains 96 markers per tile. These are bounded phases, not exact 90-second wall-clock windows.

| Terminals retained during quiet period | Mean total CPU | First quiet RSS MiB | Last quiet RSS MiB |
| --- | ---: | ---: | ---: |
| 1 | 9.81% | 1058.69 | 1058.83 |
| 3 | 8.71% | 1178.81 | 1178.67 |
| 5 | 12.72% | 1278.95 | 1278.75 |

Quiet memory is stable over each observed period. The widgets, owned sessions and fixture's animation-frame observer remain mounted/running: quiet CPU is not a measurement of app-idle CPU. Browser mean load RSS rises 459.58→513.35→547.22 MiB across N=1/3/5, while daemon mean rises 152.81→160.18→174.11 MiB. The sequential shared browser/backend prevents attributing all differences solely to concurrency or declaring long-duration leak freedom. The isolated workload uses local echo CLIs, not real provider inference or a complex streaming TUI.

This run closes the missing actual-renderer and missing WebKit-process validation gaps. The separate fresh-browser matched raw test addresses the remaining large browser-memory confound in the earlier off/on run.
