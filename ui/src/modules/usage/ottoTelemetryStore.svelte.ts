import { announce } from '../../lib/announce.svelte';
import { api } from '../../lib/api/client';
import type {
  TelemetryConfig,
  TelemetryOverview,
  TelemetryProfile,
  TelemetrySpan,
  TelemetryStatus,
  TelemetrySuggestion,
} from '../../lib/api/types';
import { loadErrorText } from '../../lib/loadError';
import { pollWhileVisible } from '../../lib/poll';
import { telemetrySettingsChanged } from '../../lib/telemetryBoot';
import { usageUi, type UsageSubView } from './usageUiState.svelte';

export { usageUi, type UsageSubView };

class OttoTelemetryStore {
  config = $state<TelemetryConfig | null>(null);
  savedConfig = $state('');
  get dirty(): boolean {
    return !!this.config && JSON.stringify(this.config) !== this.savedConfig;
  }

  status = $state<TelemetryStatus | null>(null);
  overview = $state<TelemetryOverview | null>(null);
  suggestions = $state<TelemetrySuggestion[]>([]);
  loading = $state(true);
  loadedOnce = $state(false);
  busy = $state(false);
  analyzing = $state(false);
  error = $state('');
  loadError = $state('');
  message = $state('');

  trace = $state<TelemetrySpan[] | null>(null);
  traceBusy = $state(false);
  traceError = $state('');
  traceId = $state<string | null>(null);
  profile = $state<TelemetryProfile | null>(null);

  hours = $state(24);
  operationPage = $state(0);

  private loadSeq = 0;
  private detailSeq = 0;

  private setMessage(msg: string): void {
    this.message = msg;
    if (msg) announce(msg);
  }

  private setError(msg: string): void {
    this.error = msg;
    if (msg) announce(msg, { assertive: true });
  }

  discardChanges(): void {
    if (this.savedConfig) {
      try {
        this.config = JSON.parse(this.savedConfig) as TelemetryConfig;
      } catch {
        /* ignore corrupt snapshot */
      }
    }
    this.message = '';
    this.error = '';
  }

  async load(forceInitial = false): Promise<void> {
    const seq = ++this.loadSeq;
    const requestedHours = this.hours;
    if (forceInitial || !this.loadedOnce) this.loading = true;
    this.loadError = '';
    try {
      const [cfg, state] = await Promise.all([
        api.get<TelemetryConfig>('/telemetry/config'),
        api.get<TelemetryStatus>('/telemetry/status'),
      ]);
      if (seq !== this.loadSeq) return;
      if (!this.config || forceInitial || !this.dirty) {
        this.config = cfg;
        this.savedConfig = JSON.stringify(cfg);
      }
      this.status = state;
      if (state.collector_ready || !state.enabled) {
        const [data, items] = await Promise.all([
          api.get<TelemetryOverview>(`/telemetry/overview?hours=${requestedHours}`),
          api.get<TelemetrySuggestion[]>('/telemetry/suggestions'),
        ]);
        if (seq !== this.loadSeq) return;
        this.overview = data;
        this.suggestions = items;
      }
      this.loadedOnce = true;
    } catch (e) {
      if (seq === this.loadSeq) this.loadError = loadErrorText(e);
    } finally {
      if (seq === this.loadSeq) this.loading = false;
    }
  }

  mount(): () => void {
    if (!this.analyzing && !this.busy) {
      void this.load(false);
    }
    const poller = pollWhileVisible(
      () => {
        if (this.status?.enabled && !this.busy && !this.loading) return this.load(false);
      },
      { ms: 15_000, immediate: false },
    );
    return () => {
      poller.stop();
    };
  }

  async save(): Promise<void> {
    if (!this.config || this.busy) return;
    this.busy = true;
    this.setError('');
    this.setMessage('');
    try {
      const next = await api.put<TelemetryConfig>('/telemetry/config', { ...this.config });
      this.config = next;
      this.savedConfig = JSON.stringify(next);
      telemetrySettingsChanged();
      this.setMessage(
        next.enabled
          ? 'Settings saved. The local collector starts automatically.'
          : 'Collection stopped. Retained history expires on its schedule.',
      );
      await this.load(false);
    } catch (e) {
      this.setError(loadErrorText(e));
    } finally {
      this.busy = false;
    }
  }

  async analyze(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.analyzing = true;
    this.setError('');
    try {
      this.suggestions = await api.post<TelemetrySuggestion[]>('/telemetry/analyze');
      await this.load(false);
    } catch (e) {
      this.setError(loadErrorText(e));
    } finally {
      this.analyzing = false;
      this.busy = false;
    }
  }

  async dismiss(id: string): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    try {
      await api.post(`/telemetry/suggestions/${encodeURIComponent(id)}/dismiss`);
      this.suggestions = this.suggestions.map((s) => (s.id === id ? { ...s, dismissed: true } : s));
    } catch (e) {
      this.setError(loadErrorText(e));
    } finally {
      this.busy = false;
    }
  }

  async openTrace(id: string): Promise<void> {
    const seq = ++this.detailSeq;
    this.traceId = id;
    this.trace = [];
    this.traceBusy = true;
    this.traceError = '';
    try {
      const spans = await api.get<TelemetrySpan[]>(`/telemetry/traces/${encodeURIComponent(id)}`);
      if (seq === this.detailSeq) this.trace = spans;
    } catch (e) {
      if (seq === this.detailSeq) this.traceError = loadErrorText(e);
    } finally {
      if (seq === this.detailSeq) this.traceBusy = false;
    }
  }

  closeTrace(): void {
    this.detailSeq++;
    this.traceId = null;
    this.trace = null;
    this.traceBusy = false;
    this.traceError = '';
  }

  async viewLatestProfile(): Promise<void> {
    try {
      const next = await api.get<TelemetryProfile | null>('/telemetry/profile');
      this.profile = next;
      this.setMessage(next ? '' : 'No retained native profile yet.');
    } catch (e) {
      this.setError(loadErrorText(e));
    }
  }

  async capture(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.setError('');
    try {
      this.profile = await api.post<TelemetryProfile>('/telemetry/profile');
    } catch (e) {
      this.setError(loadErrorText(e));
    } finally {
      this.busy = false;
    }
  }

  resetForTests(): void {
    this.loadSeq++;
    this.detailSeq++;
    this.config = null;
    this.savedConfig = '';
    this.status = null;
    this.overview = null;
    this.suggestions = [];
    this.loading = true;
    this.loadedOnce = false;
    this.busy = false;
    this.analyzing = false;
    this.error = '';
    this.loadError = '';
    this.message = '';
    this.trace = null;
    this.traceBusy = false;
    this.traceError = '';
    this.traceId = null;
    this.profile = null;
    this.hours = 24;
    this.operationPage = 0;
    usageUi.telemetryDirty = false;
  }
}

export const ottoTelemetry = new OttoTelemetryStore();
usageUi.onDiscardTelemetry = () => ottoTelemetry.discardChanges();
