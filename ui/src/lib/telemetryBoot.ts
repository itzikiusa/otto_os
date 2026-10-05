import { api, baseUrl, getToken } from './api/client';
import type { TelemetryConfig } from './api/types';
import { configureTelemetry, beginNavigation } from './telemetry';

/** Auth-scoped lifecycle. An old config response cannot reactivate after logout. */
export function bootTelemetry(module: string): () => void {
  let stopped = false;
  let generation = 0;
  let retryMs = 5000;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let channel: BroadcastChannel | null = null;
  const controller = new AbortController();
  const update = async (): Promise<void> => {
    if (stopped) return;
    const seq = ++generation;
    try {
      const config = await api.get<TelemetryConfig>('/telemetry/config', controller.signal);
      if (stopped || seq !== generation) return;
      if (config.enabled) {
        await import('./telemetryRuntime');
        if (stopped || seq !== generation) return;
      }
      retryMs = 5000;
      configureTelemetry(config.enabled, async (spans, signal) => {
        const token = getToken();
        if (!token) throw new Error('Signed out');
        const response = await fetch(`${baseUrl()}/api/v1/telemetry/ingest`, {
          method: 'POST', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
          body: JSON.stringify({ spans }), signal,
        });
        if (!response.ok) throw new Error('Telemetry unavailable');
      });
      if (config.enabled) {
        // Administrative changes outside this webview are noticed within 30 s;
        // server collection and ingestion stop immediately when disabled.
        timer = setTimeout(() => void update(), 30000);
      }
    } catch {
      if (!stopped && seq === generation) {
        configureTelemetry(false);
        timer = setTimeout(() => void update(), retryMs);
        retryMs = Math.min(60000, retryMs * 2);
      }
    }
  };
  const refresh = (): void => {
    if (timer) clearTimeout(timer);
    void update();
  };
  try {
    channel = new BroadcastChannel('otto-telemetry-settings');
    channel.onmessage = () => { configureTelemetry(false); refresh(); };
  } catch { /* A same-window settings event still works in restricted webviews. */ }
  window.addEventListener('otto:telemetry-settings', refresh);
  void update().then(() => {
    if (!stopped) {
      const done = beginNavigation(module);
      requestAnimationFrame(() => done());
    }
  });
  return () => {
    stopped = true;
    controller.abort();
    if (timer) clearTimeout(timer);
    channel?.close();
    window.removeEventListener('otto:telemetry-settings', refresh);
    configureTelemetry(false);
  };
}

export function telemetrySettingsChanged(): void {
  configureTelemetry(false);
  window.dispatchEvent(new Event('otto:telemetry-settings'));
  try {
    const channel = new BroadcastChannel('otto-telemetry-settings');
    channel.postMessage('changed');
    channel.close();
  } catch { /* Cross-window config polls provide the fallback. */ }
}
