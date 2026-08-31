import { useEffect, useRef, useState } from 'react';
import { Card } from './Card';
import { IconButton } from './IconButton';
import { Calendar, ChevronLeft, ChevronRight } from '../../lib/icons';
import { localDayKey, formatDayLabel } from '../../lib/dayKey';

const WEEKDAY_LABELS = ['Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa', 'Su'];

/** Monday-first weekday index (0=Mon..6=Sun) for `date`, matching
 * HeatmapTab's grid convention so the two calendar UIs read consistently. */
function mondayFirstWeekday(date: Date): number {
  return (date.getDay() + 6) % 7;
}

interface DatePickerProps {
  /** Selected day, `YYYY-MM-DD`, or `null` if none picked yet. */
  value: string | null;
  /** Whether the date this picker represents is the currently active period —
   * drives the trigger's pill styling to match the Today/Yesterday/Week pills. */
  active: boolean;
  onSelect: (dayKey: string) => void;
}

/** Segmented-pill-styled trigger that opens a glass calendar popover, for
 * picking an arbitrary past local calendar day. Sits alongside the
 * Today/Yesterday/This Week pills as a 4th, freeform option. */
export function DatePicker({ value, active, onSelect }: DatePickerProps) {
  const [open, setOpen] = useState(false);
  const [viewDate, setViewDate] = useState(() => {
    if (value) {
      const [y, m] = value.split('-').map(Number);
      return new Date(y, m - 1, 1);
    }
    const now = new Date();
    return new Date(now.getFullYear(), now.getMonth(), 1);
  });
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') setOpen(false);
    }
    window.addEventListener('mousedown', onPointerDown);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('mousedown', onPointerDown);
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  function toggleOpen() {
    if (!open && value) {
      const [y, m] = value.split('-').map(Number);
      setViewDate(new Date(y, m - 1, 1));
    }
    setOpen((o) => !o);
  }

  const today = new Date();
  const todayKey = localDayKey(today.toISOString());
  const year = viewDate.getFullYear();
  const month = viewDate.getMonth();
  const atCurrentMonth = year === today.getFullYear() && month === today.getMonth();
  const daysInMonth = new Date(year, month + 1, 0).getDate();
  const leadingBlanks = mondayFirstWeekday(new Date(year, month, 1));
  const cells: (string | null)[] = [
    ...Array(leadingBlanks).fill(null),
    ...Array.from({ length: daysInMonth }, (_, i) => localDayKey(new Date(year, month, i + 1).toISOString())),
  ];

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={toggleOpen}
        aria-haspopup="dialog"
        aria-expanded={open}
        data-testid="date-picker-trigger"
        className={[
          'flex items-center gap-[var(--space-2xs)]',
          'px-[var(--space-sm)] py-[var(--space-2xs)]',
          'text-[length:var(--text-label)] font-[var(--weight-medium)]',
          'rounded-[var(--radius-sm)]',
          'transition-[background,color] duration-[var(--duration-fast)]',
          active
            ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
            : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
        ].join(' ')}
      >
        <Calendar size={12} />
        {value && formatDayLabel(value)}
      </button>
      {open && (
        <Card
          variant="glass"
          role="dialog"
          aria-label="Choose a date"
          data-testid="date-picker-popover"
          className="absolute right-0 top-[calc(100%+var(--space-2xs))] z-10 p-[var(--space-sm)] w-[240px] flex flex-col gap-[var(--space-sm)]"
        >
          <div className="flex items-center justify-between">
            <IconButton label="Previous month" onClick={() => setViewDate(new Date(year, month - 1, 1))}>
              <ChevronLeft size={13} />
            </IconButton>
            <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text)]">
              {viewDate.toLocaleDateString('en-US', { month: 'long', year: 'numeric' })}
            </span>
            <IconButton
              label="Next month"
              onClick={() => setViewDate(new Date(year, month + 1, 1))}
              disabled={atCurrentMonth}
            >
              <ChevronRight size={13} />
            </IconButton>
          </div>
          <div className="grid grid-cols-7 gap-[2px]">
            {WEEKDAY_LABELS.map((d) => (
              <span
                key={d}
                className="text-center text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]"
              >
                {d}
              </span>
            ))}
            {cells.map((dayKey, i) => {
              if (!dayKey) return <span key={`blank-${i}`} />;
              const disabled = dayKey > todayKey;
              const selected = dayKey === value;
              return (
                <button
                  key={dayKey}
                  type="button"
                  disabled={disabled}
                  onClick={() => {
                    onSelect(dayKey);
                    setOpen(false);
                  }}
                  data-testid={`date-picker-day-${dayKey}`}
                  className={[
                    'aspect-square rounded-[var(--radius-sm)] text-[length:var(--text-label)]',
                    'transition-[background,color] duration-[var(--duration-fast)]',
                    disabled
                      ? 'text-[color:var(--color-text-muted)] opacity-30 cursor-default'
                      : selected
                        ? 'bg-[var(--color-accent)] text-[color:var(--color-bg-card)]'
                        : 'text-[color:var(--color-text)] hover:bg-[var(--color-bg-card-hover)]',
                  ].join(' ')}
                >
                  {Number(dayKey.slice(-2))}
                </button>
              );
            })}
          </div>
        </Card>
      )}
    </div>
  );
}
