import { describe, it, expect, beforeEach } from 'vitest';
import {
  useAppearanceStore,
  readStoredAccentColor,
  readStoredDensity,
  readStoredReduceMotion,
} from '../appearance';

describe('readStoredAccentColor', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('defaults to terracotta when localStorage is empty', () => {
    expect(readStoredAccentColor()).toBe('terracotta');
  });

  it('returns a valid stored value', () => {
    localStorage.setItem('accent-color', 'teal');
    expect(readStoredAccentColor()).toBe('teal');
  });

  it('falls back to terracotta for an unrecognized stored value', () => {
    localStorage.setItem('accent-color', 'chartreuse');
    expect(readStoredAccentColor()).toBe('terracotta');
  });
});

describe('readStoredDensity', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('defaults to comfortable when localStorage is empty', () => {
    expect(readStoredDensity()).toBe('comfortable');
  });

  it('returns a valid stored value', () => {
    localStorage.setItem('density', 'compact');
    expect(readStoredDensity()).toBe('compact');
  });

  it('falls back to comfortable for an unrecognized stored value', () => {
    localStorage.setItem('density', 'roomy');
    expect(readStoredDensity()).toBe('comfortable');
  });
});

describe('readStoredReduceMotion', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('defaults to false when localStorage is empty', () => {
    expect(readStoredReduceMotion()).toBe(false);
  });

  it('returns true when stored as "true"', () => {
    localStorage.setItem('reduce-motion', 'true');
    expect(readStoredReduceMotion()).toBe(true);
  });

  it('returns false for any other stored value', () => {
    localStorage.setItem('reduce-motion', 'nah');
    expect(readStoredReduceMotion()).toBe(false);
  });
});

describe('useAppearanceStore', () => {
  beforeEach(() => {
    localStorage.clear();
    useAppearanceStore.setState({
      accentColor: 'terracotta',
      density: 'comfortable',
      reduceMotion: false,
    });
  });

  it('updates and persists the accent color', () => {
    useAppearanceStore.getState().setAccentColor('graphite');
    expect(useAppearanceStore.getState().accentColor).toBe('graphite');
    expect(localStorage.getItem('accent-color')).toBe('graphite');
  });

  it('updates and persists the density', () => {
    useAppearanceStore.getState().setDensity('compact');
    expect(useAppearanceStore.getState().density).toBe('compact');
    expect(localStorage.getItem('density')).toBe('compact');
  });

  it('updates and persists reduce motion', () => {
    useAppearanceStore.getState().setReduceMotion(true);
    expect(useAppearanceStore.getState().reduceMotion).toBe(true);
    expect(localStorage.getItem('reduce-motion')).toBe('true');
  });
});
