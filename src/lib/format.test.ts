import { describe, it, expect } from 'vitest';
import { formatTokens, formatCost, formatCents } from './format';

describe('formatTokens', () => {
  it('returns "0" for zero', () => {
    expect(formatTokens(0)).toBe('0');
  });

  it('returns plain number for values below 1 000', () => {
    expect(formatTokens(999)).toBe('999');
  });

  it('returns 1.0K at exactly 1 000', () => {
    expect(formatTokens(1_000)).toBe('1.0K');
  });

  it('returns 1.5K for 1 500', () => {
    expect(formatTokens(1_500)).toBe('1.5K');
  });

  it('returns 1.0M at exactly 1 000 000', () => {
    expect(formatTokens(1_000_000)).toBe('1.0M');
  });

  it('returns 2.5M for 2 500 000', () => {
    expect(formatTokens(2_500_000)).toBe('2.5M');
  });
});

describe('formatCost', () => {
  it('returns "<$0.01" for zero', () => {
    expect(formatCost(0)).toBe('<$0.01');
  });

  it('returns "<$0.01" for values below 0.01', () => {
    expect(formatCost(0.005)).toBe('<$0.01');
  });

  it('returns "$0.01" for exactly 0.01', () => {
    expect(formatCost(0.01)).toBe('$0.01');
  });

  it('returns "$1.50" for 1.5', () => {
    expect(formatCost(1.5)).toBe('$1.50');
  });

  it('returns "$100.00" for 100', () => {
    expect(formatCost(100)).toBe('$100.00');
  });
});

describe('formatCents', () => {
  it('formats cents as dollars with two decimals', () => {
    expect(formatCents(3120)).toBe('$31.20');
    expect(formatCents(10000)).toBe('$100.00');
    expect(formatCents(0)).toBe('$0.00');
    expect(formatCents(5)).toBe('$0.05');
  });
});
