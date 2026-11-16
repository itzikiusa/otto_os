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
import { plural } from '../../lib/plural';
import { pollWhileVisible } from '../../lib/poll';
import { telemetrySettingsChanged } from '../../lib/telemetryBoot';

export interface TelemetryScope {
  hours: number;
}

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

  togglingEnabled = $state(false);
  savingRetention = $state(false);
  clearing = $state(false);
  dismissingId = $state<string | null>(null);
  analyzing = $state(false);
  capturing = $state(false);

  get busy(): boolean {
    return (
      this.togglingEnabled ||
      this.savingRetention ||
      this.clearing ||
      this.analyzing ||
      this.capturing
    );
  }

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
  private scopeGetter: (() => TelemetryScope) | null = null;

  configure(opts: { getScope?: () => TelemetryScope }): void {
    this.scopeGetter = opts.getScope ?? null;
  }

  private currentHours(): number {
    return this.scopeGetter ? this.scopeGetter().hours : this.hours;
  }

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
    const requestedHours = this.currentHours();
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

  mount(opts?: { getScope?: () => TelemetryScope }): () => void {
    if (opts?.getScope) this.scopeGetter = opts.getScope;
    if (!this.analyzing && !this.togglingEnabled && !this.savingRetention) {
      void this.load(false);
    }
    const poller = pollWhileVisible(
      () => {
        if (this.status?.enabled && !this.togglingEnabled && !this.savingRetention && !this.loading) {
          return this.load(false);
        }
      },
      { ms: 15_000, immediate: false },
    );
    return () => {
      poller.stop();
    };
  }

  async toggleEnabled(nextEnabled: boolean): Promise<void> {
    if (!this.config || this.togglingEnabled || this.savingRetention) return;
    const prevEnabled = this.config.enabled;
    this.config = { ...this.config, enabled: nextEnabled };
    this.togglingEnabled = true;
    this.setError('');
    this.setMessage('');
    try {
      const next = await api.put<TelemetryConfig>('/telemetry/config', { ...this.config });
      this.config = next;
      this.savedConfig = JSON.stringify(next);
      telemetrySettingsChanged();
      this.setMessage(
        next.enabled
          ? 'Collection enabled. The local collector starts automatically.'
          : 'Collection stopped. Retained history expires on its schedule.',
      );
      await this.load(false);
    } catch (e) {
      if (this.config) this.config = { ...this.config, enabled: prevEnabled };
      this.setError(loadErrorText(e));
    } finally {
      this.togglingEnabled = false;
    }
  }

  async save(): Promise<void> {
    if (!this.config || this.savingRetention || this.togglingEnabled) return;
    this.savingRetention = true;
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
      this.savingRetention = false;
    }
  }

  async analyze(): Promise<void> {
    if (this.analyzing) return;
    this.analyzing = true;
    this.setError('');
    try {
      const found = await api.post<TelemetrySuggestion[]>('/telemetry/analyze');
      this.suggestions = found;
      const openCount = found.filter((s) => !s.dismissed).length;
      announce(
        openCount > 0
          ? `Analysis complete: ${plural(openCount, 'performance suggestion')}.`
          : 'Analysis complete: no open performance suggestions.',
      );
      await this.load(false);
    } catch (e) {
      this.setError(loadErrorText(e));
    } finally {
      this.analyzing = false;
    }
  }

  async dismiss(id: string): Promise<void> {
    if (this.clearing) return;
    this.clearing = true;
    this.dismissingId = id;
    try {
      await api.post(`/telemetry/suggestions/${encodeURIComponent(id)}/dismiss`);
      this.suggestions = this.suggestions.map((s) => (s.id === id ? { ...s, dismissed: true } : s));
    } catch (e) {
      this.setError(loadErrorText(e));
    } finally {
      this.clearing = false;
      this.dismissingId = null;
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
    if (this.capturing) return;
    this.capturing = true;
    this.setError('');
    try {
      this.profile = await api.post<TelemetryProfile>('/telemetry/profile');
    } catch (e) {
      this.setError(loadErrorText(e));
    } finally {
      this.capturing = false;
    }
  }

  resetForTests(): void {
    this.loadSeq++;
    this.detailSeq++;
    this.scopeGetter = null;
    this.config = null;
    this.savedConfig = '';
    this.status = null;
    this.overview = null;
    this.suggestions = [];
    this.loading = true;
    this.loadedOnce = false;
    this.togglingEnabled = false;
    this.savingRetention = false;
    this.clearing = false;
    this.dismissingId = null;
    this.analyzing = false;
    this.capturing = false;
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
  }
}

export const ottoTelemetry = new OttoTelemetryStore();
