import { describe, it, expect, beforeEach } from 'vitest';
import { isRoutingHintDismissed, dismissRoutingHint } from './routingHintDismissal';

describe('routing hint dismissal', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('is not dismissed before dismissRoutingHint is called', () => {
    expect(isRoutingHintDismissed('acct-1', '2026-08-20T00:00:00Z')).toBe(false);
  });

  it('is dismissed for the same resetsAt after dismissing', () => {
    dismissRoutingHint('acct-1', '2026-08-20T00:00:00Z');
    expect(isRoutingHintDismissed('acct-1', '2026-08-20T00:00:00Z')).toBe(true);
  });

  it('reappears once the 7D window rolls over to a new resetsAt', () => {
    dismissRoutingHint('acct-1', '2026-08-20T00:00:00Z');
    expect(isRoutingHintDismissed('acct-1', '2026-08-27T00:00:00Z')).toBe(false);
  });

  it('scopes dismissal per account', () => {
    dismissRoutingHint('acct-1', '2026-08-20T00:00:00Z');
    expect(isRoutingHintDismissed('acct-2', '2026-08-20T00:00:00Z')).toBe(false);
  });

  it('treats a null resetsAt consistently, and still clears once real data arrives', () => {
    dismissRoutingHint('acct-1', null);
    expect(isRoutingHintDismissed('acct-1', null)).toBe(true);
    expect(isRoutingHintDismissed('acct-1', '2026-08-20T00:00:00Z')).toBe(false);
  });
});
