# Room recap validation

This report distinguishes synthetic engine validation from a real recorded room.
No user microphone, desktop or live room was captured. No API key was used.

## Engine probes (2026-09-27)

Host: Apple M5 Pro, 48 GiB RAM, macOS 27.0.

- Apple `SpeechTranscriber.isAvailable` returned true, but installed locales were
  empty and supported locales did not include Hebrew. Otto's deployment target
  is macOS 12, while SpeechAnalyzer requires macOS 26. This ruled it out as the
  sole initial transcription engine.
- Built whisper.cpp CLI from upstream revision
  `d09f61a708f3487afa956ff578e60eae5e7a233c` in a temporary directory, using the
  multilingual `base` GGML model. The upstream public JFK sample (11 s, 16 kHz
  mono PCM) produced the expected timestamped English sentence. A warm standalone
  process took 0.23 s wall time, 0.11 s user CPU and 0.06 s system CPU, with
  maximum RSS 375,111,680 bytes (`/usr/bin/time -l`). This includes the recognizer,
  not an active four-person room; it does not establish combined performance.
- A synthetic Hebrew sentence generated locally with the installed Carmit voice
  produced Hebrew text but had spelling/word-boundary errors. The warm `base`
  run took 0.27 s wall time and maximum RSS 374,652,928 bytes. Language support is
  not evidence of production transcription accuracy, code-switching quality or
  correct names. No actual microphone was opened.
- Repeating the same synthetic Hebrew input with the multilingual `small` model
  improved spelling and spacing but still altered the person's name. It took
  1.07 s wall time with maximum RSS 877,690,880 bytes. This illustrates the
  quality/memory tradeoff; neither run establishes accuracy on real meetings.
- A generated 960×540 JPEG contained “Session recap test”, a retry-button decision
  and Bob's test-writing action. Production Vision code extracted those lines.
  A killable `room-ocr` subprocess now performs recognition with a 30 s deadline;
  this replaces an uncancelable blocking task. Its early dispatch starts no
  daemon listener, database, user-state initialization or logging subsystem.
- The actual freshly built `ottod room-ocr` helper initially exceeded a 35 s
  probe deadline. A stack sample localized the wait to Apple Neural Engine model
  compilation over XPC. Subsequent actual-daemon calls recognized the same image
  in 0.200–0.209 s. A newly linked synchronous helper, without Tokio, reproduced
  the cold delay (over 8 s, then 34.081 s, then 0.182 s), so runtime initialization
  was not the cause. Production retains its 30 s deadline: a timed-out OCR job
  preserves the screen image with empty text and an explicit `screen_text` gap.
  First-use latency after installing a new binary remains a native acceptance
  concern; warm timings do not establish cold performance.
- The production pipeline transcribed canonical WAV, recognized the generated
  JPEG, then called Codex with two synthetic evidence events and that image. It
  returned a structured draft citing source event IDs 1 and 2, the retry-button
  decision and Bob's action. The capability check reported ChatGPT subscription
  sign-in. The `codex exec` invocation used no API key.
- An independent local HTTP Responses fixture captured the installed Codex
  0.157.1 request with the exact disabled-tool switches, empty temporary Codex
  home and a synthetic local provider: `tools: null`. No external model call or
  user credentials were used in that tool-catalog check.

The production summary invocation uses an isolated temporary directory,
`--ignore-user-config`, `--ignore-rules`, `--ephemeral`, read-only sandbox,
`forced_login_method="chatgpt"`, disabled web/apps/plugins/hooks/shell/browser/
computer-use/multi-agent tools, and an allowlisted environment. Transcript data
is passed literally over stdin. Output and runtime are bounded. Unsupported
CLI switches cause failure rather than an unrestricted fallback. The current
model/runtime tool result does not prove future Codex versions behave identically.

## Automated coverage

Engine tests cover strict mono PCM bounds, segment timestamps and Hebrew text,
configuration bounds, malformed/oversized JPEG headers, process timeout/failure,
output limits, literal stdin, actual child termination on cancellation, UTF-8 chunk completeness, invalid model event IDs,
and explicit errors for empty or oversized summary input. The isolated harness
passed 11 tests. The final focused server run passed 43 room/recap/engine tests
plus the HTTP/WebSocket/PTY integration test, including consent cancellation,
final speech draining, a delayed archive writer, lazy recovery of interrupted
journals and paginated reads. See [feature verification](session-rooms.md).

Temporary probe files live outside the repository. The speech model and CLI are
not bundled, not checked in and not installed globally. Product users must set up
the local recognizer/model; setup surfaces missing prerequisites.

## Still requires acceptance

Real voices, multiple speakers, overlapping speech and mixed Hebrew/English;
packaged Otto AudioWorklet/capture behavior; sustained four-person sessions with
recognition enabled; queue backpressure under load; sleep/reconnect and consent
withdrawal during capture; model-dependent quality and memory on Intel/older
Apple Silicon. Browser/synthetic fixtures do not establish these results.
