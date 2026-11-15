export type UsageSubView = 'overview' | 'report' | 'otto';

class UsageUiState {
  view = $state<UsageSubView>('overview');
  selectedDailyKey = $state('');
  configOpen = $state(false);
  budgetsOpen = $state(false);
  telemetryDirty = $state(false);
  telemetryHours = $state(24);
  onDiscardTelemetry: (() => void) | null = null;

  discardTelemetry(): void {
    this.onDiscardTelemetry?.();
    this.telemetryDirty = false;
  }

  resetForTests(): void {
    this.view = 'overview';
    this.selectedDailyKey = '';
    this.configOpen = false;
    this.budgetsOpen = false;
    this.telemetryDirty = false;
    this.telemetryHours = 24;
    this.onDiscardTelemetry = null;
  }
}

export const usageUi = new UsageUiState();
