export type UsageSubView = 'overview' | 'report' | 'otto';
/** The "Where it went" switch. */
export type WhereView = 'provider' | 'model' | 'feature' | 'session' | 'work';

class UsageUiState {
  view = $state<UsageSubView>('overview');
  where = $state<WhereView>('provider');
  /** The one chart: cost by default, tokens one tap away. */
  chartMetric = $state<'cost' | 'tokens'>('cost');
  selectedDailyKey = $state('');
  configOpen = $state(false);
  budgetsOpen = $state(false);
  systemOpen = $state(false);
  telemetryDirty = $state(false);
  telemetryHours = $state(24);
  onDiscardTelemetry: (() => void) | null = null;

  discardTelemetry(): void {
    this.onDiscardTelemetry?.();
    this.telemetryDirty = false;
  }

  resetForTests(): void {
    this.view = 'overview';
    this.where = 'provider';
    this.chartMetric = 'cost';
    this.selectedDailyKey = '';
    this.configOpen = false;
    this.budgetsOpen = false;
    this.systemOpen = false;
    this.telemetryDirty = false;
    this.telemetryHours = 24;
    this.onDiscardTelemetry = null;
  }
}

export const usageUi = new UsageUiState();
