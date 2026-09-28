// Opt-in diagnostic UI. Capture occurs only inside explicit button handlers.
// No media bytes, device names, screen titles, or transcripts are saved.
void (async () => {
  const role = window.__probeRole;
  document.body.innerHTML = `<main style="font:16px system-ui;max-width:760px;margin:40px auto;padding:24px">
    <h1>Otto physical media check — ${role}</h1>
    <p>This isolated test starts no daemon. Capture begins only when you press a capture button and accept the system prompt. Choose a neutral test window. Nothing is recorded or sent to another participant.</p>
    <p>Speak briefly after enabling the microphone. After selecting a window, resize it and stop sharing using the system control. Check that the operating-system indicators clear after Stop all.</p>
    <p><button id="mic">Enable microphone</button> <button id="screen">Choose display or window</button> <button id="stop">Stop all capture</button></p>
    <video id="preview" muted playsinline style="max-width:100%;max-height:300px"></video>
    <pre id="status" style="white-space:pre-wrap" aria-live="polite"></pre>
    <p><label><input id="indicators" type="checkbox"> I checked that capture indicators cleared after stopping.</label></p>
    <button id="finish">Finish this window and save metadata only</button>
    </main>`;
  const result = {role, microphoneSamples: 0, microphonePeakRms: 0, videoFrames: 0, displaySurface: '', geometries: [], permissionErrors: [], systemStoppedVideo: false};
  let microphone, screen, context, input, processor, timer, busy = false, epoch = 0;
  const owned = [];
  const status = () => { document.querySelector('#status').textContent = JSON.stringify(result, null, 2); };
  const stop = async () => {
    epoch++;
    clearInterval(timer);
    processor?.disconnect(); input?.disconnect();
    for (const stream of [microphone, screen]) for (const track of stream?.getTracks() ?? []) track.stop();
    microphone = screen = null;
    if (context && context.state !== 'closed') await context.close();
    document.querySelector('#preview').srcObject = null;
    status();
  };
  document.querySelector('#mic').onclick = async () => {
    if (busy || microphone) return; busy = true;
    const started = epoch;
    try {
      microphone = await navigator.mediaDevices.getUserMedia({audio: true, video: false});
      owned.push(...microphone.getTracks());
      if (started !== epoch) { await stop(); return; }
      context = new AudioContext();
      await context.audioWorklet.addModule(new URL('/room-recap-worklet.js', location.href).href);
      await context.resume();
      processor = new AudioWorkletNode(context, 'room-recap-pcm');
      processor.port.onmessage = ({data}) => {
        if (data.samples?.length) {
          let energy = 0; for (const value of data.samples) energy += value * value;
          result.microphoneSamples += data.samples.length;
          result.microphonePeakRms = Math.max(result.microphonePeakRms, Math.sqrt(energy / data.samples.length));
          status();
        }
      };
      input = context.createMediaStreamSource(microphone); input.connect(processor); processor.connect(context.destination);
      timer = setInterval(() => processor.port.postMessage('flush'), 1000);
    } catch (error) { result.permissionErrors.push({kind: 'microphone', name: error.name}); await stop(); }
    finally { busy = false; status(); }
  };
  document.querySelector('#screen').onclick = async () => {
    if (busy || screen) return; busy = true;
    const started = epoch;
    try {
      screen = await navigator.mediaDevices.getDisplayMedia({video: {width: {ideal: 1920}, height: {ideal: 1080}, frameRate: {ideal: 15, max: 15}}, audio: false});
      owned.push(...screen.getTracks());
      if (started !== epoch) { await stop(); return; }
      const track = screen.getVideoTracks()[0];
      result.displaySurface = track.getSettings().displaySurface ?? 'unreported';
      track.addEventListener('ended', () => { result.systemStoppedVideo = true; screen = null; status(); }, {once: true});
      const preview = document.querySelector('#preview'); preview.srcObject = screen; await preview.play();
      const frame = () => {
        if (track.readyState !== 'live') return;
        result.videoFrames++;
        const geometry = `${preview.videoWidth}x${preview.videoHeight}`;
        if (result.geometries.at(-1) !== geometry) result.geometries.push(geometry);
        if (result.videoFrames % 10 === 0) status();
        preview.requestVideoFrameCallback(frame);
      };
      preview.requestVideoFrameCallback(frame);
    } catch (error) { result.permissionErrors.push({kind: 'display', name: error.name}); await stop(); }
    finally { busy = false; status(); }
  };
  document.querySelector('#stop').onclick = stop;
  document.querySelector('#finish').onclick = async () => {
    if (busy) return;
    await stop();
    const checks = {microphone_pcm: result.microphoneSamples > 0, video_frames: result.videoFrames > 0,
      tracks_ended: owned.every(track => track.readyState === 'ended'), audio_closed: !context || context.state === 'closed',
      user_checked_indicators: document.querySelector('#indicators').checked};
    window.__probeResult = {...result, checks, passed: Object.values(checks).every(Boolean), note: 'User-driven capture capability check, no media retained; not four-person networking or performance acceptance.'};
    document.querySelector('#finish').disabled = true;
  };
  window.addEventListener('pagehide', () => { void stop(); });
  if (role === 'local') {
    await window.__TAURI_INTERNALS__.invoke('daemon_restart'); // Harmless fixture sentinel.
    await window.__TAURI_INTERNALS__.invoke('open_room_window', {url: window.__probeInvitation});
  }
  status();
})();
