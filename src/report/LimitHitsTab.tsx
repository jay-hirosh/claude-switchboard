import { Card } from '../components/ui/Card';
import { EmptyState } from '../components/ui/EmptyState';
import { Button } from '../components/ui/Button';
import { formatCost } from '../lib/format';
import { IconWarning } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import type { AccountLimitHits } from '../lib/generated/bindings';

export function LimitHitsTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const { data, error, loading, reload } = useTabData(
    () => ipc.getLimitHitHistory(30),
    [version],
  );

  if (error) {
    return (
      <EmptyState
        icon={<IconWarning size={32} />}
        title="Couldn't load limit-hit history"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !data) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  const accountsWithHits = data.accounts.filter(
    (a) => a.five_hour_hits + a.seven_day_hits > 0,
  );
  if (accountsWithHits.length === 0) {
    return (
      <EmptyState
        icon={<IconWarning size={32} />}
        title="No limit hits yet"
        description="This report tracks rate-limit peaks going forward — check back after using Claude for a while."
      />
    );
  }

  return (
    <div className="flex flex-col gap-[var(--space-lg)]">
      {accountsWithHits.map((a) => (
        <AccountLimitHitsCard key={a.account_id} account={a} />
      ))}
    </div>
  );
}

function AccountLimitHitsCard({ account }: { account: AccountLimitHits }) {
  const maxCount = Math.max(...account.hourly_distribution, 1);
  return (
    <Card>
      <div className="flex items-center justify-between px-[var(--space-md)] pt-[var(--space-md)]">
        <span className="text-[length:var(--text-label)] font-[var(--weight-medium)]">
          {account.email}
        </span>
        <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          {account.five_hour_hits} × 5H · {account.seven_day_hits} × 7D
        </span>
      </div>
      <div
        className="flex items-end gap-[2px] px-[var(--space-md)] py-[var(--space-md)]"
        style={{ height: 80 }}
      >
        {account.hourly_distribution.map((count, hour) => (
          <div
            key={hour}
            className="flex-1 flex flex-col justify-end"
            title={`${hour}:00 — ${count} hit${count === 1 ? '' : 's'}`}
          >
            <div
              className="rounded-t-sm bg-[var(--color-danger)]"
              style={{ height: `${(count / maxCount) * 100}%`, minHeight: count > 0 ? 2 : 0 }}
            />
          </div>
        ))}
      </div>
      {account.top_projects.length > 0 && (
        <div className="flex flex-col gap-[var(--space-2xs)] px-[var(--space-md)] pb-[var(--space-md)]">
          {account.top_projects.map((p) => (
            <div
              key={p.project}
              className="flex items-center justify-between text-[length:var(--text-micro)]"
            >
              <span className="text-[color:var(--color-text-secondary)] truncate">{p.project}</span>
              <span className="mono text-[color:var(--color-text-muted)]">{formatCost(p.cost_usd)}</span>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
