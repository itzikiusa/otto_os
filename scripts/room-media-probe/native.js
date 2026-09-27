// Executed by the isolated Tauri example in local and production guest windows.
// Synthetic sources only; never requests microphone, display, or camera access.
(async () => {
  if (window.__probeStarted) return;
  window.__probeStarted = true;
  const checks = {};
  const errors = [];
  const invoke = (...args) => window.__TAURI_INTERNALS__.invoke(...args);
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const bounded = (promise, ms = 4000) => Promise.race([promise, pause(ms).then(() => { throw new Error('fixture timeout'); })]);
  const role = window.__probeRole;
  let context, oscillator, destination, input, processor;
  try {
    checks.secure = isSecureContext;
    checks.capture_api_exposed = typeof navigator.mediaDevices?.getDisplayMedia === 'function' && typeof navigator.mediaDevices?.getUserMedia === 'function';
    if (role === 'local') {
      await bounded(invoke('plugin:window|title', {label: 'main'}));
      checks.local_ipc = true;
      checks.custom_local_ipc = await bounded(invoke('daemon_restart')) === 1;
      const label = await bounded(invoke('open_room_window', {url: window.__probeInvitation}));
      checks.production_guest_created = label.startsWith('room-');
    } else {
      for (const [name, command, args] of [
        ['custom_command_denied', 'daemon_restart', {}],
        ['room_creation_denied', 'open_room_window', {url: window.__probeInvitation}],
        ['local_window_read_denied', 'plugin:window|title', {label: 'main'}],
        ['local_window_mutation_denied', 'plugin:window|set_title', {label: 'main', title: 'MUTATION MUST NOT HAPPEN'}],
        ['event_listen_denied', 'plugin:event|listen', {event: 'otto://probe', target: {kind: 'Any'}, handler: 314159}],
      ]) {
        try { await bounded(invoke(command, args)); checks[name] = false; }
        catch (error) {
          const message = String(error);
          // A timeout or unknown-command failure is not proof of authorization denial.
          checks[name] = /not allowed|not permitted|denied|local Join room/i.test(message);
          checks[`${name}_secret_absent`] = !message.includes(window.__probeInvitation.split('/').at(-1));
          if (!checks[name]) errors.push(`${name}: ${message}`);
        }
      }
      window.open('https://example.invalid/', '_blank');
    }
    for (let cycle = 0; cycle < window.__probeCycles; cycle++) {
    context = new AudioContext();
    await bounded(context.audioWorklet.addModule(new URL('/room-recap-worklet.js', location.href).href));
    await bounded(context.resume());
    oscillator = context.createOscillator(); oscillator.frequency.value = 440;
    destination = context.createMediaStreamDestination(); oscillator.connect(destination); oscillator.start();
    input = context.createMediaStreamSource(destination.stream);
    processor = new AudioWorkletNode(context, 'room-recap-pcm');
    input.connect(processor); processor.connect(context.destination);
    let samples = 0, sum = 0, rate = 0;
    await bounded(new Promise(resolve => {
      processor.port.onmessage = ({data}) => {
        if (data.samples) { samples += data.samples.length; rate = data.sampleRate; for (const x of data.samples) sum += x * x; }
        if (data.flushed) resolve();
      };
      setTimeout(() => processor.port.postMessage('flush'), 1200);
    }));
    checks.worklet_pcm = samples >= rate * .5 && Math.sqrt(sum / samples) > .5;
    if (!checks.worklet_pcm) throw new Error(`Synthetic PCM failed on cycle ${cycle + 1}`);
    checks.worklet_flush = true;
    processor.disconnect(); input.disconnect(); oscillator.stop(); oscillator.disconnect();
    for (const track of destination.stream.getTracks()) track.stop();
    await context.close();
    if (context.state !== 'closed' || destination.stream.getTracks().some(track => track.readyState !== 'ended')) throw new Error('Audio lifecycle cleanup failed');
    processor = input = oscillator = destination = context = null;
    }
    checks.audio_closed = true;
    if (role === 'guest') {
      // Each prohibited navigation must leave the original invitation document alive.
      const original = location.href;
      for (const url of ['tauri://localhost/', 'https://tauri.localhost/', new URL('/elsewhere', original).href]) {
        location.href = url;
        await pause(150);
        if (location.href !== original) throw new Error('Guest navigation escaped');
      }
      checks.navigation_blocked = true;
    }
  } catch (error) { errors.push(String(error)); }
  finally {
    processor?.disconnect(); input?.disconnect();
    if (oscillator) { oscillator.stop(); oscillator.disconnect(); }
    for (const track of destination?.stream.getTracks() ?? []) track.stop();
    if (context) { await context.close(); checks.audio_closed = context.state === 'closed'; }
    checks.tracks_ended = (destination?.stream.getTracks() ?? []).every(track => track.readyState === 'ended');
    window.__probeResult = {role, cycles: window.__probeCycles, checks, errors, passed: errors.length === 0 && Object.values(checks).every(Boolean)};
  }
})();
