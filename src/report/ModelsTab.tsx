import { useMemo } from 'react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { EmptyState } from '../components/ui/EmptyState';
import { formatTokens, formatCost, formatCostPerToken } from '../lib/format';
import { IconChart } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import { MODEL_VARIANT, modelKey, shortName } from './modelDisplay';

export function ModelsTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const { data, error, loading, reload } = useTabData(
    () => Promise.all([ipc.getModelBreakdown(30), ipc.getCacheStats(30)]).then(([m, c]) => ({ models: m, cache: c })),
    [version],
  );
  const models = data?.models ?? null;
  const cache = data?.cache ?? null;

  const totalTokens = useMemo(
    () => (models ?? []).reduce((s, m) => s + m.input_tokens + m.output_tokens, 0),
    [models],
  );
  const totalCost = useMemo(
    () => (models ?? []).reduce((s, m) => s + m.cost_usd, 0),
    [models],
  );

  const radius = 50;
  const circumference = 2 * Math.PI * radius;

  const segments = useMemo(() => {
    let acc = 0;
    return (models ?? []).map((m) => {
      const pct = totalTokens > 0 ? ((m.input_tokens + m.output_tokens) / totalTokens) * 100 : 0;
      const strokeLength = (pct / 100) * circumference;
      const offset = acc;
      acc += strokeLength;
      return {
        ...m,
        key: modelKey(m.model),
        total: m.input_tokens + m.output_tokens,
        pct,
        strokeLength,
        offset,
      };
    });
  }, [models, totalTokens, circumference]);

  if (error) {
    return (
      <EmptyState
        icon={<IconChart size={32} />}
        title="Couldn't load models"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !models || !cache) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  if (models.length === 0) {
    return (
      <EmptyState
        icon={<IconChart size={32} />}
        title="No model data"
        description="Model breakdown will appear after your first sessions."
      />
    );
  }

  return (
    <div className="flex flex-col gap-[var(--space-lg)]">
      {/* Donut chart */}
      <div className="flex items-center justify-center py-[var(--space-lg)]">
        <div className="relative">
          <svg width="140" height="140" viewBox="0 0 140 140">
            {segments.map((seg) => {
              const { strokeLength, offset } = seg;

              const colors: Record<string, string> = {
                opus: 'var(--color-model-opus)',
                sonnet: 'var(--color-model-sonnet)',
                haiku: 'var(--color-model-haiku)',
              };

              return (
                <circle
                  key={seg.model}
                  cx="70"
                  cy="70"
                  r={radius}
                  fill="none"
                  stroke={colors[seg.key] ?? 'var(--color-text-muted)'}
                  strokeWidth="14"
                  strokeDasharray={`${strokeLength} ${circumference - strokeLength}`}
                  strokeDashoffset={-offset}
                  strokeLinecap="round"
                  transform="rotate(-90 70 70)"
                  className="transition-[stroke-dasharray,stroke-dashoffset] duration-[var(--duration-slow)] ease-[var(--ease-spring)]"
                />
              );
            })}
          </svg>
          <div className="absolute inset-0 flex flex-col items-center justify-center">
            <span className="mono text-[length:var(--text-title)] font-[var(--weight-semibold)] text-[color:var(--color-text)]">
              {formatTokens(totalTokens)}
            </span>
            <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">tokens</span>
          </div>
        </div>
      </div>

      {/* Model list */}
      <div className="flex flex-col gap-[var(--space-sm)]">
        {segments.map((seg) => (
          <Card key={seg.model} className="p-[var(--space-sm)]">
            <div className="flex items-center gap-[var(--space-sm)]">
              <Badge variant={MODEL_VARIANT[seg.key] ?? 'default'}>
                {shortName(seg.model)}
              </Badge>
              <div className="flex-1">
                <div className="w-full h-[6px] rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
                  <div
                    className="h-full rounded-[var(--radius-pill)] transition-[width] duration-[var(--duration-bar)] ease-[var(--ease-spring)]"
                    style={{
                      width: `${seg.pct}%`,
                      background:
                        seg.key === 'opus'
                          ? 'var(--color-model-opus)'
                          : seg.key === 'sonnet'
                            ? 'var(--color-model-sonnet)'
                            : 'var(--color-model-haiku)',
                    }}
                  />
                </div>
              </div>
              <div className="flex items-center gap-[var(--space-md)] shrink-0">
                <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] tabular-nums min-w-[52px] text-right">
                  {seg.pct.toFixed(0)}%
                </span>
                <div className="flex flex-col items-end min-w-[48px]">
                  <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-muted)] tabular-nums">
                    {formatCost(seg.cost_usd)}
                  </span>
                  <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
                    {formatCostPerToken(seg.cost_usd, seg.total)}
                  </span>
                </div>
              </div>
            </div>
          </Card>
        ))}
      </div>

      {/* Cache efficiency */}
      <Card className="p-[var(--space-md)]">
        <h3 className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)] mb-[var(--space-sm)]">
          Cache efficiency (30d)
        </h3>
        <div className="grid grid-cols-2 gap-[var(--space-sm)]">
          <div>
            <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">Hit ratio</span>
            <p className="mono text-[length:var(--text-body)] font-[var(--weight-semibold)] text-[color:var(--color-text)]">
              {(cache.hit_ratio * 100).toFixed(1)}%
            </p>
          </div>
          <div>
            <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">Est. savings</span>
            <p className="mono text-[length:var(--text-body)] font-[var(--weight-semibold)] text-[color:var(--color-safe)]">
              ${cache.estimated_savings_usd.toFixed(2)}
            </p>
          </div>
        </div>
      </Card>

      {/* Total */}
      <div className="flex items-center justify-between px-[var(--space-2xs)]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">Total</span>
        <span className="mono text-[length:var(--text-body)] font-[var(--weight-semibold)] text-[color:var(--color-text)]">
          {formatCost(totalCost)}
        </span>
      </div>
    </div>
  );
}
