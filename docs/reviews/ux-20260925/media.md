# Instrumental walkthrough evidence

The new original 112 BPM instrumental edition retains the published H.264 video stream and bundled instructional captions; narration is removed from the Remotion source.

- Output: `otto-tour-instrumental-20260925.mp4` (new versioned filename).
- Duration: 177.033333 seconds; 1920 × 1080, 30 fps; AAC stereo, 48 kHz.
- Original and updated compressed video stream SHA256: `3f6f7bfd1ef51641084c0e61678e72d966e175c89e36aa6a0aba441eabd83492`.
- Full ffmpeg decode completed without errors. Encoded audio: −16.32 LUFS, −1.99 dBTP, 2.60 LU loudness range.
- Tour TypeScript check passes.
- Source soundtrack is procedural, with seeded synthesis and no third-party samples.
- Uploaded the new versioned asset to the walkthroughs release. Chromium and iPhone WebKit verified actual instrumental playback/audio decoding, captions, seeking, chapter playback, no autoplay, and Retry (12/12 Help checks). See [Help review](r1-help.md) for evidence and the remaining chapter-scroll issue.
