import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import type { CachedUsage } from '../lib/types';
import { UsageSummary } from './UsageSummary';

function usage(): CachedUsage {
  return {
    snapshot: {
      five_hour: { utilization: 19, resets_at: new Date(Date.now() + 5400_000).toISOString() },
      seven_day: { utilization: 32, resets_at: new Date(Date.now() + 93600_000).toISOString() },
      seven_day_sonnet: { utilization: 20, resets_at: null },
      seven_day_opus: { utilization: 40, resets_at: null },
      extra_usage: null,
      fetched_at: new Date().toISOString(),
    },
    account_id: 'uuid-1',
    account_email: 'a@x.com',
    last_error: null,
    burn_rate: null,
    auth_source: 'OAuth',
  } as CachedUsage;
}

/** A Pro / Team-Pro plan snapshot: the aggregate 7-day bucket is populated,
 * but the API returns null for the per-model opus/sonnet split (only Max
 * plans carry separate per-model weekly quotas). */
function proUsage(): CachedUsage {
  const u = usage();
  u.snapshot.seven_day_sonnet = null;
  u.snapshot.seven_day_opus = null;
  return u;
}

describe('UsageSummary collapsible details', () => {
  it('renders only the hero numbers plus a disclosure row when collapsed', () => {
    const onToggle = vi.fn();
    render(
      <UsageSummary
        usage={usage()}
        thresholds={[75, 90]}
        collapsible
        detailsOpen={false}
        onToggleDetails={onToggle}
      />,
    );
    // Hero numbers visible…
    expect(screen.getByText('19')).toBeTruthy();
    expect(screen.getByText('32')).toBeTruthy();
    // …disclosure row present…
    expect(screen.getByRole('button', { name: /details/i })).toBeTruthy();
    // …but the model split is hidden.
    expect(screen.queryByText('Opus')).toBeNull();
    expect(screen.queryByText('Sonnet')).toBeNull();
  });

  it('renders the detail rows when expanded', () => {
    render(
      <UsageSummary
        usage={usage()}
        thresholds={[75, 90]}
        collapsible
        detailsOpen
        onToggleDetails={() => {}}
      />,
    );
    expect(screen.getByText('Opus')).toBeTruthy();
    expect(screen.getByText('Sonnet')).toBeTruthy();
  });

  it('clicking the disclosure row toggles', () => {
    const onToggle = vi.fn();
    render(
      <UsageSummary
        usage={usage()}
        thresholds={[75, 90]}
        collapsible
        detailsOpen={false}
        onToggleDetails={onToggle}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /details/i }));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it('renders fully with no disclosure when not collapsible (expanded report)', () => {
    render(
      <UsageSummary usage={usage()} thresholds={[75, 90]} condensed />,
    );
    expect(screen.queryByRole('button', { name: /details/i })).toBeNull();
    expect(screen.getByText('Opus')).toBeTruthy();
  });

  it('hides the Opus/Sonnet rows when the plan provides no per-model data (Pro)', () => {
    // seven_day is present but both per-model buckets are null — the API only
    // breaks out per-model utilization for Max plans. The empty rows must not
    // render, even with details expanded.
    render(
      <UsageSummary
        usage={proUsage()}
        thresholds={[75, 90]}
        collapsible
        detailsOpen
        onToggleDetails={() => {}}
      />,
    );
    expect(screen.queryByText('Opus')).toBeNull();
    expect(screen.queryByText('Sonnet')).toBeNull();
  });

  it('merges the reset countdown into the bucket label row (no separate caption row)', () => {
    render(<UsageSummary usage={usage()} thresholds={[75, 90]} />);
    // The 5h label row carries its reset time — and nothing else (in the old
    // layout the label's parent was the whole column, hero number included).
    const labelRow = screen.getByText('5h').parentElement!;
    expect(labelRow.textContent).toMatch(/in 1h \d+m/);
    expect(labelRow.textContent).not.toContain('19');
    // Exactly one reset countdown per bucket — not duplicated below the meter.
    expect(screen.getAllByText(/^1h \d+m$/)).toHaveLength(1);
    expect(screen.getAllByText(/^2[56]h \d+m$/)).toHaveLength(1);
  });

  it('keeps the burn-rate projection as the sole bottom caption when present', () => {
    const u = usage();
    u.burn_rate = { utilization_per_min: 0.5, projected_at_reset: 113 };
    render(<UsageSummary usage={u} thresholds={[75, 90]} />);
    expect(screen.getByText(/~113% by reset/)).toBeTruthy();
  });
});

function paygUsage(): CachedUsage {
  const u = usage();
  u.snapshot.extra_usage = {
    is_enabled: true,
    monthly_limit_cents: 10000,
    used_credits_cents: 3120,
    utilization: 31.2,
    resets_at: new Date(Date.now() + 10 * 86400_000).toISOString(),
  };
  (u as CachedUsage).extra_burn_rate = {
    cents_per_min: 1.5,
    projected_cents_at_reset: 5200,
  };
  return u;
}

describe('pay-as-you-go dollars and forecast', () => {
  it('shows spent-of-limit dollars and the projected spend', () => {
    render(
      <UsageSummary usage={paygUsage()} thresholds={[75, 90]} />,
    );
    expect(screen.getByText(/\$31\.20 of \$100\.00/)).toBeTruthy();
    expect(screen.getByText(/→ ~\$52 by reset/)).toBeTruthy();
  });

  it('hides dollars when no limit and forecast when no projection', () => {
    const u = paygUsage();
    u.snapshot.extra_usage!.monthly_limit_cents = 0;
    u.extra_burn_rate = null;
    render(<UsageSummary usage={u} thresholds={[75, 90]} />);
    expect(screen.queryByText(/ of \$/)).toBeNull();
    expect(screen.queryByText(/by reset/)).toBeNull();
  });

  it('clamps a negative projected forecast to $0 instead of showing a negative dollar figure', () => {
    // A reset/top-up can leave a brief negative projection before the
    // backend's buffer clears on the next poll — the UI must never render
    // a "→ ~$-N by reset" figure.
    const u = paygUsage();
    u.extra_burn_rate = {
      cents_per_min: -5,
      projected_cents_at_reset: -587_500,
    };
    render(<UsageSummary usage={u} thresholds={[75, 90]} />);
    expect(screen.getByText(/→ ~\$0 by reset/)).toBeTruthy();
    expect(screen.queryByText(/-\$/)).toBeNull();
    expect(screen.queryByText(/\$-/)).toBeNull();
  });
});
