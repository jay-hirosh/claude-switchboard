import { describe, it, expect, beforeEach } from 'vitest';
import { useDataScopeStore, readStoredShowAllDevices } from '../dataScope';

describe('readStoredShowAllDevices', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('defaults to false when localStorage is empty', () => {
    expect(readStoredShowAllDevices()).toBe(false);
  });

  it('returns true when stored as "true"', () => {
    localStorage.setItem('show-all-devices', 'true');
    expect(readStoredShowAllDevices()).toBe(true);
  });

  it('returns false for any other stored value', () => {
    localStorage.setItem('show-all-devices', 'nah');
    expect(readStoredShowAllDevices()).toBe(false);
  });
});

describe('useDataScopeStore', () => {
  beforeEach(() => {
    localStorage.clear();
    useDataScopeStore.setState({ showAllDevices: false });
  });

  it('updates and persists showAllDevices', () => {
    useDataScopeStore.getState().setShowAllDevices(true);
    expect(useDataScopeStore.getState().showAllDevices).toBe(true);
    expect(localStorage.getItem('show-all-devices')).toBe('true');
  });
});
