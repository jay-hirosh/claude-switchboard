import { create } from 'zustand';

const SHOW_ALL_DEVICES_KEY = 'show-all-devices';

export function readStoredShowAllDevices(): boolean {
  if (typeof localStorage === 'undefined') return false;
  return localStorage.getItem(SHOW_ALL_DEVICES_KEY) === 'true';
}

interface DataScopeStore {
  /** false (default): dashboards show only this machine's activity. */
  showAllDevices: boolean;
  setShowAllDevices: (showAllDevices: boolean) => void;
}

function persist(value: boolean) {
  try {
    localStorage.setItem(SHOW_ALL_DEVICES_KEY, String(value));
  } catch {
    // Safari private mode / quota-exceeded — preference still applies for
    // this session, just isn't persisted across launches. Same rationale as
    // appearance.ts's persist().
  }
}

export const useDataScopeStore = create<DataScopeStore>((set) => ({
  showAllDevices: readStoredShowAllDevices(),
  setShowAllDevices: (showAllDevices) => {
    persist(showAllDevices);
    set({ showAllDevices });
  },
}));
