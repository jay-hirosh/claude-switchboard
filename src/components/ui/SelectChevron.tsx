import { ChevronDown } from '../../lib/icons';

/** The chevron for a `selectClass` field. Absolutely positioned, so the
 *  `<select>` and this must share a `relative` wrapper. */
export function SelectChevron() {
  return (
    <ChevronDown
      size={12}
      aria-hidden
      className="
        pointer-events-none absolute right-[var(--space-sm)] top-1/2
        -translate-y-1/2 text-[color:var(--color-text-muted)]
      "
    />
  );
}
