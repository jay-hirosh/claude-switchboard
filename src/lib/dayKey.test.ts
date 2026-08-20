import { describe, it, expect } from 'vitest';
import { localDayKey, formatDayLabel, weekStartDayKey } from './dayKey';

describe('localDayKey', () => {
  it('returns the local calendar day as YYYY-MM-DD', () => {
    const d = new Date(2026, 7, 18, 23, 30); // Aug 18, 2026, 23:30 local
    expect(localDayKey(d.toISOString())).toBe('2026-08-18');
  });
});

describe('formatDayLabel', () => {
  it('labels today as "Today"', () => {
    const todayKey = localDayKey(new Date().toISOString());
    expect(formatDayLabel(todayKey)).toBe('Today');
  });

  it('labels yesterday as "Yesterday"', () => {
    const yesterday = new Date();
    yesterday.setDate(yesterday.getDate() - 1);
    expect(formatDayLabel(localDayKey(yesterday.toISOString()))).toBe('Yesterday');
  });

  it('formats an older day as "Mon D"', () => {
    expect(formatDayLabel('2026-01-05')).toBe('Jan 5');
  });
});

describe('weekStartDayKey', () => {
  it('returns the day key 6 calendar days before today, matching a 7-day rolling window', () => {
    const sixDaysAgo = new Date();
    sixDaysAgo.setDate(sixDaysAgo.getDate() - 6);
    expect(weekStartDayKey()).toBe(localDayKey(sixDaysAgo.toISOString()));
  });
});
