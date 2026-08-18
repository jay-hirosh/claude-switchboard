import { useState } from 'react';
import { UsageTrends } from './trends/UsageTrends';
import { DailyPatternPanel } from './trends/DailyPatternPanel';

const SUB_TABS = [
  { id: 'usage', label: 'Usage' },
  { id: 'pattern', label: 'Daily pattern' },
] as const;
type SubTabId = (typeof SUB_TABS)[number]['id'];

export function TrendsTab() {
  const [subTab, setSubTab] = useState<SubTabId>('usage');

  return (
    <div className="flex flex-col gap-[var(--space-md)]">
      <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
        {SUB_TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => setSubTab(t.id)}
            className={[
              'px-[var(--space-sm)] py-[var(--space-2xs)]',
              'text-[length:var(--text-label)] font-[var(--weight-medium)]',
              'rounded-[var(--radius-sm)]',
              'transition-[background,color] duration-[var(--duration-fast)]',
              subTab === t.id
                ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
            ].join(' ')}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Lazily mounted — the Daily pattern panel fetches its own history
          window, and mounting both eagerly would double that fetch every
          time Trends is opened. */}
      {subTab === 'usage' ? <UsageTrends /> : <DailyPatternPanel />}
    </div>
  );
}
